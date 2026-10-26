mod crypto;
mod fields;
mod incremental;
mod pdf;
mod trust;

use std::{io::Read, path::Path};

use lopdf::{Dictionary, Document, Object, ObjectId};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::document::{Result, invalid};

pub use fields::{SignatureFieldInfo, SignatureWidgetInfo};
pub use trust::{
    ChainEvaluation, MAX_ANCHOR_BYTES, MAX_TRUST_ANCHORS, TrustStore, TrustStoreInfo,
    read_anchor_file,
};

pub const MAX_IDENTITY_BYTES: usize = 4 * 1024 * 1024;
const MAX_CMS_BYTES: usize = 256 * 1024;
const RESERVED_CMS_BYTES: usize = 32 * 1024;
const MAX_SIGNATURES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    Intact,
    Invalid,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    WholeDocument,
    LaterChanges,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateValidity {
    ValidNow,
    Expired,
    NotYetValid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateInfo {
    pub subject: String,
    pub issuer: String,
    pub serial: String,
    pub sha256_fingerprint: String,
    pub not_before: String,
    pub not_after: String,
    pub validity: CertificateValidity,
    pub self_signed: bool,
    pub identity_trust: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureInfo {
    pub id: String,
    pub integrity: Integrity,
    pub coverage: Coverage,
    pub message: String,
    pub digest: Option<String>,
    pub algorithm: Option<String>,
    pub signed_bytes: Option<usize>,
    pub current_bytes: usize,
    pub certificate: Option<CertificateInfo>,
    pub chain: ChainEvaluation,
    pub claimed_signing_time: Option<String>,
    pub revocation: &'static str,
    pub trusted_time: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureReport {
    pub document_id: u64,
    pub revision: u64,
    pub signatures: Vec<SignatureInfo>,
    pub signing_blocked_reason: Option<String>,
    pub warnings: Vec<String>,
    pub empty_fields: Vec<SignatureFieldInfo>,
    pub trust_store: TrustStoreInfo,
}

#[derive(Debug)]
struct CheckError {
    status: Integrity,
    message: String,
}

type CheckResult<T> = std::result::Result<T, CheckError>;

impl CheckError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: Integrity::Invalid,
            message: message.into(),
        }
    }

    fn unsupported(message: impl Into<String>) -> Self {
        Self {
            status: Integrity::Unsupported,
            message: message.into(),
        }
    }
}

impl From<CheckError> for crate::document::Error {
    fn from(value: CheckError) -> Self {
        invalid(value.message)
    }
}

struct Candidate<'a> {
    id: Option<ObjectId>,
    dictionary: &'a Dictionary,
}

fn signature_dictionary(dictionary: &Dictionary) -> bool {
    matches!(
        dictionary.get(b"Type").and_then(Object::as_name).ok(),
        Some(b"Sig" | b"DocTimeStamp")
    ) || dictionary.has(b"ByteRange")
}

fn walk<'a>(
    object: &'a Object,
    root_id: Option<ObjectId>,
    depth: usize,
    action: &mut impl FnMut(Option<ObjectId>, &'a Dictionary) -> CheckResult<()>,
) -> CheckResult<()> {
    if depth > 64 {
        return Err(CheckError::unsupported(
            "The PDF has excessively nested signature/form structures.",
        ));
    }
    match object {
        Object::Dictionary(dictionary) => {
            action(root_id, dictionary)?;
            for (_, value) in dictionary.iter() {
                walk(value, None, depth + 1, action)?;
            }
        }
        Object::Array(values) => {
            for value in values {
                walk(value, None, depth + 1, action)?;
            }
        }
        Object::Stream(stream) => {
            action(None, &stream.dict)?;
            for (_, value) in stream.dict.iter() {
                walk(value, None, depth + 1, action)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn candidates(document: &Document) -> CheckResult<Vec<Candidate<'_>>> {
    let mut signatures = Vec::new();
    for (id, object) in &document.objects {
        walk(object, Some(*id), 0, &mut |id, dictionary| {
            if signature_dictionary(dictionary) {
                if signatures.len() >= MAX_SIGNATURES {
                    return Err(CheckError::unsupported(
                        "This PDF exceeds the 64-signature inspection limit.",
                    ));
                }
                signatures.push(Candidate { id, dictionary });
            }
            Ok(())
        })?;
    }
    Ok(signatures)
}

pub(crate) fn has_signatures(document: &Document) -> bool {
    if document
        .catalog()
        .is_ok_and(|catalog| catalog.has(b"Perms"))
    {
        return true;
    }
    let mut signed = false;
    for object in document.objects.values() {
        let outcome = walk(object, None, 0, &mut |_, dictionary| {
            signed |= signature_dictionary(dictionary)
                || dictionary.get(b"FT").and_then(Object::as_name).ok() == Some(b"Sig")
                    && dictionary
                        .get(b"V")
                        .is_ok_and(|value| !matches!(value, Object::Null));
            Ok(())
        });
        if outcome.is_err() || signed {
            return true;
        }
    }
    false
}

fn signing_fields(document: &Document) -> Result<fields::Fields> {
    if document.was_encrypted() || document.is_encrypted() || document.trailer.has(b"Encrypt") {
        return Err(invalid(
            "Signing encrypted PDFs is not supported. Folio will not remove or weaken their password protection.",
        ));
    }
    if document.catalog()?.has(b"Perms") {
        return Err(invalid(
            "Certification/DocMDP and usage-rights policies are not implemented. This PDF will not be signed.",
        ));
    }
    if document.xref_recovery.is_some() {
        return Err(invalid(
            "Signing PDFs that required cross-reference repair is not supported.",
        ));
    }
    if document.trailer.has(b"XRefStm") {
        return Err(invalid("Signing hybrid-reference PDFs is not supported."));
    }
    if document.max_id > u32::MAX - 128 {
        return Err(invalid(
            "Signing requires at least 128 unused object numbers below the PDF engine's object-number limit.",
        ));
    }
    let fields = fields::read(document)?;
    for (id, object) in &document.objects {
        walk(object, Some(*id), 0, &mut |id, dictionary| {
            if dictionary.get(b"FT").and_then(Object::as_name).ok() == Some(b"Sig")
                && id.is_none_or(|id| !fields.visited.contains(&id))
            {
                return Err(CheckError::unsupported(
                    "Existing signature fields must be indirect, unambiguous members of AcroForm Fields; orphan or embedded fields are not supported.",
                ));
            }
            Ok(())
        })?;
    }
    if let Ok(form) = document.catalog()?.get(b"AcroForm") {
        if matches!(form, Object::Null) {
            return Ok(fields);
        }
        let form = document.dereference(form)?.1.as_dict()?;
        if let Ok(flags) = form.get(b"SigFlags")
            && !(0..=3).contains(&document.dereference(flags)?.1.as_i64()?)
        {
            return Err(invalid("The form has unsupported signature flags."));
        }
    }
    Ok(fields)
}

fn signing_checks(
    document: &Document,
    bytes: &[u8],
    signatures: &[SignatureInfo],
) -> Result<fields::Fields> {
    let fields = signing_fields(document)?;
    pdf::validate_footer(bytes, document)?;
    pdf::ensure_supported_revisions(bytes, document)?;
    if signatures.len() >= MAX_SIGNATURES {
        return Err(invalid("This PDF has reached the 64-signature limit."));
    }
    if signatures.is_empty() && has_signatures(document) {
        return Err(invalid(
            "Unrecognized or ambiguous signature markers prevent signing this PDF.",
        ));
    }
    if signatures
        .iter()
        .any(|signature| signature.integrity != Integrity::Intact)
    {
        return Err(invalid(
            "Every existing signature must pass the supported structural and CMS integrity checks before adding an approval signature.",
        ));
    }
    if !signatures.is_empty()
        && signatures
            .iter()
            .filter(|signature| signature.coverage == Coverage::WholeDocument)
            .count()
            != 1
    {
        return Err(invalid(
            "The latest signature must cover the entire current file. Unsigned appended bytes or revisions must not be approved by co-signing.",
        ));
    }
    let mut ends = std::collections::HashSet::new();
    for signature in signatures {
        if !ends.insert(signature.signed_bytes) {
            return Err(invalid(
                "Multiple signatures claim the same PDF revision; approval signing is ambiguous.",
            ));
        }
        if fields
            .fields
            .iter()
            .filter(|field| {
                field
                    .signature
                    .is_some_and(|id| fields::key(id) == signature.id)
            })
            .count()
            != 1
        {
            return Err(invalid(
                "Each existing signature must remain bound to exactly one logical signature field.",
            ));
        }
    }
    if fields
        .fields
        .iter()
        .filter(|field| field.signature.is_some())
        .count()
        != signatures.len()
    {
        return Err(invalid(
            "The signature fields and supported signature dictionaries do not match.",
        ));
    }
    Ok(fields)
}

pub(crate) fn ensure_signable(document: &Document, bytes: &[u8]) -> Result<()> {
    let document = signing_source(document, bytes)?;
    let signatures = inspect_signatures(&document, bytes, &TrustStore::default())?;
    signing_checks(&document, bytes, &signatures)?;
    Ok(())
}

fn signing_source(document: &Document, bytes: &[u8]) -> Result<Document> {
    if document.was_encrypted() || document.is_encrypted() || document.trailer.has(b"Encrypt") {
        return Err(invalid(
            "Signing encrypted PDFs is not supported. Protection is left intact.",
        ));
    }
    // A Session's mutation document can carry the previous serialization's xref offsets.
    // Incremental output must be based on the exact current working bytes instead.
    Ok(crate::protection::load(bytes, None)?.document)
}

pub(crate) fn inspect(
    document: &Document,
    bytes: &[u8],
    document_id: u64,
    revision: u64,
) -> Result<SignatureReport> {
    inspect_with_trust(
        document,
        bytes,
        document_id,
        revision,
        &TrustStore::default(),
    )
}

pub(crate) fn inspect_with_trust(
    document: &Document,
    bytes: &[u8],
    document_id: u64,
    revision: u64,
    trust: &TrustStore,
) -> Result<SignatureReport> {
    if !(document.was_encrypted() || document.is_encrypted() || document.trailer.has(b"Encrypt")) {
        let source = signing_source(document, bytes)?;
        return inspect_loaded(&source, bytes, document_id, revision, trust);
    }
    inspect_loaded(document, bytes, document_id, revision, trust)
}

fn inspect_loaded(
    document: &Document,
    bytes: &[u8],
    document_id: u64,
    revision: u64,
    trust: &TrustStore,
) -> Result<SignatureReport> {
    let signatures = inspect_signatures(document, bytes, trust)?;
    let warnings = if signatures.is_empty() && has_signatures(document) {
        vec!["Signature fields or certification markers are present, but no supported signature dictionary was found. This is not an unsigned-document assurance.".into()]
    } else {
        Vec::new()
    };
    let checks = signing_checks(document, bytes, &signatures);
    Ok(SignatureReport {
        document_id,
        revision,
        signatures,
        signing_blocked_reason: checks.as_ref().err().map(ToString::to_string),
        warnings,
        empty_fields: checks
            .map(|fields| fields.empty_fields())
            .unwrap_or_default(),
        trust_store: trust.info()?,
    })
}

fn inspect_signatures(
    document: &Document,
    bytes: &[u8],
    trust: &TrustStore,
) -> Result<Vec<SignatureInfo>> {
    Ok(candidates(document)?.iter().enumerate().map(|(index, candidate)| {
        let mut info = SignatureInfo {
            id: candidate.id.map_or_else(
                || format!("direct-{}", index + 1),
                |id| format!("{}:{}", id.0, id.1),
            ),
            integrity: Integrity::Invalid,
            coverage: Coverage::Unknown,
            message: String::new(),
            digest: None,
            algorithm: None,
            signed_bytes: None,
            current_bytes: bytes.len(),
            certificate: None,
            chain: ChainEvaluation::not_checked(),
            claimed_signing_time: candidate.dictionary.get(b"M").ok()
                .filter(|value| value.as_str().is_ok_and(|value| value.len() <= 256))
                .and_then(|value| lopdf::decode_text_string(value).ok()),
            revocation: "not_checked",
            trusted_time: "not_checked",
        };
        match inspect_one(document, bytes, candidate, &mut info, trust) {
            Ok(()) => {
                info.integrity = Integrity::Intact;
                info.message = if info.coverage == Coverage::WholeDocument {
                    "The CMS signature and signed bytes are cryptographically intact. Certificate-chain trust is a separate check."
                } else {
                    "The signed revision is cryptographically intact, but later bytes are NOT covered by this signature. Review each later approval independently."
                }.into();
            }
            Err(error) => {
                info.integrity = error.status;
                info.message = error.message;
            }
        }
        info
    }).collect())
}

fn inspect_one(
    document: &Document,
    bytes: &[u8],
    candidate: &Candidate<'_>,
    info: &mut SignatureInfo,
    trust: &TrustStore,
) -> CheckResult<()> {
    if document.was_encrypted() || document.is_encrypted() || document.trailer.has(b"Encrypt") {
        return Err(CheckError::unsupported(
            "Signature verification on encrypted PDFs is not supported. Protection is left intact.",
        ));
    }
    if document
        .catalog()
        .map_err(|_| CheckError::invalid("The PDF catalog is malformed."))?
        .has(b"Perms")
        || candidate.dictionary.has(b"Reference")
    {
        return Err(CheckError::unsupported(
            "Certification/DocMDP and field-locking/FieldMDP policies are not supported. Folio makes no certification or permitted-change assurance.",
        ));
    }
    let id = candidate.id.ok_or_else(|| CheckError::unsupported(
        "Direct or embedded signature dictionaries are not supported; an ordinary indirect signature object is required.",
    ))?;
    let raw = pdf::signature_object(bytes, document, id)?;
    if raw.entries.contains_key(b"Reference".as_slice()) {
        return Err(CheckError::unsupported(
            "Signature transform and certification policies are not supported.",
        ));
    }
    if let Some(value) = raw.entries.get(b"Type".as_slice())
        && !matches!(&value.kind, pdf::Kind::Name(name) if name == b"Sig")
    {
        return Err(CheckError::unsupported(
            "Document timestamps and non-Sig signature types are not supported.",
        ));
    }
    if !matches!(
        raw.name(b"SubFilter")?,
        b"adbe.pkcs7.detached" | b"ETSI.CAdES.detached"
    ) {
        return Err(CheckError::unsupported(
            "Only adbe.pkcs7.detached and ETSI.CAdES.detached CMS content signatures are supported.",
        ));
    }
    let [_, first_length, second_start, second_length] = raw.byte_range(bytes.len())?;
    let signed_end = second_start + second_length;
    info.signed_bytes = Some(signed_end);
    info.coverage = if signed_end == bytes.len() {
        Coverage::WholeDocument
    } else {
        Coverage::LaterChanges
    };
    let signed_document = crate::protection::load(&bytes[..signed_end], None)
        .map_err(|_| {
            CheckError::invalid(
                "The signed byte spans do not contain a readable, complete PDF revision.",
            )
        })?
        .document;
    pdf::validate_footer(&bytes[..signed_end], &signed_document)?;
    let original = pdf::signature_object(&bytes[..signed_end], &signed_document, id)?;
    if original.offset != raw.offset || original.end != raw.end {
        return Err(CheckError::invalid(
            "The current signature object does not belong to the claimed signed revision.",
        ));
    }
    let original_field = signature_field(&signed_document, id)?;
    if original_field.is_none() {
        return Err(CheckError::invalid(
            "The signature is not bound to a signature field in the signed revision.",
        ));
    }
    if signature_field(document, id)? != original_field {
        return Err(CheckError::invalid(
            "The signature is no longer bound to the same unique logical signature field in the current document.",
        ));
    }
    let pdf::Kind::Hex(contents) = &raw.get(b"Contents")?.kind else {
        return Err(CheckError::invalid(
            "Signature Contents must be hexadecimal.",
        ));
    };
    let der = crypto::cms_der(contents)?;
    let mut details = crypto::cms_details(der)?;
    info.digest = Some(details.digest.clone());
    info.algorithm = Some(details.algorithm.clone());
    let mut data = Vec::with_capacity(first_length + second_length);
    data.extend_from_slice(&bytes[..first_length]);
    data.extend_from_slice(&bytes[second_start..signed_end]);
    let verified = crypto::verify(der, &data, &details);
    info.chain = trust.evaluate(&details.signer, &details.embedded);
    details.certificate.identity_trust = info.chain.status;
    info.certificate = Some(details.certificate);
    verified
}

fn signature_field(document: &Document, signature: ObjectId) -> CheckResult<Option<ObjectId>> {
    let fields =
        fields::read(document).map_err(|error| CheckError::unsupported(error.to_string()))?;
    let mut matching = fields
        .fields
        .iter()
        .filter(|field| field.signature == Some(signature));
    let first = matching.next().map(|field| field.id);
    if matching.next().is_some() {
        return Err(CheckError::invalid(
            "A signature is ambiguously bound to multiple logical fields.",
        ));
    }
    Ok(first)
}

pub(crate) fn signed_copy(
    document: &Document,
    bytes: &[u8],
    pfx: &[u8],
    password: &str,
    field_id: Option<&str>,
) -> Result<Vec<u8>> {
    let document = signing_source(document, bytes)?;
    if pfx.is_empty() || pfx.len() > MAX_IDENTITY_BYTES {
        return Err(invalid(
            "Choose a nonempty PKCS#12/PFX identity of at most 4 MiB.",
        ));
    }
    let signatures = inspect_signatures(&document, bytes, &TrustStore::default())?;
    let fields = signing_checks(&document, bytes, &signatures)?;
    if field_id.is_none() && fields.fields.len() >= MAX_SIGNATURES {
        return Err(invalid(
            "The 64-signature-field limit was reached. Select a supported existing empty field instead.",
        ));
    }
    let selected = field_id.map(|id| fields.selected(id)).transpose()?;
    incremental::signed_copy(&document, bytes, pfx, password, selected)
}

pub fn read_identity_file(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    if !path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("p12") || extension.eq_ignore_ascii_case("pfx")
    }) {
        return Err(invalid("Choose a .p12 or .pfx PKCS#12 identity file."));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_IDENTITY_BYTES as u64 {
        return Err(invalid("Choose a PKCS#12 identity file of at most 4 MiB."));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_IDENTITY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_IDENTITY_BYTES {
        return Err(invalid(
            "Choose a nonempty PKCS#12 identity file of at most 4 MiB.",
        ));
    }
    Ok(bytes)
}
