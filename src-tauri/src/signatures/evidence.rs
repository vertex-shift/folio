use std::{
    collections::{BTreeMap, HashSet},
    io::Read,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use der::{
    Decode, Encode,
    asn1::{GeneralizedTime, Uint},
};
use openssl::x509::{X509, X509Crl};
use serde::{Deserialize, Serialize};
use x509_cert::{
    Version,
    crl::CertificateList,
    ext::pkix::{
        AuthorityKeyIdentifier, CrlReason, KeyUsage, SubjectKeyIdentifier, name::GeneralName,
    },
};
use zeroize::Zeroizing;

use super::{
    CheckError, CheckResult, Integrity, crypto,
    timestamp::{TimestampToken, TrustedTimeEvaluation},
    trust::{TrustStore, VerifiedChain, parse_certificate},
};
use crate::document::{Result, invalid};

pub const MAX_CRL_BYTES: usize = 1024 * 1024;
pub const MAX_TIMESTAMP_BYTES: usize = 256 * 1024;
pub const MAX_EVIDENCE_ITEMS: usize = 32;
pub const MAX_EVIDENCE_STORE_BYTES: usize = 8 * 1024 * 1024;
const MAX_REVOKED_ENTRIES: usize = 32_768;
const MAX_TIMESTAMPS_PER_SIGNATURE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Crl,
    Timestamp,
}

impl EvidenceKind {
    pub fn max_bytes(self) -> usize {
        match self {
            Self::Crl => MAX_CRL_BYTES,
            Self::Timestamp => MAX_TIMESTAMP_BYTES,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceInfo {
    pub kind: EvidenceKind,
    pub fingerprint: String,
    pub bytes: usize,
    pub issuer: String,
    pub this_update: Option<String>,
    pub next_update: Option<String>,
    pub crl_number: Option<String>,
    pub gen_time: Option<String>,
    pub bound_certificate_fingerprint: Option<String>,
    pub bound_signature_value_sha256: Option<String>,
}

pub(crate) struct SignatureBinding {
    pub certificate_fingerprint: String,
    pub signature_value_sha256: String,
    pub signature_value: Vec<u8>,
}

impl SignatureBinding {
    pub(super) fn new(details: &crypto::CmsDetails) -> Self {
        Self {
            certificate_fingerprint: details.certificate.sha256_fingerprint.clone(),
            signature_value_sha256: crypto::fingerprint(&details.signature_value),
            signature_value: details.signature_value.clone(),
        }
    }

    fn matches(&self, info: &EvidenceInfo) -> bool {
        info.bound_certificate_fingerprint.as_ref() == Some(&self.certificate_fingerprint)
            && info.bound_signature_value_sha256.as_ref() == Some(&self.signature_value_sha256)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationStatus {
    NotChecked,
    GoodCurrentEvidence,
    Revoked,
    Incomplete,
    Missing,
    Expired,
    Future,
    Invalid,
    Unsupported,
    Untrusted,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateRevocation {
    pub certificate_fingerprint: String,
    pub serial: String,
    pub issuer_fingerprint: String,
    pub status: RevocationStatus,
    pub message: String,
    pub evidence_fingerprint: Option<String>,
    pub this_update: Option<String>,
    pub next_update: Option<String>,
    pub revocation_time: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevocationEvaluation {
    pub status: RevocationStatus,
    pub message: String,
    pub checked_at: Option<String>,
    pub certificates: Vec<CertificateRevocation>,
}

impl RevocationEvaluation {
    pub(super) fn not_checked(message: impl Into<String>) -> Self {
        Self {
            status: RevocationStatus::NotChecked,
            message: message.into(),
            checked_at: None,
            certificates: vec![],
        }
    }
}

pub(super) struct CrlEvidence {
    info: EvidenceInfo,
    parsed: CertificateList,
    crl: X509Crl,
    aki: AuthorityKeyIdentifier,
    number: Vec<u8>,
}

pub(super) enum EvidenceItem {
    Crl(Box<CrlEvidence>),
    Timestamp {
        info: EvidenceInfo,
        token: Box<TimestampToken>,
    },
}

impl EvidenceItem {
    pub(super) fn info(&self) -> &EvidenceInfo {
        match self {
            Self::Crl(crl) => &crl.info,
            Self::Timestamp { info, .. } => info,
        }
    }
}

#[derive(Default)]
pub(super) struct EvidenceStore {
    items: BTreeMap<String, EvidenceItem>,
}

pub(super) fn now() -> CheckResult<Duration> {
    SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        CheckError::invalid("The current system time cannot be used for offline evidence checks.")
    })
}

pub(super) fn time_label(time: Duration) -> CheckResult<String> {
    der::DateTime::from_unix_duration(time)
        .map(|value| value.to_string())
        .map_err(|_| CheckError::invalid("The evidence date is outside the supported time range."))
}

fn evidence_error(_: impl std::fmt::Debug) -> CheckError {
    CheckError::invalid("The public CRL is not a supported, exact DER/PEM structure.")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn strong_crl_algorithm(parsed: &CertificateList) -> CheckResult<()> {
    let algorithm = &parsed.signature_algorithm;
    let rsa = matches!(
        algorithm.oid.to_string().as_str(),
        "1.2.840.113549.1.1.11" | "1.2.840.113549.1.1.12" | "1.2.840.113549.1.1.13"
    );
    let ec = matches!(
        algorithm.oid.to_string().as_str(),
        "1.2.840.10045.4.3.2" | "1.2.840.10045.4.3.3" | "1.2.840.10045.4.3.4"
    );
    if !rsa && !ec
        || rsa
            && algorithm
                .parameters
                .as_ref()
                .is_some_and(|value| !value.is_null())
        || ec && algorithm.parameters.is_some()
    {
        return Err(CheckError::unsupported(
            "CRLs require SHA-256/384/512 RSA PKCS#1 or supported ECDSA signatures with canonical parameters.",
        ));
    }
    if algorithm != &parsed.tbs_cert_list.signature
        || parsed
            .signature
            .as_bytes()
            .is_none_or(|value| value.is_empty())
    {
        return Err(CheckError::invalid(
            "The CRL signature declarations or signature bits are invalid.",
        ));
    }
    Ok(())
}

impl CrlEvidence {
    fn parse(bytes: &[u8]) -> CheckResult<Self> {
        let crl = if bytes.trim_ascii().starts_with(b"-----BEGIN") {
            let trimmed = bytes.trim_ascii();
            let body = trimmed.strip_prefix(b"-----BEGIN X509 CRL-----")
                .and_then(|value| value.strip_suffix(b"-----END X509 CRL-----"))
                .filter(|value| !value.is_empty() && value.iter().all(|byte| {
                    byte.is_ascii_alphanumeric() || b"+/=".contains(byte) || byte.is_ascii_whitespace()
                })).ok_or_else(|| CheckError::invalid(
                    "Choose exactly one public X509 CRL PEM block, without certificates, private keys, bundles or other data.",
                ))?;
            if body.iter().all(u8::is_ascii_whitespace) {
                return Err(CheckError::invalid("The X509 CRL PEM block is empty."));
            }
            X509Crl::from_pem(trimmed).map_err(evidence_error)?
        } else {
            let parsed = CertificateList::from_der(bytes).map_err(evidence_error)?;
            if parsed.to_der().map_err(evidence_error)? != bytes {
                return Err(CheckError::invalid(
                    "The CRL encoding is not canonical, exact DER.",
                ));
            }
            X509Crl::from_der(bytes).map_err(evidence_error)?
        };
        let der = crl.to_der().map_err(evidence_error)?;
        let parsed = CertificateList::from_der(&der).map_err(evidence_error)?;
        strong_crl_algorithm(&parsed)?;
        let tbs = &parsed.tbs_cert_list;
        if tbs.version != Version::V2 {
            return Err(CheckError::unsupported(
                "Only full, direct version-2 CRLs with AKI and CRL number are supported.",
            ));
        }
        let next = tbs.next_update.ok_or_else(|| {
            CheckError::unsupported(
                "A CRL without nextUpdate cannot establish current revocation coverage.",
            )
        })?;
        if next.to_unix_duration() <= tbs.this_update.to_unix_duration() {
            return Err(CheckError::invalid(
                "CRL nextUpdate must be after thisUpdate.",
            ));
        }
        let mut seen = HashSet::new();
        let mut aki = None;
        let mut number = None;
        for extension in tbs.crl_extensions.as_deref().unwrap_or_default() {
            if !seen.insert(extension.extn_id) {
                return Err(CheckError::invalid("The CRL has duplicate extensions."));
            }
            if extension.critical {
                return Err(CheckError::unsupported(
                    "Critical/scoped CRL extensions are not supported. Delta, indirect and distribution-point-scoped CRLs are refused.",
                ));
            }
            match extension.extn_id.to_string().as_str() {
                "2.5.29.35" => {
                    aki = Some(
                        AuthorityKeyIdentifier::from_der(extension.extn_value.as_bytes())
                            .map_err(evidence_error)?,
                    )
                }
                "2.5.29.20" => {
                    let value =
                        Uint::from_der(extension.extn_value.as_bytes()).map_err(evidence_error)?;
                    if value.as_bytes().len() > 20 {
                        return Err(CheckError::unsupported(
                            "CRL numbers above 160 bits are unsupported.",
                        ));
                    }
                    number = Some(value.as_bytes().to_vec());
                }
                _ => {
                    return Err(CheckError::unsupported(
                        "Only full direct CRLs with AKI and CRL-number extensions are supported; delta, indirect, freshest and other scopes/extensions are refused.",
                    ));
                }
            }
        }
        let aki = aki
            .filter(|value| {
                value
                    .key_identifier
                    .as_ref()
                    .is_some_and(|key| !key.as_bytes().is_empty())
            })
            .ok_or_else(|| {
                CheckError::unsupported(
                    "A full CRL must include an authority key identifier with a key ID.",
                )
            })?;
        let number = number
            .ok_or_else(|| CheckError::unsupported("A full CRL must include a CRL number."))?;
        let entries = tbs.revoked_certificates.as_deref().unwrap_or_default();
        if entries.len() > MAX_REVOKED_ENTRIES
            || tbs.revoked_certificates.as_ref().is_some_and(Vec::is_empty)
        {
            return Err(CheckError::unsupported(
                "A CRL must omit an empty revoked list and may list at most 32,768 entries.",
            ));
        }
        let mut serials = HashSet::new();
        for entry in entries {
            if !serials.insert(entry.serial_number.as_bytes().to_vec()) {
                return Err(CheckError::invalid(
                    "The CRL has duplicate revoked certificate serials.",
                ));
            }
            if entry.revocation_date.to_unix_duration() > tbs.this_update.to_unix_duration() {
                return Err(CheckError::invalid(
                    "A CRL revocation entry is dated after thisUpdate.",
                ));
            }
            let mut seen = HashSet::new();
            for extension in entry.crl_entry_extensions.as_deref().unwrap_or_default() {
                if extension.critical || !seen.insert(extension.extn_id) {
                    return Err(CheckError::unsupported(
                        "Critical or duplicate CRL-entry extensions are refused.",
                    ));
                }
                match extension.extn_id.to_string().as_str() {
                    "2.5.29.21" => {
                        let reason = CrlReason::from_der(extension.extn_value.as_bytes())
                            .map_err(evidence_error)? as u32;
                        if !matches!(reason, 0..=6 | 9 | 10) {
                            return Err(CheckError::unsupported(
                                "removeFromCRL and unsupported CRL reasons are refused in full CRLs.",
                            ));
                        }
                    }
                    "2.5.29.24" => {
                        let date = GeneralizedTime::from_der(extension.extn_value.as_bytes())
                            .map_err(evidence_error)?;
                        if date.to_unix_duration() > tbs.this_update.to_unix_duration() {
                            return Err(CheckError::invalid(
                                "The CRL invalidityDate is after thisUpdate.",
                            ));
                        }
                    }
                    _ => {
                        return Err(CheckError::unsupported(
                            "Indirect certificateIssuer and other CRL-entry extensions are not supported.",
                        ));
                    }
                }
            }
        }
        let info = EvidenceInfo {
            kind: EvidenceKind::Crl,
            fingerprint: crypto::fingerprint(&der),
            bytes: der.len(),
            issuer: crypto::name(crl.issuer_name())?,
            this_update: Some(time_label(tbs.this_update.to_unix_duration())?),
            next_update: Some(time_label(next.to_unix_duration())?),
            crl_number: Some(hex(&number)),
            gen_time: None,
            bound_certificate_fingerprint: None,
            bound_signature_value_sha256: None,
        };
        Ok(Self {
            info,
            parsed,
            crl,
            aki,
            number,
        })
    }

    fn verify_issuer(&self, issuer: &X509) -> CheckResult<()> {
        let certificate = parse_certificate(issuer)?;
        if self.parsed.tbs_cert_list.issuer != certificate.tbs_certificate.subject {
            return Err(CheckError::invalid(
                "The CRL issuer is not this chain certificate's issuer.",
            ));
        }
        if !certificate
            .tbs_certificate
            .get::<KeyUsage>()
            .map_err(evidence_error)?
            .is_some_and(|(_, usage)| usage.crl_sign())
        {
            return Err(CheckError::unsupported(
                "The issuing CA does not authorize CRL signing.",
            ));
        }
        let ski = certificate
            .tbs_certificate
            .get::<SubjectKeyIdentifier>()
            .map_err(evidence_error)?
            .ok_or_else(|| {
                CheckError::unsupported("CRL issuer binding requires a CA subject key identifier.")
            })?
            .1;
        if self.aki.key_identifier.as_ref() != Some(&ski.0) {
            return Err(CheckError::invalid(
                "The CRL authority key ID does not match the actual issuing CA.",
            ));
        }
        match (
            &self.aki.authority_cert_issuer,
            &self.aki.authority_cert_serial_number,
        ) {
            (None, None) => {}
            (Some(names), Some(serial))
                if serial == &certificate.tbs_certificate.serial_number
                    && names.len() == 1
                    && matches!(&names[0], GeneralName::DirectoryName(name) if name == &certificate.tbs_certificate.issuer) =>
                {}
            _ => {
                return Err(CheckError::invalid(
                    "The CRL authority issuer/serial does not identify the actual CA.",
                ));
            }
        }
        let key = issuer.public_key().map_err(evidence_error)?;
        crypto::validate_key(&key)?;
        if !self.crl.verify(&key).map_err(evidence_error)? {
            return Err(CheckError::invalid(
                "The CRL signature does not verify with the actual issuing CA key.",
            ));
        }
        Ok(())
    }
}

impl EvidenceStore {
    pub(super) fn info(&self) -> Vec<EvidenceInfo> {
        self.items
            .values()
            .map(|item| item.info().clone())
            .collect()
    }

    pub(super) fn bytes(&self) -> usize {
        self.items.values().map(|item| item.info().bytes).sum()
    }

    pub(super) fn prepare(
        kind: EvidenceKind,
        bytes: &[u8],
        binding: Option<&SignatureBinding>,
    ) -> CheckResult<EvidenceItem> {
        if bytes.is_empty() || bytes.len() > kind.max_bytes() {
            return Err(CheckError::invalid(format!(
                "Choose nonempty public evidence of at most {} bytes.",
                kind.max_bytes()
            )));
        }
        match kind {
            EvidenceKind::Crl if binding.is_none() => {
                Ok(EvidenceItem::Crl(Box::new(CrlEvidence::parse(bytes)?)))
            }
            EvidenceKind::Crl => Err(CheckError::invalid(
                "CRLs are issuer evidence, not signature-specific timestamp tokens.",
            )),
            EvidenceKind::Timestamp => {
                let binding = binding.ok_or_else(|| {
                    CheckError::invalid("Select the actual PDF signature to bind this timestamp.")
                })?;
                let token = TimestampToken::parse(bytes)?;
                token.check_imprint(binding)?;
                let info = EvidenceInfo {
                    kind,
                    fingerprint: token.fingerprint.clone(),
                    bytes: token.der_bytes,
                    issuer: token.details.certificate.subject.clone(),
                    this_update: None,
                    next_update: None,
                    crl_number: None,
                    gen_time: Some(token.gen_time_label.clone()),
                    bound_certificate_fingerprint: Some(binding.certificate_fingerprint.clone()),
                    bound_signature_value_sha256: Some(binding.signature_value_sha256.clone()),
                };
                Ok(EvidenceItem::Timestamp {
                    info,
                    token: Box::new(token),
                })
            }
        }
    }

    pub(super) fn check_add(&self, item: &EvidenceItem) -> CheckResult<()> {
        if self.items.contains_key(&item.info().fingerprint) {
            return Err(CheckError::invalid(
                "That public evidence is already loaded.",
            ));
        }
        if self.items.len() >= MAX_EVIDENCE_ITEMS
            || self.bytes() + item.info().bytes > MAX_EVIDENCE_STORE_BYTES
        {
            return Err(CheckError::invalid(
                "Public evidence is limited to 32 items and 8 MiB in this app run.",
            ));
        }
        if item.info().kind == EvidenceKind::Timestamp
            && self
                .items
                .values()
                .filter(|existing| {
                    existing.info().kind == EvidenceKind::Timestamp
                        && existing.info().bound_certificate_fingerprint
                            == item.info().bound_certificate_fingerprint
                        && existing.info().bound_signature_value_sha256
                            == item.info().bound_signature_value_sha256
                })
                .count()
                >= MAX_TIMESTAMPS_PER_SIGNATURE
        {
            return Err(CheckError::invalid(
                "At most four imported timestamps may be bound to one PDF signature.",
            ));
        }
        Ok(())
    }

    pub(super) fn get(&self, fingerprint: &str) -> Option<&EvidenceItem> {
        self.items.get(fingerprint)
    }
    pub(super) fn insert(&mut self, item: EvidenceItem) {
        self.items.insert(item.info().fingerprint.clone(), item);
    }
    pub(super) fn remove(&mut self, fingerprint: &str) {
        self.items.remove(fingerprint);
    }

    pub(super) fn revocation(&self, chain: &VerifiedChain) -> RevocationEvaluation {
        if chain.evaluation.status != "anchored_custom_store" || chain.certificates.len() < 2 {
            let mut result = RevocationEvaluation::not_checked(
                "Current revocation coverage requires a valid current-date path to an explicitly loaded custom CA. Embedded certificates are not trust anchors.",
            );
            if chain.evaluation.status == "untrusted" {
                result.status = RevocationStatus::Untrusted;
            }
            return result;
        }
        match self.revocation_checked(chain) {
            Ok(result) => result,
            Err(error) => RevocationEvaluation {
                status: if error.status == Integrity::Unsupported {
                    RevocationStatus::Unsupported
                } else {
                    RevocationStatus::Invalid
                },
                message: error.message,
                checked_at: None,
                certificates: vec![],
            },
        }
    }

    fn revocation_checked(&self, chain: &VerifiedChain) -> CheckResult<RevocationEvaluation> {
        let now = now()?;
        let mut certificates = Vec::new();
        for pair in chain.certificates.windows(2) {
            certificates.push(self.certificate_revocation(&pair[0], &pair[1], now)?);
        }
        let status = [
            RevocationStatus::Revoked,
            RevocationStatus::Invalid,
            RevocationStatus::Unsupported,
            RevocationStatus::Future,
            RevocationStatus::Expired,
            RevocationStatus::Missing,
        ]
        .into_iter()
        .find(|status| certificates.iter().any(|row| row.status == *status))
        .map(|status| {
            if status == RevocationStatus::Missing {
                RevocationStatus::Incomplete
            } else {
                status
            }
        })
        .unwrap_or(RevocationStatus::GoodCurrentEvidence);
        let message = if status == RevocationStatus::GoodCurrentEvidence {
            "No listed revocation in current, cryptographically verified full CRLs for every non-anchor certificate. Only your imported lists were checked, not OCSP or historical revocation."
        } else if status == RevocationStatus::Revoked {
            "A certificate in the actual signer path is revoked in a current, verified imported CRL. An intact PDF or valid timestamp does not clear this current revocation."
        } else {
            "Imported CRLs do not establish complete current revocation coverage for this path. Missing, expired, future, invalid or unsupported evidence is not a good result."
        };
        Ok(RevocationEvaluation {
            status,
            message: message.into(),
            checked_at: Some(time_label(now)?),
            certificates,
        })
    }

    fn certificate_revocation(
        &self,
        certificate: &X509,
        issuer: &X509,
        now: Duration,
    ) -> CheckResult<CertificateRevocation> {
        let cert = parse_certificate(certificate)?;
        let ca = parse_certificate(issuer)?;
        let info = crypto::certificate_info(certificate)?;
        let mut row = CertificateRevocation {
            certificate_fingerprint: info.sha256_fingerprint,
            serial: info.serial,
            issuer_fingerprint: crypto::certificate_info(issuer)?.sha256_fingerprint,
            status: RevocationStatus::Missing,
            message: "No imported full CRL covers this actual issuing CA.".into(),
            evidence_fingerprint: None,
            this_update: None,
            next_update: None,
            revocation_time: None,
        };
        if cert.tbs_certificate.issuer != ca.tbs_certificate.subject {
            return Err(CheckError::invalid(
                "The verified chain's certificate/issuer association is inconsistent.",
            ));
        }
        let mut matching = Vec::new();
        for item in self.items.values() {
            if let EvidenceItem::Crl(crl) = item
                && crl.parsed.tbs_cert_list.issuer == ca.tbs_certificate.subject
            {
                if let Err(error) = crl.verify_issuer(issuer) {
                    row.status = if error.status == Integrity::Unsupported {
                        RevocationStatus::Unsupported
                    } else {
                        RevocationStatus::Invalid
                    };
                    row.message = error.message;
                    row.evidence_fingerprint = Some(crl.info.fingerprint.clone());
                    return Ok(row);
                }
                matching.push(crl.as_ref());
            }
        }
        matching.sort_by(|a, b| (a.number.len(), &a.number).cmp(&(b.number.len(), &b.number)));
        for pair in matching.windows(2) {
            if pair[0].number == pair[1].number
                || pair[0].parsed.tbs_cert_list.this_update.to_unix_duration()
                    > pair[1].parsed.tbs_cert_list.this_update.to_unix_duration()
            {
                row.status = RevocationStatus::Invalid;
                row.message = "Imported issuer CRLs have conflicting numbers or nonmonotonic update dates; no older-good fallback is used.".into();
                return Ok(row);
            }
        }
        let Some(crl) = matching.last() else {
            return Ok(row);
        };
        row.evidence_fingerprint = Some(crl.info.fingerprint.clone());
        row.this_update = crl.info.this_update.clone();
        row.next_update = crl.info.next_update.clone();
        let tbs = &crl.parsed.tbs_cert_list;
        if tbs.this_update.to_unix_duration() > now {
            row.status = RevocationStatus::Future;
            row.message = "The newest authenticated issuer CRL is future-dated; no older-good fallback is used.".into();
        } else if tbs
            .next_update
            .is_none_or(|next| next.to_unix_duration() < now)
        {
            row.status = RevocationStatus::Expired;
            row.message = "The newest authenticated issuer CRL has expired; it does not establish current status.".into();
        } else if let Some(entry) = tbs
            .revoked_certificates
            .as_deref()
            .unwrap_or_default()
            .iter()
            .find(|entry| entry.serial_number == cert.tbs_certificate.serial_number)
        {
            row.status = RevocationStatus::Revoked;
            row.message = "This exact certificate serial is revoked in the current, verified full issuer CRL.".into();
            row.revocation_time = Some(time_label(entry.revocation_date.to_unix_duration())?);
        } else {
            row.status = RevocationStatus::GoodCurrentEvidence;
            row.message =
                "This exact serial is not listed in the newest current, verified full issuer CRL."
                    .into();
        }
        Ok(row)
    }

    pub(super) fn timestamps(
        &self,
        binding: &SignatureBinding,
        embedded: Option<&[u8]>,
        trust: &TrustStore,
    ) -> TrustedTimeEvaluation {
        let mut results = Vec::new();
        if let Some(bytes) = embedded {
            results.push(super::timestamp::evaluate_embedded(bytes, binding, trust));
        }
        for item in self.items.values() {
            if let EvidenceItem::Timestamp { info, token } = item
                && binding.matches(info)
            {
                results.push(token.evaluate(binding, trust, "imported"));
            }
        }
        TrustedTimeEvaluation::from_timestamps(results)
    }
}

pub fn read_evidence_file(path: &Path, kind: EvidenceKind) -> Result<Zeroizing<Vec<u8>>> {
    let allowed: &[&str] = match kind {
        EvidenceKind::Crl => &["crl", "pem", "der"],
        EvidenceKind::Timestamp => &["tsr", "tst", "der"],
    };
    if !path.extension().is_some_and(|extension| {
        allowed
            .iter()
            .any(|value| extension.eq_ignore_ascii_case(value))
    }) {
        return Err(invalid(match kind {
            EvidenceKind::Crl => "Choose one public .crl, .pem or .der CRL.",
            EvidenceKind::Timestamp => "Choose one DER .tsr response or .tst/.der RFC3161 token.",
        }));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > kind.max_bytes() as u64 {
        return Err(invalid(
            "The selected public evidence exceeds its file-size limit.",
        ));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(kind.max_bytes() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > kind.max_bytes() {
        return Err(invalid(
            "Choose nonempty public evidence within its file-size limit.",
        ));
    }
    Ok(bytes)
}
