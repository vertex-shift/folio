use std::{collections::BTreeMap, io::Read, path::Path};

use der::Decode;
use openssl::{
    stack::Stack,
    x509::{
        X509, X509PurposeId, X509StoreContext,
        store::X509StoreBuilder,
        verify::{X509VerifyFlags, X509VerifyParam},
    },
};
use serde::Serialize;
use x509_cert::{
    Certificate,
    ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage},
};
use zeroize::Zeroizing;

use super::{CertificateInfo, CheckError, CheckResult, crypto};
use crate::document::{Result, invalid};

pub const MAX_ANCHOR_BYTES: usize = 64 * 1024;
pub const MAX_TRUST_ANCHORS: usize = 32;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustStoreInfo {
    pub revision: u64,
    pub anchors: Vec<CertificateInfo>,
    pub persistent: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainEvaluation {
    pub status: &'static str,
    pub message: String,
    pub anchor_fingerprint: Option<String>,
    pub chain_fingerprints: Vec<String>,
}

impl ChainEvaluation {
    pub(super) fn not_checked() -> Self {
        Self {
            status: "not_checked",
            message: "Certificate-chain trust has not been checked. Only explicitly loaded custom CA anchors can be used.".into(),
            anchor_fingerprint: None,
            chain_fingerprints: vec![],
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            status: "untrusted",
            message: message.into(),
            anchor_fingerprint: None,
            chain_fingerprints: vec![],
        }
    }
}

#[derive(Default)]
pub struct TrustStore {
    revision: u64,
    anchors: BTreeMap<String, X509>,
}

fn trust_error(_: openssl::error::ErrorStack) -> CheckError {
    CheckError::invalid("The local custom certificate store could not evaluate this chain.")
}

fn parse_certificate(certificate: &X509) -> CheckResult<Certificate> {
    Certificate::from_der(&certificate.to_der().map_err(trust_error)?).map_err(|_| {
        CheckError::invalid("The public certificate is not a supported DER certificate.")
    })
}

fn strong_certificate(certificate: &X509) -> CheckResult<()> {
    let key = certificate.public_key().map_err(trust_error)?;
    crypto::validate_key(&key)?;
    let parsed = parse_certificate(certificate)?;
    if !matches!(
        parsed.signature_algorithm.oid.to_string().as_str(),
        "1.2.840.113549.1.1.11"
            | "1.2.840.113549.1.1.12"
            | "1.2.840.113549.1.1.13"
            | "1.2.840.10045.4.3.2"
            | "1.2.840.10045.4.3.3"
            | "1.2.840.10045.4.3.4"
    ) {
        return Err(CheckError::unsupported(
            "Custom chains require SHA-256/384/512 RSA PKCS#1 or supported ECDSA certificate signatures; weak and other certificate algorithms are not accepted.",
        ));
    }
    if parsed.signature_algorithm != parsed.tbs_certificate.signature {
        return Err(CheckError::invalid(
            "The certificate's inner and outer signature algorithms disagree.",
        ));
    }
    Ok(())
}

pub(super) fn document_purpose(certificate: &X509, leaf: bool) -> CheckResult<()> {
    let parsed = parse_certificate(certificate)?;
    if leaf {
        if parsed
            .tbs_certificate
            .get::<BasicConstraints>()
            .map_err(|_| CheckError::invalid("The signer has malformed basic constraints."))?
            .is_some_and(|(_, constraints)| constraints.ca)
        {
            return Err(CheckError::invalid(
                "A CA certificate is not accepted as an end-entity PDF signing identity.",
            ));
        }
        crypto::validate_key_usage(&parsed)?;
    }
    if let Some((_, usage)) = parsed
        .tbs_certificate
        .get::<ExtendedKeyUsage>()
        .map_err(|_| CheckError::invalid("A chain certificate has malformed extended key usage."))?
        && !usage.0.iter().any(|oid| {
            matches!(
                oid.to_string().as_str(),
                "1.3.6.1.5.5.7.3.36" | "2.5.29.37.0"
            )
        })
    {
        return Err(CheckError::unsupported(
            "A certificate's extended key usage does not permit document signing. TLS, email, code-signing, and timestamp-only purposes are not PDF-signing authorization.",
        ));
    }
    Ok(())
}

fn public_anchor(bytes: &[u8]) -> CheckResult<X509> {
    if bytes.is_empty() || bytes.len() > MAX_ANCHOR_BYTES {
        return Err(CheckError::invalid(
            "Choose one nonempty public PEM/DER CA certificate of at most 64 KiB.",
        ));
    }
    let trimmed = bytes.trim_ascii();
    let certificate = if trimmed.starts_with(b"-----BEGIN") {
        let body = trimmed
            .strip_prefix(b"-----BEGIN CERTIFICATE-----")
            .and_then(|value| value.strip_suffix(b"-----END CERTIFICATE-----"))
            .filter(|value| {
                value.iter().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || b"+/=".contains(byte)
                        || byte.is_ascii_whitespace()
                })
            })
            .ok_or_else(|| {
                CheckError::invalid(
                    "Choose exactly one PUBLIC CERTIFICATE PEM block, without private keys, bundles, or other data.",
                )
            })?;
        if body.is_empty() {
            return Err(CheckError::invalid(
                "The public certificate PEM block is empty.",
            ));
        }
        X509::from_pem(trimmed).map_err(|_| {
            CheckError::invalid("The public certificate PEM block could not be decoded.")
        })?
    } else {
        // Decode the entire input first: OpenSSL's d2i parser alone permits trailing data.
        Certificate::from_der(bytes).map_err(|_| {
            CheckError::invalid(
                "Choose exactly one public X.509 DER certificate, not private-key or PKCS#12 material.",
            )
        })?;
        X509::from_der(bytes).map_err(trust_error)?
    };
    strong_certificate(&certificate)?;
    let parsed = parse_certificate(&certificate)?;
    if !parsed
        .tbs_certificate
        .get::<BasicConstraints>()
        .map_err(|_| CheckError::invalid("The CA basic constraints are malformed."))?
        .is_some_and(|(critical, constraints)| critical && constraints.ca)
        || !parsed
            .tbs_certificate
            .get::<KeyUsage>()
            .map_err(|_| CheckError::invalid("The CA key usage is malformed."))?
            .is_some_and(|(_, usage)| usage.key_cert_sign())
    {
        return Err(CheckError::invalid(
            "A custom anchor must be a CA certificate with critical CA basic constraints and certificate-signing key usage. A signer/leaf certificate cannot be an anchor.",
        ));
    }
    if !crypto::certificate_info(&certificate)?.self_signed {
        return Err(CheckError::invalid(
            "Choose a self-signed root CA certificate as the custom anchor. Intermediate certificates are taken only from the signature's embedded chain.",
        ));
    }
    // Also reject an explicitly restricted CA before it enters the store.
    document_purpose(&certificate, false)?;
    Ok(certificate)
}

impl TrustStore {
    pub fn info(&self) -> Result<TrustStoreInfo> {
        Ok(TrustStoreInfo {
            revision: self.revision,
            anchors: self
                .anchors
                .values()
                .map(crypto::certificate_info)
                .collect::<CheckResult<_>>()?,
            persistent: false,
        })
    }

    fn check_revision(&self, revision: u64) -> Result<()> {
        if revision != self.revision {
            return Err(invalid(
                "The custom CA store changed. Refresh signature checks before changing its anchors.",
            ));
        }
        Ok(())
    }

    pub fn add(&mut self, revision: u64, bytes: &[u8]) -> Result<TrustStoreInfo> {
        self.check_revision(revision)?;
        if self.anchors.len() >= MAX_TRUST_ANCHORS {
            return Err(invalid("The custom CA store is limited to 32 anchors."));
        }
        let certificate = public_anchor(bytes)?;
        let info = crypto::certificate_info(&certificate)?;
        if self.anchors.contains_key(&info.sha256_fingerprint) {
            return Err(invalid(
                "That public CA certificate is already a custom anchor.",
            ));
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("The custom CA store revision limit was reached."))?;
        let fingerprint = info.sha256_fingerprint.clone();
        let mut store = self.info()?;
        store.revision = next_revision;
        store.anchors.push(info);
        store
            .anchors
            .sort_by(|a, b| a.sha256_fingerprint.cmp(&b.sha256_fingerprint));
        self.anchors.insert(fingerprint, certificate);
        self.revision = next_revision;
        Ok(store)
    }

    pub fn remove(&mut self, revision: u64, fingerprint: &str) -> Result<TrustStoreInfo> {
        self.check_revision(revision)?;
        if !self.anchors.contains_key(fingerprint) {
            return Err(invalid("That custom CA anchor is no longer loaded."));
        }
        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("The custom CA store revision limit was reached."))?;
        let mut store = self.info()?;
        store.revision = next_revision;
        store
            .anchors
            .retain(|anchor| anchor.sha256_fingerprint != fingerprint);
        self.anchors.remove(fingerprint);
        self.revision = next_revision;
        Ok(store)
    }

    pub(super) fn evaluate(&self, signer: &X509, embedded: &[X509]) -> ChainEvaluation {
        if self.anchors.is_empty() {
            return ChainEvaluation::not_checked();
        }
        match self.evaluate_chain(signer, embedded) {
            Ok(evaluation) => evaluation,
            Err(error) => ChainEvaluation::failed(error.message),
        }
    }

    fn evaluate_chain(&self, signer: &X509, embedded: &[X509]) -> CheckResult<ChainEvaluation> {
        let mut store = X509StoreBuilder::new().map_err(trust_error)?;
        for anchor in self.anchors.values() {
            store.add_cert(anchor.clone()).map_err(trust_error)?;
        }
        let mut parameters = X509VerifyParam::new().map_err(trust_error)?;
        parameters.set_depth(16);
        parameters.set_auth_level(2);
        parameters
            .set_flags(
                X509VerifyFlags::X509_STRICT
                    | X509VerifyFlags::TRUSTED_FIRST
                    | X509VerifyFlags::CHECK_SS_SIGNATURE,
            )
            .map_err(trust_error)?;
        // OpenSSL has no PDF purpose. ANY avoids imposing email/TLS semantics;
        // document EKU and leaf signing usage are checked explicitly below.
        parameters
            .set_purpose(X509PurposeId::ANY)
            .map_err(trust_error)?;
        store.set_param(&parameters).map_err(trust_error)?;
        let store = store.build();
        let mut intermediates = Stack::new().map_err(trust_error)?;
        for certificate in embedded {
            intermediates
                .push(certificate.clone())
                .map_err(trust_error)?;
        }
        let mut context = X509StoreContext::new().map_err(trust_error)?;
        let (valid, error, depth, chain) = context
            .init(&store, signer, &intermediates, |context| {
                let valid = context.verify_cert()?;
                Ok((
                    valid,
                    context.error().error_string(),
                    context.error_depth(),
                    context
                        .chain()
                        .map(|chain| chain.iter().map(ToOwned::to_owned).collect::<Vec<_>>())
                        .unwrap_or_default(),
                ))
            })
            .map_err(trust_error)?;
        if !valid {
            return Ok(ChainEvaluation::failed(format!(
                "No valid current-date path to a custom CA anchor: {error} (chain depth {depth}). Revocation and trusted time were not checked."
            )));
        }
        let mut fingerprints = Vec::new();
        for (index, certificate) in chain.iter().enumerate() {
            strong_certificate(certificate)?;
            document_purpose(certificate, index == 0)?;
            fingerprints.push(crypto::certificate_info(certificate)?.sha256_fingerprint);
        }
        let anchor = fingerprints
            .last()
            .filter(|fingerprint| self.anchors.contains_key(*fingerprint))
            .ok_or_else(|| {
                CheckError::invalid(
                    "The verified path did not terminate at a loaded custom anchor.",
                )
            })?
            .clone();
        Ok(ChainEvaluation {
            status: "anchored_custom_store",
            message: "The certificate chain is valid now and anchored in your custom CA store, not Adobe AATL/EUTL or OS trust. This does not establish revocation status, trusted signing time, or PDF content integrity.".into(),
            anchor_fingerprint: Some(anchor),
            chain_fingerprints: fingerprints,
        })
    }
}

pub fn read_anchor_file(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    if !path.extension().is_some_and(|extension| {
        ["pem", "der", "cer", "crt"]
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
    }) {
        return Err(invalid(
            "Choose a public .pem, .der, .cer, or .crt CA certificate.",
        ));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_ANCHOR_BYTES as u64 {
        return Err(invalid("Choose a public CA certificate of at most 64 KiB."));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_ANCHOR_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > MAX_ANCHOR_BYTES {
        return Err(invalid(
            "Choose a nonempty public CA certificate of at most 64 KiB.",
        ));
    }
    Ok(bytes)
}
