mod crypto;
mod pdf;

use std::{collections::HashSet, io::Read, path::Path};

use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat, dictionary};
use serde::Serialize;
use zeroize::Zeroizing;

use crate::document::{MAX_DOCUMENT_BYTES, Result, invalid};

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
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureReport {
    pub document_id: u64,
    pub revision: u64,
    pub signatures: Vec<SignatureInfo>,
    pub signing_blocked_reason: Option<String>,
    pub warnings: Vec<String>,
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

pub(crate) fn ensure_signable(document: &Document) -> Result<()> {
    if document.was_encrypted() || document.is_encrypted() || document.trailer.has(b"Encrypt") {
        return Err(invalid(
            "Signing encrypted PDFs is not supported. Folio will not remove or weaken their password protection.",
        ));
    }
    if has_signatures(document) {
        return Err(invalid(
            "This PDF already contains a signature or certification policy. Adding signatures or certifying PDFs is not supported; existing signatures will not be rewritten.",
        ));
    }
    if document.xref_recovery.is_some() {
        return Err(invalid(
            "Signing PDFs that required cross-reference repair is not supported.",
        ));
    }
    if document.trailer.has(b"Prev") || document.trailer.has(b"XRefStm") {
        return Err(invalid(
            "Signing incremental or hybrid-reference PDFs is not supported in this first signing workflow.",
        ));
    }
    for object in document.objects.values() {
        walk(object, None, 0, &mut |_, dictionary| {
            if dictionary.get(b"FT").and_then(Object::as_name).ok() == Some(b"Sig") {
                return Err(CheckError::unsupported(
                    "Existing signature fields, including empty fields, are not supported for signed-copy export.",
                ));
            }
            Ok(())
        })?;
    }
    if let Ok(form) = document.catalog()?.get(b"AcroForm") {
        let form = document.dereference(form)?.1.as_dict()?;
        if form.has(b"XFA") {
            return Err(invalid("Signing XFA forms is not supported."));
        }
        if let Ok(fields) = form.get(b"Fields") {
            document.dereference(fields)?.1.as_array()?;
        }
    }
    Ok(())
}

pub(crate) fn inspect(
    document: &Document,
    bytes: &[u8],
    document_id: u64,
    revision: u64,
) -> Result<SignatureReport> {
    let candidates = candidates(document)?;
    let warnings = if candidates.is_empty() && has_signatures(document) {
        vec!["Signature fields or certification markers are present, but no supported signature dictionary was found. This is not an unsigned-document assurance.".into()]
    } else {
        Vec::new()
    };
    let signatures = candidates.iter().enumerate().map(|(index, candidate)| {
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
        };
        match inspect_one(document, bytes, candidate, &mut info) {
            Ok(()) => {
                info.integrity = Integrity::Intact;
                info.message = if info.coverage == Coverage::WholeDocument {
                    "The CMS signature and signed bytes are cryptographically intact. Identity trust has not been checked."
                } else {
                    "The signed revision is cryptographically intact, but later bytes are NOT covered. Do not treat the current document as fully signed."
                }.into();
            }
            Err(error) => {
                info.integrity = error.status;
                info.message = error.message;
            }
        }
        info
    }).collect();
    Ok(SignatureReport {
        document_id,
        revision,
        signatures,
        signing_blocked_reason: ensure_signable(document)
            .err()
            .map(|error| error.to_string()),
        warnings,
    })
}

fn inspect_one(
    document: &Document,
    bytes: &[u8],
    candidate: &Candidate<'_>,
    info: &mut SignatureInfo,
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
    if !signature_field_references(&signed_document, id)? {
        return Err(CheckError::invalid(
            "The signature is not bound to a signature field in the signed revision.",
        ));
    }
    let pdf::Kind::Hex(contents) = &raw.get(b"Contents")?.kind else {
        return Err(CheckError::invalid(
            "Signature Contents must be hexadecimal.",
        ));
    };
    let der = crypto::cms_der(contents)?;
    let details = crypto::cms_details(der)?;
    info.digest = Some(details.digest.clone());
    info.algorithm = Some(details.algorithm.clone());
    let mut data = Vec::with_capacity(first_length + second_length);
    data.extend_from_slice(&bytes[..first_length]);
    data.extend_from_slice(&bytes[second_start..signed_end]);
    let verified = crypto::verify(der, &data, &details);
    info.certificate = Some(details.certificate);
    verified
}

fn signature_field_references(document: &Document, signature: ObjectId) -> CheckResult<bool> {
    fn visit(
        document: &Document,
        value: &Object,
        inherited_signature: bool,
        signature: ObjectId,
        depth: usize,
        visited: &mut HashSet<ObjectId>,
    ) -> CheckResult<bool> {
        if depth > 64 {
            return Err(CheckError::invalid(
                "The signature field tree is excessively deep.",
            ));
        }
        let (id, object) = document
            .dereference(value)
            .map_err(|_| CheckError::invalid("A signature field reference is malformed."))?;
        if let Some(id) = id
            && !visited.insert(id)
        {
            return Err(CheckError::invalid(
                "The signature field tree contains a cycle or duplicate field.",
            ));
        }
        let field = object
            .as_dict()
            .map_err(|_| CheckError::invalid("A signature field is not a dictionary."))?;
        let is_signature = match field.get(b"FT") {
            Ok(value) => {
                value
                    .as_name()
                    .map_err(|_| CheckError::invalid("The signature field type is malformed."))?
                    == b"Sig"
            }
            Err(_) => inherited_signature,
        };
        if is_signature && field.get(b"V").and_then(Object::as_reference).ok() == Some(signature) {
            return Ok(true);
        }
        if let Ok(children) = field.get(b"Kids") {
            for child in document
                .dereference(children)
                .and_then(|(_, value)| value.as_array())
                .map_err(|_| CheckError::invalid("The signature field children are malformed."))?
            {
                if visit(document, child, is_signature, signature, depth + 1, visited)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    let catalog = document
        .catalog()
        .map_err(|_| CheckError::invalid("The signed revision has no catalog."))?;
    let form = catalog
        .get(b"AcroForm")
        .and_then(|value| document.dereference(value))
        .and_then(|(_, value)| value.as_dict())
        .map_err(|_| CheckError::invalid("The signed revision has no readable AcroForm."))?;
    let fields = form
        .get(b"Fields")
        .and_then(|value| document.dereference(value))
        .and_then(|(_, value)| value.as_array())
        .map_err(|_| {
            CheckError::invalid("The signed revision has no readable signature fields.")
        })?;
    let mut visited = HashSet::new();
    for field in fields {
        if visit(document, field, false, signature, 0, &mut visited)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn signed_copy(document: &Document, pfx: &[u8], password: &str) -> Result<Vec<u8>> {
    ensure_signable(document)?;
    if pfx.is_empty() || pfx.len() > MAX_IDENTITY_BYTES {
        return Err(invalid(
            "Choose a nonempty PKCS#12/PFX identity of at most 4 MiB.",
        ));
    }
    let identity = crypto::identity(pfx, password)?;
    let mut output = document.clone();
    let signature = output.add_object(dictionary! {
        "Type" => "Sig",
        "Filter" => "Adobe.PPKLite",
        "SubFilter" => "adbe.pkcs7.detached",
        "ByteRange" => vec![
            Object::Integer(0),
            Object::Integer(9_999_999_999),
            Object::Integer(9_999_999_999),
            Object::Integer(9_999_999_999),
        ],
        "Contents" => Object::String(vec![0; RESERVED_CMS_BYTES], StringFormat::Hexadecimal),
    });
    let first_page = output
        .get_pages()
        .values()
        .next()
        .copied()
        .ok_or_else(|| invalid("The PDF has no page for its invisible signature field."))?;
    let field = output.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Sig",
        "T" => Object::string_literal(format!("FolioSignature{}", signature.0)),
        "V" => signature,
        "P" => first_page,
        "Rect" => vec![0.into(), 0.into(), 0.into(), 0.into()],
        "F" => 132,
    });
    let mut annotations = match output.get_dictionary(first_page)?.get(b"Annots") {
        Ok(value) => output.dereference(value)?.1.as_array()?.clone(),
        Err(_) => Vec::new(),
    };
    annotations.push(field.into());
    output
        .get_dictionary_mut(first_page)?
        .set("Annots", annotations);
    let mut form = match output.catalog()?.get(b"AcroForm") {
        Ok(value) => output.dereference(value)?.1.as_dict()?.clone(),
        Err(_) => Dictionary::new(),
    };
    let mut fields = match form.get(b"Fields") {
        Ok(value) => output.dereference(value)?.1.as_array()?.clone(),
        Err(_) => Vec::new(),
    };
    fields.push(field.into());
    form.set("Fields", fields);
    form.set("SigFlags", 3);
    let form_id = output.add_object(form);
    output.catalog_mut()?.set("AcroForm", form_id);
    let mut bytes = Vec::new();
    output.save_to(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("The signed copy exceeds the 128 MiB PDF limit."));
    }
    let loaded = crate::protection::load(&bytes, None)?.document;
    let raw = pdf::signature_object(&bytes, &loaded, signature)?;
    let contents = raw.get(b"Contents")?.span.clone();
    let byte_range = raw.get(b"ByteRange")?.span.clone();
    let replacement = format!(
        "[0 {} {} {}]",
        contents.start,
        contents.end,
        bytes.len() - contents.end
    );
    if replacement.len() > byte_range.len() || contents.len() != RESERVED_CMS_BYTES * 2 + 2 {
        return Err(invalid(
            "The PDF writer produced incompatible signature placeholder bounds.",
        ));
    }
    bytes[byte_range.clone()].fill(b' ');
    bytes[byte_range.start..byte_range.start + replacement.len()]
        .copy_from_slice(replacement.as_bytes());
    let mut data = Vec::with_capacity(bytes.len() - contents.len());
    data.extend_from_slice(&bytes[..contents.start]);
    data.extend_from_slice(&bytes[contents.end..]);
    let cms = crypto::sign(&identity, &data)?;
    if cms.len() > RESERVED_CMS_BYTES {
        return Err(invalid(
            "The certificate chain/signature exceeds the 32 KiB signing reservation. No file was exported.",
        ));
    }
    const HEX: &[u8] = b"0123456789ABCDEF";
    for (index, byte) in cms.iter().enumerate() {
        bytes[contents.start + 1 + index * 2] = HEX[(byte >> 4) as usize];
        bytes[contents.start + 2 + index * 2] = HEX[(byte & 15) as usize];
    }
    let final_document = crate::protection::load(&bytes, None)?.document;
    let report = inspect(&final_document, &bytes, 0, 0)?;
    if report.signatures.len() != 1
        || report.signatures[0].integrity != Integrity::Intact
        || report.signatures[0].coverage != Coverage::WholeDocument
    {
        return Err(invalid(
            "The newly signed PDF did not pass its final structural and cryptographic verification. No file was exported.",
        ));
    }
    Ok(bytes)
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
