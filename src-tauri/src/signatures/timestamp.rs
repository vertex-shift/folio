use std::{collections::HashSet, time::Duration};

use cms::content_info::ContentInfo;
use der::{
    Any, Decode, Encode, Sequence, Tag, Tagged,
    asn1::{BitString, ObjectIdentifier, OctetString, Uint},
};
use openssl::hash::hash;
use serde::Serialize;
use x509_cert::{
    ext::{
        Extensions,
        pkix::{
            SubjectAltName,
            name::{GeneralName, GeneralNames},
        },
    },
    serial_number::SerialNumber,
    spki::{AlgorithmIdentifierOwned, AlgorithmIdentifierRef},
};

use super::{
    CertificateInfo, CheckError, CheckResult, Integrity, crypto,
    evidence::{
        MAX_TIMESTAMP_BYTES, RevocationEvaluation, RevocationStatus, SignatureBinding, now,
    },
    trust::{
        ChainEvaluation, TrustStore, parse_certificate, strong_certificate, timestamp_purpose,
    },
};

#[derive(Sequence)]
struct MessageImprint {
    hash_algorithm: AlgorithmIdentifierOwned,
    hashed_message: OctetString,
}

#[derive(Sequence)]
struct Accuracy {
    seconds: Option<u64>,
    #[asn1(context_specific = "0", tag_mode = "IMPLICIT", optional = "true")]
    millis: Option<u16>,
    #[asn1(context_specific = "1", tag_mode = "IMPLICIT", optional = "true")]
    micros: Option<u16>,
}

#[derive(Sequence)]
struct TstInfo {
    version: u8,
    policy: ObjectIdentifier,
    message_imprint: MessageImprint,
    serial_number: Uint,
    gen_time: Any,
    accuracy: Option<Accuracy>,
    #[asn1(default = "bool::default")]
    ordering: bool,
    nonce: Option<Uint>,
    #[asn1(context_specific = "0", tag_mode = "EXPLICIT", optional = "true")]
    tsa: Option<GeneralName>,
    #[asn1(context_specific = "1", tag_mode = "IMPLICIT", optional = "true")]
    extensions: Option<Extensions>,
}

#[derive(Sequence)]
struct PkiStatusInfo {
    status: u8,
    status_string: Option<Vec<String>>,
    fail_info: Option<BitString>,
}

#[derive(Sequence)]
struct TimeStampResp {
    status: PkiStatusInfo,
    time_stamp_token: Option<ContentInfo>,
}

#[derive(Sequence)]
struct IssuerSerial {
    issuer: GeneralNames,
    serial_number: SerialNumber,
}

#[derive(Sequence)]
struct EssCertIdV2<'a> {
    #[asn1(default = "default_sha256_algorithm")]
    hash_algorithm: AlgorithmIdentifierRef<'a>,
    cert_hash: OctetString,
    issuer_serial: Option<IssuerSerial>,
}

fn default_sha256_algorithm<'a>() -> AlgorithmIdentifierRef<'a> {
    AlgorithmIdentifierRef {
        oid: ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1"),
        parameters: None,
    }
}

#[derive(Sequence)]
struct SigningCertificateV2<'a> {
    certs: Vec<EssCertIdV2<'a>>,
    policies: Option<Any>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustedTimeStatus {
    NotChecked,
    VerifiedCustomStore,
    Incomplete,
    Revoked,
    Future,
    Untrusted,
    Invalid,
    Unsupported,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimestampEvaluation {
    pub status: TrustedTimeStatus,
    pub message: String,
    pub source: &'static str,
    pub evidence_fingerprint: String,
    pub cryptographic_integrity: Integrity,
    pub imprint_matches: Option<bool>,
    pub imprint_digest: Option<String>,
    pub bound_signature_value_sha256: String,
    pub gen_time: Option<String>,
    pub accuracy_micros: Option<u64>,
    pub policy_oid: Option<String>,
    pub serial_number: Option<String>,
    pub nonce: Option<String>,
    pub ordering: Option<bool>,
    pub tsa_certificate: Option<CertificateInfo>,
    pub tsa_current_chain: ChainEvaluation,
    pub tsa_chain_at_gen_time: ChainEvaluation,
    pub tsa_revocation: RevocationEvaluation,
    #[serde(skip)]
    recheck_at: Option<Duration>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedTimeEvaluation {
    pub status: TrustedTimeStatus,
    pub message: String,
    pub timestamps: Vec<TimestampEvaluation>,
}

impl TrustedTimeEvaluation {
    pub(super) fn recheck_at(&self) -> Option<Duration> {
        self.timestamps
            .iter()
            .filter_map(|timestamp| timestamp.recheck_at)
            .min()
    }

    pub(super) fn not_checked() -> Self {
        Self {
            status: TrustedTimeStatus::NotChecked,
            message: "No supported signature-value-bound RFC3161 evidence has been checked. PDF /M and visible dates are unverified claims.".into(),
            timestamps: vec![],
        }
    }

    pub(super) fn from_timestamps(timestamps: Vec<TimestampEvaluation>) -> Self {
        if timestamps.is_empty() {
            return Self::not_checked();
        }
        let status = [
            TrustedTimeStatus::VerifiedCustomStore,
            TrustedTimeStatus::Revoked,
            TrustedTimeStatus::Invalid,
            TrustedTimeStatus::Unsupported,
            TrustedTimeStatus::Future,
            TrustedTimeStatus::Untrusted,
            TrustedTimeStatus::Incomplete,
        ]
        .into_iter()
        .find(|status| {
            timestamps
                .iter()
                .any(|timestamp| timestamp.status == *status)
        })
        .unwrap_or(TrustedTimeStatus::NotChecked);
        let message = if status == TrustedTimeStatus::VerifiedCustomStore {
            "At least one RFC3161 token authenticates this actual CMS signature value under your custom CA and complete current TSA CRL evidence. Its genTime is a TSA assertion, not the PDF's claimed signing time. This does not establish historical signer/TSA revocation, clear current signer revocation or confer qualified/PAdES-LTV status."
        } else {
            "No token meets the supported custom-anchor, exact-imprint, date and complete current TSA-revocation checks. Inspect timestamp cryptography and missing/failed evidence separately; no trusted-time assurance is inferred."
        };
        Self {
            status,
            message: message.into(),
            timestamps,
        }
    }
}

pub(super) struct TimestampToken {
    pub fingerprint: String,
    pub der_bytes: usize,
    pub details: crypto::CmsDetails,
    pub gen_time_label: String,
    gen_time: Duration,
    gen_time_seconds: i64,
    accuracy_micros: Option<u64>,
    policy: String,
    serial: String,
    nonce: Option<String>,
    ordering: bool,
    imprint_digest: String,
    imprint: Vec<u8>,
}

fn timestamp_error(_: impl std::fmt::Debug) -> CheckError {
    CheckError::invalid(
        "The RFC3161 response, token or TSTInfo is not a supported exact DER structure.",
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn token_der(bytes: &[u8]) -> CheckResult<Vec<u8>> {
    if bytes.is_empty() || bytes.len() > MAX_TIMESTAMP_BYTES {
        return Err(CheckError::unsupported(
            "An RFC3161 token/response is limited to 256 KiB.",
        ));
    }
    if let Ok(token) = ContentInfo::from_der(bytes) {
        let encoded = token.to_der().map_err(timestamp_error)?;
        if encoded != bytes {
            return Err(CheckError::invalid(
                "The timestamp token is not canonical, exact DER.",
            ));
        }
        return Ok(encoded);
    }
    let response = TimeStampResp::from_der(bytes).map_err(timestamp_error)?;
    if response.to_der().map_err(timestamp_error)? != bytes
        || !matches!(response.status.status, 0 | 1)
        || response.status.fail_info.is_some()
        || response.status.status_string.as_ref().is_some_and(|text| {
            text.len() > 8 || text.iter().map(String::len).sum::<usize>() > 1024
        })
    {
        return Err(CheckError::invalid(
            "The timestamp response is noncanonical, unsuccessful or contains contradictory failure information.",
        ));
    }
    response
        .time_stamp_token
        .ok_or_else(|| {
            CheckError::invalid("A granted RFC3161 response must contain its timestamp token.")
        })?
        .to_der()
        .map_err(timestamp_error)
}

fn generalized_time(value: &Any) -> CheckResult<(Duration, String)> {
    let bytes = value.value();
    if value.tag() != Tag::GeneralizedTime
        || !(15..=25).contains(&bytes.len())
        || bytes.last() != Some(&b'Z')
        || !bytes[..14].iter().all(u8::is_ascii_digit)
    {
        return Err(CheckError::unsupported(
            "Timestamp genTime must be UTC GeneralizedTime with seconds and at most nine fractional digits.",
        ));
    }
    let fraction = if bytes.len() == 15 {
        &[][..]
    } else {
        let fraction = &bytes[15..bytes.len() - 1];
        if bytes[14] != b'.'
            || fraction.is_empty()
            || !fraction.iter().all(u8::is_ascii_digit)
            || fraction.last() == Some(&b'0')
        {
            return Err(CheckError::invalid(
                "Timestamp fractional genTime is not canonical DER.",
            ));
        }
        fraction
    };
    let number = |start: usize, end: usize| {
        bytes[start..end]
            .iter()
            .fold(0u16, |value, byte| value * 10 + u16::from(byte - b'0'))
    };
    let date = der::DateTime::new(
        number(0, 4),
        number(4, 6) as u8,
        number(6, 8) as u8,
        number(8, 10) as u8,
        number(10, 12) as u8,
        number(12, 14) as u8,
    )
    .map_err(timestamp_error)?;
    let nanos = fraction
        .iter()
        .fold(0u32, |value, byte| value * 10 + u32::from(byte - b'0'))
        * 10u32.pow(9 - fraction.len() as u32);
    let time = date.unix_duration() + Duration::from_nanos(u64::from(nanos));
    let mut label = date.to_string();
    if !fraction.is_empty() {
        label.pop();
        label.push('.');
        label.push_str(std::str::from_utf8(fraction).map_err(timestamp_error)?);
        label.push('Z');
    }
    Ok((time, label))
}

fn accuracy_micros(accuracy: Option<&Accuracy>) -> CheckResult<Option<u64>> {
    let Some(accuracy) = accuracy else {
        return Ok(None);
    };
    if accuracy
        .millis
        .is_some_and(|value| !(1..=999).contains(&value))
        || accuracy
            .micros
            .is_some_and(|value| !(1..=999).contains(&value))
        || accuracy.seconds.is_some_and(|value| value > 86_400)
    {
        return Err(CheckError::unsupported(
            "Timestamp accuracy supports at most one day, with milliseconds/microseconds in 1..999.",
        ));
    }
    Ok(Some(
        accuracy.seconds.unwrap_or(0) * 1_000_000
            + u64::from(accuracy.millis.unwrap_or(0)) * 1000
            + u64::from(accuracy.micros.unwrap_or(0)),
    ))
}

fn check_ess_binding(details: &crypto::CmsDetails) -> CheckResult<()> {
    let encoded = details.signing_certificate_v2.as_deref().ok_or_else(|| {
        CheckError::unsupported("The TSA signingCertificateV2 binding is missing.")
    })?;
    let binding = SigningCertificateV2::from_der(encoded).map_err(timestamp_error)?;
    if binding.to_der().map_err(timestamp_error)? != encoded {
        return Err(CheckError::invalid(
            "The signed ESSCertIDv2 binding is not canonical DER.",
        ));
    }
    if binding.certs.is_empty() || binding.certs.len() > 16 || binding.policies.is_some() {
        return Err(CheckError::unsupported(
            "TSA signingCertificateV2 supports 1..16 exact certificate IDs without ESS policy extensions.",
        ));
    }
    let mut seen = HashSet::new();
    for (index, id) in binding.certs.iter().enumerate() {
        let algorithm = &id.hash_algorithm;
        if algorithm
            .parameters
            .as_ref()
            .is_some_and(|value| !value.is_null())
        {
            return Err(CheckError::unsupported(
                "ESS SHA-2 hash parameters must be absent or NULL.",
            ));
        }
        let digest = crypto::digest_name(algorithm.oid)?;
        let mut matches = Vec::new();
        for certificate in std::iter::once(&details.signer).chain(details.embedded.iter()) {
            let encoded = certificate.to_der().map_err(timestamp_error)?;
            let computed =
                hash(crypto::message_digest(digest)?, &encoded).map_err(timestamp_error)?;
            if !openssl::memcmp::eq(&computed, id.cert_hash.as_bytes()) {
                continue;
            }
            let parsed = parse_certificate(certificate)?;
            if id.issuer_serial.as_ref().is_some_and(|issuer| {
                issuer.serial_number != parsed.tbs_certificate.serial_number || issuer.issuer.len() != 1
                    || !matches!(&issuer.issuer[0], GeneralName::DirectoryName(name) if name == &parsed.tbs_certificate.issuer)
            }) { continue; }
            matches.push(crypto::fingerprint(&encoded));
        }
        if matches.len() != 1
            || index == 0 && matches[0] != details.certificate.sha256_fingerprint
            || !seen.insert(matches[0].clone())
        {
            return Err(CheckError::invalid(
                "The signed ESSCertIDv2 hash/issuer/serial does not uniquely bind the actual TSA certificate and supplied chain.",
            ));
        }
    }
    Ok(())
}

impl TimestampToken {
    pub(super) fn next_check_at(&self, now: Duration, deadline: Duration) -> CheckResult<Duration> {
        let mut deadline = super::evidence::certificate_deadline(
            std::iter::once(&self.details.signer).chain(self.details.embedded.iter()),
            now,
            deadline,
        )?;
        let date = self
            .gen_time
            .checked_add(Duration::from_micros(self.accuracy_micros.unwrap_or(0)))
            .ok_or_else(|| {
                CheckError::invalid("The timestamp accuracy overflows its date range.")
            })?;
        if date > now {
            deadline = deadline.min(date);
        }
        Ok(deadline)
    }

    pub(super) fn parse(bytes: &[u8]) -> CheckResult<Self> {
        let der = token_der(bytes)?;
        let details = crypto::timestamp_details(&der)?;
        strong_certificate(&details.signer)?;
        timestamp_purpose(&details.signer, true)?;
        let content = details
            .content
            .as_deref()
            .ok_or_else(|| CheckError::invalid("The token TSTInfo is missing."))?;
        if content.len() > 16 * 1024 {
            return Err(CheckError::unsupported("The token TSTInfo exceeds 16 KiB."));
        }
        let info = TstInfo::from_der(content).map_err(timestamp_error)?;
        if info.to_der().map_err(timestamp_error)? != content
            || info.version != 1
            || info.serial_number.as_bytes().len() > 20
            || info.serial_number.as_bytes().iter().all(|byte| *byte == 0)
            || info
                .nonce
                .as_ref()
                .is_some_and(|nonce| nonce.as_bytes().len() > 32)
        {
            return Err(CheckError::invalid(
                "TSTInfo must be canonical version 1, with a positive serial up to 160 bits and nonce up to 256 bits.",
            ));
        }
        if info.extensions.is_some() {
            return Err(CheckError::unsupported(
                "TSTInfo extensions are outside the supported timestamp subset.",
            ));
        }
        let parsed = parse_certificate(&details.signer)?;
        if let Some(name) = &info.tsa {
            let subject_matches = matches!(name, GeneralName::DirectoryName(name) if name == &parsed.tbs_certificate.subject);
            let alternative_matches = parsed
                .tbs_certificate
                .get::<SubjectAltName>()
                .map_err(timestamp_error)?
                .is_some_and(|(_, names)| names.0.contains(name));
            if !subject_matches && !alternative_matches {
                return Err(CheckError::invalid(
                    "The token TSA name does not match the actual signer certificate subject/SAN.",
                ));
            }
        }
        let imprint_digest = crypto::digest_name(info.message_imprint.hash_algorithm.oid)?;
        if info
            .message_imprint
            .hash_algorithm
            .parameters
            .as_ref()
            .is_some_and(|value| !value.is_null())
            || info.message_imprint.hashed_message.as_bytes().len()
                != crypto::message_digest(imprint_digest)?.size()
        {
            return Err(CheckError::invalid(
                "The RFC3161 message-imprint algorithm, parameters or exact digest length are invalid.",
            ));
        }
        let (gen_time, gen_time_label) = generalized_time(&info.gen_time)?;
        let gen_time_seconds = i64::try_from(gen_time.as_secs()).map_err(timestamp_error)?;
        let accuracy_micros = accuracy_micros(info.accuracy.as_ref())?;
        check_ess_binding(&details)?;
        crypto::verify_timestamp(&der, &details)?;
        Ok(Self {
            fingerprint: crypto::fingerprint(&der),
            der_bytes: der.len(),
            details,
            gen_time,
            gen_time_seconds,
            gen_time_label,
            accuracy_micros,
            policy: info.policy.to_string(),
            serial: hex(info.serial_number.as_bytes()),
            nonce: info.nonce.as_ref().map(|nonce| hex(nonce.as_bytes())),
            ordering: info.ordering,
            imprint_digest: imprint_digest.into(),
            imprint: info.message_imprint.hashed_message.as_bytes().to_vec(),
        })
    }

    pub(super) fn check_imprint(&self, binding: &SignatureBinding) -> CheckResult<()> {
        let digest = hash(
            crypto::message_digest(&self.imprint_digest)?,
            &binding.signature_value,
        )
        .map_err(timestamp_error)?;
        if !openssl::memcmp::eq(&digest, &self.imprint) {
            return Err(CheckError::invalid(
                "The RFC3161 message imprint does not match the selected PDF signature's actual CMS signature value. A PDF date, certificate or document hash is not a substitute.",
            ));
        }
        Ok(())
    }

    pub(super) fn evaluate(
        &self,
        binding: &SignatureBinding,
        trust: &TrustStore,
        source: &'static str,
    ) -> TimestampEvaluation {
        let current = trust.timestamp_chain(&self.details.signer, &self.details.embedded, None);
        let at_gen_time = trust.timestamp_chain(
            &self.details.signer,
            &self.details.embedded,
            Some(self.gen_time_seconds),
        );
        let tsa_revocation = trust.evidence.revocation(&current);
        let check = self.check_imprint(binding);
        let mut result = TimestampEvaluation {
            status: TrustedTimeStatus::Incomplete,
            message: String::new(),
            source,
            evidence_fingerprint: self.fingerprint.clone(),
            cryptographic_integrity: Integrity::Intact,
            imprint_matches: Some(check.is_ok()),
            imprint_digest: Some(self.imprint_digest.clone()),
            bound_signature_value_sha256: binding.signature_value_sha256.clone(),
            gen_time: Some(self.gen_time_label.clone()),
            accuracy_micros: self.accuracy_micros,
            policy_oid: Some(self.policy.clone()),
            serial_number: Some(self.serial.clone()),
            nonce: self.nonce.clone(),
            ordering: Some(self.ordering),
            tsa_certificate: Some(self.details.certificate.clone()),
            tsa_current_chain: current.evaluation,
            tsa_chain_at_gen_time: at_gen_time.evaluation,
            tsa_revocation,
            recheck_at: None,
        };
        match now()
            .and_then(|now| self.next_check_at(now, super::evidence::inspection_deadline(now)?))
        {
            Ok(deadline) => result.recheck_at = Some(deadline),
            Err(error) => {
                result.status = TrustedTimeStatus::Invalid;
                result.message = error.message;
                return result;
            }
        }
        if let Err(error) = check {
            result.status = TrustedTimeStatus::Invalid;
            result.message = error.message;
            return result;
        }
        let bounds = self.date_bounds(&at_gen_time.certificates);
        if let Err((status, message)) = bounds {
            result.status = status;
            result.message = message;
            return result;
        }
        if result.tsa_current_chain.status != "anchored_custom_store"
            || result.tsa_chain_at_gen_time.status != "anchored_custom_store"
        {
            result.status = TrustedTimeStatus::Untrusted;
            result.message = "The TSA CMS and exact signature-value imprint verify, but no acceptable custom-CA TSA path is valid both now and at genTime. Currently expired/future TSA chains are not accepted as historical trust.".into();
        } else if result.tsa_revocation.status == RevocationStatus::Revoked {
            result.status = TrustedTimeStatus::Revoked;
            result.message = "The token cryptography and imprint verify, but a current imported CRL revokes a certificate in the TSA path. No trusted time is inferred.".into();
        } else if result.tsa_revocation.status != RevocationStatus::GoodCurrentEvidence {
            result.status = TrustedTimeStatus::Incomplete;
            result.message = "The TSA CMS, ESS certificate binding, exact signature-value imprint and custom paths verify, but complete current TSA-chain CRL evidence is missing or failed. Historical revocation and the TSA policy/clock are not independently established.".into();
        } else {
            result.status = TrustedTimeStatus::VerifiedCustomStore;
            result.message = "The token authenticates this actual CMS signature value, with TSA paths valid now/at genTime and complete current imported TSA-chain CRLs. genTime/accuracy are signed TSA assertions; its policy/clock and historical revocation were not independently audited. This does not change the PDF signer's current trust or revocation.".into();
        }
        result
    }

    fn date_bounds(
        &self,
        chain: &[openssl::x509::X509],
    ) -> std::result::Result<(), (TrustedTimeStatus, String)> {
        let check = || -> CheckResult<bool> {
            let now = now()?;
            let accuracy = Duration::from_micros(self.accuracy_micros.unwrap_or(0));
            let upper = self.gen_time.checked_add(accuracy).ok_or_else(|| {
                CheckError::invalid("The timestamp accuracy overflows its date range.")
            })?;
            let lower = self.gen_time.checked_sub(accuracy).ok_or_else(|| {
                CheckError::invalid("The timestamp accuracy precedes the supported epoch.")
            })?;
            if upper > now {
                return Ok(false);
            }
            for certificate in chain {
                let parsed = parse_certificate(certificate)?;
                if lower
                    < parsed
                        .tbs_certificate
                        .validity
                        .not_before
                        .to_unix_duration()
                    || upper > parsed.tbs_certificate.validity.not_after.to_unix_duration()
                {
                    return Err(CheckError::invalid(
                        "The token genTime/accuracy interval is outside a TSA path certificate's validity dates.",
                    ));
                }
            }
            Ok(true)
        };
        match check() {
            Ok(true) => Ok(()),
            Ok(false) => Err((TrustedTimeStatus::Future, "The token genTime or its signed accuracy upper bound is in the future. It is not verified trusted time.".into())),
            Err(error) => Err((TrustedTimeStatus::Invalid, error.message)),
        }
    }
}

pub(super) fn evaluate_embedded(
    bytes: &[u8],
    binding: &SignatureBinding,
    trust: &TrustStore,
) -> TimestampEvaluation {
    match TimestampToken::parse(bytes) {
        Ok(token) => token.evaluate(binding, trust, "embedded"),
        Err(error) => TimestampEvaluation {
            status: if error.status == Integrity::Unsupported {
                TrustedTimeStatus::Unsupported
            } else {
                TrustedTimeStatus::Invalid
            },
            message: error.message,
            source: "embedded",
            evidence_fingerprint: crypto::fingerprint(bytes),
            cryptographic_integrity: error.status,
            imprint_matches: None,
            imprint_digest: None,
            bound_signature_value_sha256: binding.signature_value_sha256.clone(),
            gen_time: None,
            accuracy_micros: None,
            policy_oid: None,
            serial_number: None,
            nonce: None,
            ordering: None,
            tsa_certificate: None,
            tsa_current_chain: ChainEvaluation::not_checked(),
            tsa_chain_at_gen_time: ChainEvaluation::not_checked(),
            tsa_revocation: RevocationEvaluation::not_checked(
                "The token did not pass the supported RFC3161 structure/cryptographic checks.",
            ),
            recheck_at: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_generalized_time_preserves_canonical_fractional_precision_and_refuses_ambiguous_dates()
     {
        for value in [
            "20261101123045Z",
            "20261101123045.123456789Z",
            "20261101123045.01Z",
        ] {
            let encoded = Any::new(Tag::GeneralizedTime, value.as_bytes()).unwrap();
            let (time, label) = generalized_time(&encoded).unwrap();
            assert!(label.starts_with("2026-11-01T12:30:45"));
            if value.contains(".123") {
                assert_eq!(time.subsec_nanos(), 123_456_789);
            }
        }
        for value in [
            "20261101123045.0Z",
            "20261101123045.120Z",
            "20261101123045.Z",
            "20261101123045.1234567891Z",
            "20261301123045Z",
            "20261101123045+0100",
            "20261101123045,1Z",
            "202611011230Z",
        ] {
            assert!(
                generalized_time(&Any::new(Tag::GeneralizedTime, value.as_bytes()).unwrap())
                    .is_err(),
                "{value}"
            );
        }
        assert!(
            generalized_time(&Any::new(Tag::OctetString, b"20261101123045Z".as_slice()).unwrap())
                .is_err()
        );
    }

    #[test]
    fn timestamp_accuracy_requires_rfc3161_component_ranges_and_bounded_seconds() {
        assert_eq!(accuracy_micros(None).unwrap(), None);
        assert_eq!(
            accuracy_micros(Some(&Accuracy {
                seconds: Some(1),
                millis: Some(2),
                micros: Some(3)
            }))
            .unwrap(),
            Some(1_002_003)
        );
        for value in [
            Accuracy {
                seconds: Some(86_401),
                millis: None,
                micros: None,
            },
            Accuracy {
                seconds: None,
                millis: Some(0),
                micros: None,
            },
            Accuracy {
                seconds: None,
                millis: None,
                micros: Some(1000),
            },
        ] {
            assert!(accuracy_micros(Some(&value)).is_err());
        }
    }
}
