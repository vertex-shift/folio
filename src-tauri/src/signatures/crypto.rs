use std::{cmp::Ordering, collections::HashSet};

use cms::{
    cert::CertificateChoices,
    content_info::{CmsVersion, ContentInfo},
    signed_data::{SignedData, SignerIdentifier},
};
use der::{Decode, Encode, asn1::ObjectIdentifier};
use openssl::{
    asn1::Asn1Time,
    cms::{CMSOptions, CmsContentInfo},
    hash::MessageDigest,
    nid::Nid,
    pkcs12::Pkcs12,
    pkey::{Id, PKey, Private},
    stack::Stack,
    x509::{X509, X509NameRef},
};
use x509_cert::{
    Certificate,
    ext::pkix::{KeyUsage, SubjectKeyIdentifier},
};

use super::{CertificateInfo, CertificateValidity, CheckError, CheckResult, MAX_CMS_BYTES};

const DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");

pub(super) struct CmsDetails {
    pub digest: String,
    pub algorithm: String,
    pub certificate: CertificateInfo,
    signer: X509,
}

pub(super) struct Identity {
    key: PKey<Private>,
    certificate: X509,
    chain: Option<Stack<X509>>,
}

fn crypto_error(_: openssl::error::ErrorStack) -> CheckError {
    CheckError::invalid("The certificate or cryptographic structure could not be processed.")
}

fn der_error(_: der::Error) -> CheckError {
    CheckError::invalid(
        "The CMS signature or certificate is not a supported, well-formed DER structure.",
    )
}

fn digest_name(oid: ObjectIdentifier) -> CheckResult<&'static str> {
    match oid.to_string().as_str() {
        "2.16.840.1.101.3.4.2.1" => Ok("SHA-256"),
        "2.16.840.1.101.3.4.2.2" => Ok("SHA-384"),
        "2.16.840.1.101.3.4.2.3" => Ok("SHA-512"),
        _ => Err(CheckError::unsupported(
            "Only SHA-256, SHA-384, and SHA-512 CMS digests are supported. Weak and unknown algorithms are not accepted.",
        )),
    }
}

fn validate_key(key: &openssl::pkey::PKeyRef<openssl::pkey::Public>) -> CheckResult<&'static str> {
    match key.id() {
        Id::RSA if key.bits() >= 2048 && key.bits() <= 8192 => Ok("RSA"),
        Id::EC => {
            let ec = key.ec_key().map_err(crypto_error)?;
            match ec.group().curve_name() {
                Some(Nid::X9_62_PRIME256V1 | Nid::SECP384R1 | Nid::SECP521R1) => Ok("ECDSA"),
                _ => Err(CheckError::unsupported(
                    "Only NIST P-256, P-384, and P-521 EC certificates are supported.",
                )),
            }
        }
        _ => Err(CheckError::unsupported(
            "Only RSA 2048-8192-bit and NIST P-256/P-384/P-521 ECDSA signing keys are supported.",
        )),
    }
}

fn name(name: &X509NameRef) -> CheckResult<String> {
    name.entries()
        .map(|entry| {
            let label = entry.object().nid().short_name().unwrap_or("OID");
            let value = entry.data().to_string().map_err(crypto_error)?;
            Ok(format!("{label}={value}"))
        })
        .collect::<CheckResult<Vec<_>>>()
        .map(|entries| entries.join(", "))
}

pub(super) fn certificate_info(cert: &X509) -> CheckResult<CertificateInfo> {
    let now = Asn1Time::days_from_now(0).map_err(crypto_error)?;
    let validity = if cert.not_before().compare(&now).map_err(crypto_error)? == Ordering::Greater {
        CertificateValidity::NotYetValid
    } else if cert.not_after().compare(&now).map_err(crypto_error)? == Ordering::Less {
        CertificateValidity::Expired
    } else {
        CertificateValidity::ValidNow
    };
    let public_key = cert.public_key().map_err(crypto_error)?;
    let self_signed = cert.subject_name().to_der().map_err(crypto_error)?
        == cert.issuer_name().to_der().map_err(crypto_error)?
        && cert.verify(&public_key).map_err(crypto_error)?;
    Ok(CertificateInfo {
        subject: name(cert.subject_name())?,
        issuer: name(cert.issuer_name())?,
        serial: cert
            .serial_number()
            .to_bn()
            .map_err(crypto_error)?
            .to_hex_str()
            .map_err(crypto_error)?
            .to_string(),
        sha256_fingerprint: cert
            .digest(MessageDigest::sha256())
            .map_err(crypto_error)?
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
        not_before: cert.not_before().to_string(),
        not_after: cert.not_after().to_string(),
        validity,
        self_signed,
        identity_trust: "not_checked",
    })
}

fn validate_key_usage(certificate: &Certificate) -> CheckResult<()> {
    if let Some((_, usage)) = certificate
        .tbs_certificate
        .get::<KeyUsage>()
        .map_err(der_error)?
        && !usage.digital_signature()
        && !usage.non_repudiation()
    {
        return Err(CheckError::unsupported(
            "The certificate's key-usage extension does not allow digital signatures.",
        ));
    }
    Ok(())
}

pub(super) fn cms_der(padded: &[u8]) -> CheckResult<&[u8]> {
    if padded.len() < 2 || padded[0] != 0x30 {
        return Err(CheckError::invalid(
            "Signature Contents does not contain a DER CMS sequence.",
        ));
    }
    let (header, length) = if padded[1] < 0x80 {
        (2, padded[1] as usize)
    } else {
        let count = (padded[1] & 0x7f) as usize;
        if count == 0 || count > 4 || padded.len() < count + 2 || padded[2] == 0 {
            return Err(CheckError::invalid(
                "The CMS length encoding is invalid or indefinite.",
            ));
        }
        let length = padded[2..count + 2]
            .iter()
            .fold(0usize, |value, byte| (value << 8) | *byte as usize);
        if length < 128 {
            return Err(CheckError::invalid(
                "The CMS length encoding is not canonical DER.",
            ));
        }
        (count + 2, length)
    };
    let end = header
        .checked_add(length)
        .filter(|end| *end <= padded.len() && *end <= MAX_CMS_BYTES)
        .ok_or_else(|| CheckError::invalid("CMS extends past the signature Contents bounds."))?;
    if padded[end..].iter().any(|byte| *byte != 0) {
        return Err(CheckError::invalid(
            "Nonzero data follows CMS inside the unsigned Contents gap.",
        ));
    }
    Ok(&padded[..end])
}

pub(super) fn cms_details(der: &[u8]) -> CheckResult<CmsDetails> {
    let info = ContentInfo::from_der(der).map_err(der_error)?;
    if info.content_type != SIGNED_DATA {
        return Err(CheckError::unsupported("Contents is not CMS SignedData."));
    }
    let signed: SignedData = info.content.decode_as().map_err(der_error)?;
    if signed.encap_content_info.econtent_type != DATA
        || signed.encap_content_info.econtent.is_some()
    {
        return Err(CheckError::unsupported(
            "Only detached CMS data signatures are supported.",
        ));
    }
    if signed.signer_infos.0.len() != 1 || signed.digest_algorithms.len() != 1 {
        return Err(CheckError::unsupported(
            "Each PDF signature must contain exactly one CMS signer and digest algorithm.",
        ));
    }
    let signer = signed
        .signer_infos
        .0
        .iter()
        .next()
        .expect("one signer checked");
    let version = match signer.sid {
        SignerIdentifier::IssuerAndSerialNumber(_) => CmsVersion::V1,
        SignerIdentifier::SubjectKeyIdentifier(_) => CmsVersion::V3,
    };
    if signer.version != version || signed.version != version {
        return Err(CheckError::invalid(
            "CMS version declarations do not match the supported signer structure.",
        ));
    }
    if signed.crls.is_some() {
        return Err(CheckError::unsupported(
            "Embedded CMS revocation lists are not evaluated in this supported subset.",
        ));
    }
    let digest = digest_name(signer.digest_alg.oid)?;
    if signer
        .digest_alg
        .parameters
        .as_ref()
        .is_some_and(|value| !value.is_null())
    {
        return Err(CheckError::unsupported(
            "CMS SHA-2 digest parameters must be absent or NULL.",
        ));
    }
    let declared_digest = signed
        .digest_algorithms
        .iter()
        .next()
        .expect("one digest checked");
    if declared_digest.oid != signer.digest_alg.oid {
        return Err(CheckError::invalid(
            "CMS digest algorithm declarations do not agree.",
        ));
    }
    if declared_digest
        .parameters
        .as_ref()
        .is_some_and(|value| !value.is_null())
    {
        return Err(CheckError::unsupported(
            "CMS SHA-2 digest parameters must be absent or NULL.",
        ));
    }
    if signer
        .unsigned_attrs
        .as_ref()
        .is_some_and(|attributes| !attributes.is_empty())
    {
        return Err(CheckError::unsupported(
            "CMS timestamps, countersignatures, and other unsigned attributes are not evaluated in this supported subset.",
        ));
    }
    if let Some(attributes) = &signer.signed_attrs {
        let mut seen = HashSet::new();
        for attribute in attributes.iter() {
            if !seen.insert(attribute.oid) {
                return Err(CheckError::invalid("CMS has duplicate signed attributes."));
            }
            if attribute.oid == CONTENT_TYPE
                && (attribute.values.len() != 1
                    || attribute
                        .values
                        .iter()
                        .next()
                        .expect("one value checked")
                        .decode_as::<ObjectIdentifier>()
                        .map_err(der_error)?
                        != DATA)
            {
                return Err(CheckError::invalid(
                    "CMS content-type signed attribute is invalid.",
                ));
            }
            if attribute.oid == MESSAGE_DIGEST && attribute.values.len() != 1 {
                return Err(CheckError::invalid(
                    "CMS message-digest signed attribute is ambiguous.",
                ));
            }
        }
        if !seen.contains(&CONTENT_TYPE) || !seen.contains(&MESSAGE_DIGEST) {
            return Err(CheckError::invalid(
                "CMS signed attributes omit content-type or message-digest.",
            ));
        }
    }
    let certificates = signed
        .certificates
        .as_ref()
        .ok_or_else(|| CheckError::unsupported("The CMS signer certificate is not embedded."))?;
    if certificates.0.len() > 64 {
        return Err(CheckError::unsupported(
            "The CMS certificate set exceeds the 64-certificate limit.",
        ));
    }
    let mut matching = Vec::new();
    for choice in certificates.0.iter() {
        if let CertificateChoices::Certificate(certificate) = choice {
            let matches = match &signer.sid {
                SignerIdentifier::IssuerAndSerialNumber(id) => {
                    certificate.tbs_certificate.issuer == id.issuer
                        && certificate.tbs_certificate.serial_number == id.serial_number
                }
                SignerIdentifier::SubjectKeyIdentifier(id) => certificate
                    .tbs_certificate
                    .get::<SubjectKeyIdentifier>()
                    .map_err(der_error)?
                    .is_some_and(|(_, subject_id)| subject_id.0 == id.0),
            };
            if matches {
                matching.push(certificate);
            }
        } else {
            return Err(CheckError::unsupported(
                "Non-X.509 CMS certificate choices are not supported.",
            ));
        }
    }
    if matching.is_empty() {
        return Err(CheckError::unsupported(
            "The signer certificate is not embedded. External certificate lookup is not supported.",
        ));
    }
    if matching.len() > 1 {
        return Err(CheckError::invalid(
            "CMS has a missing or ambiguous signer certificate.",
        ));
    }
    let certificate = matching[0];
    validate_key_usage(certificate)?;
    let cert = X509::from_der(&certificate.to_der().map_err(der_error)?).map_err(crypto_error)?;
    let key = cert.public_key().map_err(crypto_error)?;
    let key_kind = validate_key(&key)?;
    let oid = signer.signature_algorithm.oid.to_string();
    let algorithm = match (key_kind, oid.as_str(), digest) {
        ("RSA", "1.2.840.113549.1.1.1", _) => "RSA PKCS#1 v1.5",
        ("RSA", "1.2.840.113549.1.1.11", "SHA-256")
        | ("RSA", "1.2.840.113549.1.1.12", "SHA-384")
        | ("RSA", "1.2.840.113549.1.1.13", "SHA-512") => "RSA PKCS#1 v1.5",
        ("ECDSA", "1.2.840.10045.4.3.2", "SHA-256")
        | ("ECDSA", "1.2.840.10045.4.3.3", "SHA-384")
        | ("ECDSA", "1.2.840.10045.4.3.4", "SHA-512") => "ECDSA",
        _ => {
            return Err(CheckError::unsupported(
                "This CMS signature algorithm or digest/key combination is not supported. RSA-PSS and EdDSA are not yet supported.",
            ));
        }
    };
    if key_kind == "RSA"
        && signer
            .signature_algorithm
            .parameters
            .as_ref()
            .is_some_and(|value| !value.is_null())
        || key_kind == "ECDSA" && signer.signature_algorithm.parameters.is_some()
    {
        return Err(CheckError::unsupported(
            "The CMS signature algorithm has unsupported parameters.",
        ));
    }
    Ok(CmsDetails {
        digest: digest.into(),
        algorithm: algorithm.into(),
        certificate: certificate_info(&cert)?,
        signer: cert,
    })
}

pub(super) fn verify(der: &[u8], data: &[u8], details: &CmsDetails) -> CheckResult<()> {
    let mut cms = CmsContentInfo::from_der(der).map_err(crypto_error)?;
    let mut certificates = Stack::new().map_err(crypto_error)?;
    certificates
        .push(details.signer.clone())
        .map_err(crypto_error)?;
    // Do not disable content or signed-attribute verification. Identity trust is
    // deliberately separate: this app does not load a system/root trust store.
    cms.verify(
        Some(&certificates),
        None,
        Some(data),
        None,
        CMSOptions::BINARY | CMSOptions::NO_SIGNER_CERT_VERIFY | CMSOptions::NOINTERN,
    )
    .map_err(|_| {
        CheckError::invalid(
            "The CMS digest or cryptographic signature does not match the signed PDF bytes.",
        )
    })
}

pub(super) fn identity(bytes: &[u8], password: &str) -> CheckResult<Identity> {
    if password.len() > 1024 || password.contains('\0') {
        return Err(CheckError::invalid(
            "The identity password is invalid or exceeds 1,024 bytes.",
        ));
    }
    let parsed = Pkcs12::from_der(bytes).and_then(|pfx| pfx.parse2(password))
        .map_err(|_| CheckError::invalid(
            "The PKCS#12 identity could not be unlocked. Check its password and use a modern PFX/P12 file with a certificate and private key; legacy RC2 identities are not supported.",
        ))?;
    let key = parsed
        .pkey
        .ok_or_else(|| CheckError::invalid("The identity contains no private signing key."))?;
    let certificate = parsed
        .cert
        .ok_or_else(|| CheckError::invalid("The identity contains no signing certificate."))?;
    let public = certificate.public_key().map_err(crypto_error)?;
    validate_key(&public)?;
    if !key.public_eq(&public) {
        return Err(CheckError::invalid(
            "The identity's certificate and private key do not match.",
        ));
    }
    let info = certificate_info(&certificate)?;
    if info.validity != CertificateValidity::ValidNow {
        return Err(CheckError::invalid(
            "The signing certificate is expired or not yet valid. Choose a certificate valid now.",
        ));
    }
    let parsed_cert =
        Certificate::from_der(&certificate.to_der().map_err(crypto_error)?).map_err(der_error)?;
    validate_key_usage(&parsed_cert)?;
    Ok(Identity {
        key,
        certificate,
        chain: parsed.ca,
    })
}

pub(super) fn sign(identity: &Identity, data: &[u8]) -> CheckResult<Vec<u8>> {
    let cms = CmsContentInfo::sign(
        Some(&identity.certificate),
        Some(&identity.key),
        identity.chain.as_deref(),
        Some(data),
        CMSOptions::BINARY | CMSOptions::DETACHED | CMSOptions::NOSMIMECAP,
    )
    .map_err(crypto_error)?;
    let der = cms.to_der().map_err(crypto_error)?;
    let details = cms_details(&der)?;
    if details.digest != "SHA-256" {
        return Err(CheckError::unsupported(
            "The signing provider did not produce the required SHA-256 digest.",
        ));
    }
    verify(&der, data, &details)?;
    Ok(der)
}
