use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cms::{
    cert::{CertificateChoices, IssuerAndSerialNumber},
    content_info::{CmsVersion, ContentInfo},
    signed_data::{
        CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo,
        SignerInfos,
    },
};
use der::{
    Any, Decode, Encode, Sequence,
    asn1::{BitString, GeneralizedTime, ObjectIdentifier, OctetString, SetOfVec, Uint},
};
use folio_core::{
    document::Session,
    signatures::{EvidenceKind, Integrity, RevocationStatus, TrustedTimeStatus},
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, Object};
use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::BigNum,
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkcs12::Pkcs12,
    pkey::{PKey, Private},
    sign::Signer,
    stack::Stack,
    x509::{
        X509, X509NameBuilder,
        extension::{
            AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage,
            SubjectKeyIdentifier,
        },
    },
};
use x509_cert::{
    Certificate, Version,
    attr::Attribute,
    crl::{CertificateList, RevokedCert, TbsCertList},
    ext::{
        Extension,
        pkix::{AuthorityKeyIdentifier as DerAki, CrlReason, SubjectKeyIdentifier as DerSki},
    },
    serial_number::SerialNumber,
    spki::AlgorithmIdentifierOwned,
    time::Time,
};

const PASSWORD: &str = "runtime-only offline evidence identity";
const TST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ECDSA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn key() -> PKey<Private> {
    PKey::from_ec_key(
        EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
    )
    .unwrap()
}
fn parsed(cert: &X509) -> Certificate {
    Certificate::from_der(&cert.to_der().unwrap()).unwrap()
}
fn algorithm(oid: ObjectIdentifier) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned {
        oid,
        parameters: None,
    }
}
fn any(value: &impl Encode) -> Any {
    Any::from_der(&value.to_der().unwrap()).unwrap()
}
fn extension(oid: &str, critical: bool, value: &impl Encode) -> Extension {
    Extension {
        extn_id: ObjectIdentifier::new(oid).unwrap(),
        critical,
        extn_value: OctetString::new(value.to_der().unwrap()).unwrap(),
    }
}
fn time(seconds: u64) -> Time {
    Time::GeneralTime(GeneralizedTime::from_unix_duration(Duration::from_secs(seconds)).unwrap())
}

fn cert(
    name: &str,
    key: &PKey<Private>,
    issuer: Option<(&X509, &PKey<Private>)>,
    serial: u32,
    ca: bool,
    tsa: bool,
) -> X509 {
    let mut subject = X509NameBuilder::new().unwrap();
    subject.append_entry_by_text("CN", name).unwrap();
    let subject = subject.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    builder
        .set_serial_number(&Asn1Integer::from_bn(&BigNum::from_u32(serial).unwrap()).unwrap())
        .unwrap();
    builder.set_subject_name(&subject).unwrap();
    builder
        .set_issuer_name(issuer.map_or(subject.as_ref(), |(cert, _)| cert.subject_name()))
        .unwrap();
    builder.set_pubkey(key).unwrap();
    builder
        .set_not_before(&Asn1Time::from_unix(now() as i64 - 7 * 86400).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::from_unix(now() as i64 + 14 * 86400).unwrap())
        .unwrap();
    let mut basic = BasicConstraints::new();
    basic.critical();
    if ca {
        basic.ca().pathlen(if issuer.is_none() { 1 } else { 0 });
    }
    builder.append_extension(basic.build().unwrap()).unwrap();
    let mut usage = KeyUsage::new();
    usage.critical();
    if ca {
        usage.key_cert_sign().crl_sign();
    } else {
        usage.digital_signature();
    }
    builder.append_extension(usage.build().unwrap()).unwrap();
    if !ca {
        let mut usage = ExtendedKeyUsage::new();
        if tsa {
            usage.critical();
        }
        usage.other(if tsa {
            "1.3.6.1.5.5.7.3.8"
        } else {
            "1.3.6.1.5.5.7.3.36"
        });
        builder.append_extension(usage.build().unwrap()).unwrap();
    }
    let ski = SubjectKeyIdentifier::new()
        .build(&builder.x509v3_context(issuer.map(|(cert, _)| cert.as_ref()), None))
        .unwrap();
    builder.append_extension(ski).unwrap();
    let aki = AuthorityKeyIdentifier::new()
        .keyid(true)
        .build(&builder.x509v3_context(issuer.map(|(cert, _)| cert.as_ref()), None))
        .unwrap();
    builder.append_extension(aki).unwrap();
    builder
        .sign(issuer.map_or(key, |(_, key)| key), MessageDigest::sha256())
        .unwrap();
    builder.build()
}

struct Pki {
    root_key: PKey<Private>,
    root: X509,
    issuer_key: PKey<Private>,
    issuer: X509,
    signer_key: PKey<Private>,
    signer: X509,
    tsa_key: PKey<Private>,
    tsa: X509,
}

fn pki() -> &'static Pki {
    static PKI: OnceLock<Pki> = OnceLock::new();
    PKI.get_or_init(|| {
        let root_key = key();
        let root = cert("Runtime evidence root", &root_key, None, 1, true, false);
        let issuer_key = key();
        let issuer = cert(
            "Runtime evidence issuer",
            &issuer_key,
            Some((&root, &root_key)),
            2,
            true,
            false,
        );
        let signer_key = key();
        let signer = cert(
            "Runtime document signer",
            &signer_key,
            Some((&issuer, &issuer_key)),
            3,
            false,
            false,
        );
        let tsa_key = key();
        let tsa = cert(
            "Runtime exclusive TSA",
            &tsa_key,
            Some((&issuer, &issuer_key)),
            4,
            false,
            true,
        );
        Pki {
            root_key,
            root,
            issuer_key,
            issuer,
            signer_key,
            signer,
            tsa_key,
            tsa,
        }
    })
}

fn signed_pdf() -> Vec<u8> {
    let pki = pki();
    let mut chain = Stack::new().unwrap();
    chain.push(pki.issuer.clone()).unwrap();
    chain.push(pki.root.clone()).unwrap();
    let identity = Pkcs12::builder()
        .name("Runtime only")
        .pkey(&pki.signer_key)
        .cert(&pki.signer)
        .ca(chain)
        .build2(PASSWORD)
        .unwrap()
        .to_der()
        .unwrap();
    Session::blank(200.0, 300.0, 1)
        .unwrap()
        .signed_copy(&identity, PASSWORD)
        .unwrap()
}

fn cms(bytes: &[u8]) -> (Vec<u8>, usize, usize) {
    let document = Document::load_mem(bytes).unwrap();
    let dictionary = document
        .objects
        .values()
        .filter_map(|object| object.as_dict().ok())
        .find(|dictionary| dictionary.get(b"Type").and_then(Object::as_name).ok() == Some(b"Sig"))
        .unwrap();
    let range = dictionary.get(b"ByteRange").unwrap().as_array().unwrap();
    let first = range[1].as_i64().unwrap() as usize;
    let second = range[2].as_i64().unwrap() as usize;
    let value = dictionary.get(b"Contents").unwrap().as_str().unwrap();
    let (header, length) = if value[1] < 128 {
        (2, value[1] as usize)
    } else {
        let count = (value[1] & 0x7f) as usize;
        (
            count + 2,
            value[2..count + 2]
                .iter()
                .fold(0usize, |number, byte| (number << 8) | *byte as usize),
        )
    };
    (value[..header + length].to_vec(), first, second)
}

fn signature_value(bytes: &[u8]) -> Vec<u8> {
    let content = ContentInfo::from_der(&cms(bytes).0).unwrap();
    let data: SignedData = content.content.decode_as().unwrap();
    data.signer_infos
        .0
        .iter()
        .next()
        .unwrap()
        .signature
        .as_bytes()
        .to_vec()
}

fn sign_crl(tbs: TbsCertList, key: &PKey<Private>) -> Vec<u8> {
    let mut signer = Signer::new(MessageDigest::sha256(), key).unwrap();
    signer.update(&tbs.to_der().unwrap()).unwrap();
    CertificateList {
        signature_algorithm: tbs.signature.clone(),
        tbs_cert_list: tbs,
        signature: BitString::from_bytes(&signer.sign_to_vec().unwrap()).unwrap(),
    }
    .to_der()
    .unwrap()
}

fn crl(
    issuer: &X509,
    key: &PKey<Private>,
    number: u8,
    revoked: &[&X509],
    start: u64,
    end: u64,
) -> Vec<u8> {
    let certificate = parsed(issuer);
    let ski = certificate
        .tbs_certificate
        .get::<DerSki>()
        .unwrap()
        .unwrap()
        .1
        .0;
    let aki = DerAki {
        key_identifier: Some(ski),
        authority_cert_issuer: None,
        authority_cert_serial_number: None,
    };
    let entries = revoked
        .iter()
        .map(|certificate| RevokedCert {
            serial_number: parsed(certificate).tbs_certificate.serial_number,
            revocation_date: time(start - 1),
            crl_entry_extensions: Some(vec![extension(
                "2.5.29.21",
                false,
                &CrlReason::KeyCompromise,
            )]),
        })
        .collect::<Vec<_>>();
    sign_crl(
        TbsCertList {
            version: Version::V2,
            signature: algorithm(ECDSA256),
            issuer: certificate.tbs_certificate.subject,
            this_update: time(start),
            next_update: Some(time(end)),
            revoked_certificates: (!entries.is_empty()).then_some(entries),
            crl_extensions: Some(vec![
                extension("2.5.29.35", false, &aki),
                extension("2.5.29.20", false, &Uint::new(&[number]).unwrap()),
            ]),
        },
        key,
    )
}

#[derive(Sequence)]
struct Imprint {
    algorithm: AlgorithmIdentifierOwned,
    hash: OctetString,
}
#[derive(Sequence)]
struct TestTstInfo {
    version: u8,
    policy: ObjectIdentifier,
    imprint: Imprint,
    serial: Uint,
    gen_time: GeneralizedTime,
    nonce: Option<Uint>,
}
#[derive(Sequence)]
struct EssId {
    hash: OctetString,
}
#[derive(Sequence)]
struct EssBinding {
    certs: Vec<EssId>,
}

fn attr(oid: &str, value: &impl Encode) -> Attribute {
    Attribute {
        oid: ObjectIdentifier::new(oid).unwrap(),
        values: SetOfVec::try_from(vec![any(value)]).unwrap(),
    }
}

fn token(value: &[u8], gen_time: u64, tsa: &X509, key: &PKey<Private>, wrong_ess: bool) -> Vec<u8> {
    let pki = pki();
    let content = TestTstInfo {
        version: 1,
        policy: ObjectIdentifier::new_unwrap("1.3.6.1.4.1.55555.2"),
        imprint: Imprint {
            algorithm: algorithm(SHA256),
            hash: OctetString::new(openssl::sha::sha256(value)).unwrap(),
        },
        serial: Uint::new(&[0x12, 0x34]).unwrap(),
        gen_time: GeneralizedTime::from_unix_duration(Duration::from_secs(gen_time)).unwrap(),
        nonce: Some(Uint::new(&[0x01, 0x23, 0x45, 0x67]).unwrap()),
    }
    .to_der()
    .unwrap();
    let ess = EssBinding {
        certs: vec![EssId {
            hash: OctetString::new(if wrong_ess {
                [0u8; 32]
            } else {
                openssl::sha::sha256(&tsa.to_der().unwrap())
            })
            .unwrap(),
        }],
    };
    let attributes = SetOfVec::try_from(vec![
        attr("1.2.840.113549.1.9.3", &TST),
        attr(
            "1.2.840.113549.1.9.4",
            &OctetString::new(openssl::sha::sha256(&content)).unwrap(),
        ),
        attr("1.2.840.113549.1.9.16.2.47", &ess),
    ])
    .unwrap();
    let mut signer = Signer::new(MessageDigest::sha256(), key).unwrap();
    signer.update(&attributes.to_der().unwrap()).unwrap();
    let certificate = parsed(tsa);
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: certificate.tbs_certificate.issuer.clone(),
            serial_number: certificate.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: algorithm(SHA256),
        signed_attrs: Some(attributes),
        signature_algorithm: algorithm(ECDSA256),
        signature: OctetString::new(signer.sign_to_vec().unwrap()).unwrap(),
        unsigned_attrs: None,
    };
    let signed = SignedData {
        version: CmsVersion::V3,
        digest_algorithms: SetOfVec::try_from(vec![algorithm(SHA256)]).unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: TST,
            econtent: Some(any(&OctetString::new(content).unwrap())),
        },
        certificates: Some(CertificateSet(
            SetOfVec::try_from(vec![
                CertificateChoices::Certificate(certificate),
                CertificateChoices::Certificate(parsed(&pki.issuer)),
                CertificateChoices::Certificate(parsed(&pki.root)),
            ])
            .unwrap(),
        )),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer]).unwrap()),
    };
    ContentInfo {
        content_type: ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2"),
        content: any(&signed),
    }
    .to_der()
    .unwrap()
}

fn embedded(mut bytes: Vec<u8>, value: &[u8], oid: &str) -> Vec<u8> {
    let (encoded, first, second) = cms(&bytes);
    let mut content = ContentInfo::from_der(&encoded).unwrap();
    let mut data: SignedData = content.content.decode_as().unwrap();
    let mut signer = data.signer_infos.0.iter().next().unwrap().clone();
    signer.unsigned_attrs = Some(
        SetOfVec::try_from(vec![Attribute {
            oid: ObjectIdentifier::new(oid).unwrap(),
            values: SetOfVec::try_from(vec![Any::from_der(value).unwrap()]).unwrap(),
        }])
        .unwrap(),
    );
    data.signer_infos = SignerInfos(SetOfVec::try_from(vec![signer]).unwrap());
    content.content = any(&data);
    let encoded = content.to_der().unwrap();
    assert!(encoded.len() * 2 < second - first - 2);
    let before = [bytes[..first].to_vec(), bytes[second..].to_vec()];
    bytes[first + 1..second - 1].fill(b'0');
    for (index, byte) in encoded.iter().enumerate() {
        bytes[first + 1 + index * 2..first + 3 + index * 2]
            .copy_from_slice(format!("{byte:02X}").as_bytes());
    }
    assert_eq!(bytes[..first], before[0]);
    assert_eq!(bytes[second..], before[1]);
    bytes
}

fn workspace(bytes: Vec<u8>) -> (Workspace, u64) {
    let mut workspace = Workspace::default();
    let OpenOutcome::Opened { document, .. } = workspace
        .request_open_bytes(bytes, "Runtime evidence.pdf".into())
        .unwrap()
    else {
        panic!()
    };
    (workspace, document.id)
}

fn add(workspace: &mut Workspace, id: u64, kind: EvidenceKind, bytes: &[u8]) {
    let info = workspace.document(id).unwrap().info().unwrap();
    let report = workspace.signature_report(id, info.revision).unwrap();
    let target = if kind == EvidenceKind::Timestamp {
        Some(report.signatures[0].id.as_str())
    } else {
        None
    };
    workspace
        .add_signature_evidence(report.evidence_context(), kind, target, bytes)
        .unwrap();
}

fn anchor(workspace: &mut Workspace, id: u64) {
    let revision = workspace.document(id).unwrap().revision();
    let trust = workspace.signature_trust_store().unwrap().revision;
    workspace
        .add_signature_anchor(id, revision, trust, &pki().root.to_der().unwrap())
        .unwrap();
}

fn full_crls(workspace: &mut Workspace, id: u64, revoked: &[&X509]) {
    let pki = pki();
    add(
        workspace,
        id,
        EvidenceKind::Crl,
        &crl(
            &pki.issuer,
            &pki.issuer_key,
            1,
            revoked,
            now() - 60,
            now() + 86400,
        ),
    );
    add(
        workspace,
        id,
        EvidenceKind::Crl,
        &crl(&pki.root, &pki.root_key, 1, &[], now() - 60, now() + 86400),
    );
}

#[test]
fn full_actual_chain_crls_verify_exact_serials_without_changing_pdf_or_signer_anchoring() {
    let bytes = signed_pdf();
    let (mut workspace, id) = workspace(bytes.clone());
    let source = serde_json::to_value(workspace.document(id).unwrap().info().unwrap()).unwrap();
    anchor(&mut workspace, id);
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        report.signatures[0].revocation.status,
        RevocationStatus::Incomplete
    );
    full_crls(&mut workspace, id, &[]);
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        report.signatures[0].revocation.status,
        RevocationStatus::GoodCurrentEvidence
    );
    assert_eq!(report.signatures[0].revocation.certificates.len(), 2);
    assert!(
        report.signatures[0]
            .revocation
            .certificates
            .iter()
            .all(|certificate| certificate.evidence_fingerprint.is_some())
    );
    add(
        &mut workspace,
        id,
        EvidenceKind::Crl,
        &crl(
            &pki().issuer,
            &pki().issuer_key,
            2,
            &[&pki().signer],
            now() - 30,
            now() + 86400,
        ),
    );
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Intact);
    assert_eq!(report.signatures[0].chain.status, "anchored_custom_store");
    assert_eq!(
        report.signatures[0].revocation.status,
        RevocationStatus::Revoked
    );
    assert_eq!(report.signatures[0].revocation.certificates[0].serial, "03");
    assert!(
        report.signatures[0].revocation.certificates[0]
            .revocation_time
            .is_some()
    );
    assert_eq!(workspace.document(id).unwrap().bytes(), bytes);
    assert_eq!(
        serde_json::to_value(workspace.document(id).unwrap().info().unwrap()).unwrap(),
        source
    );
}

#[test]
fn expired_future_wrong_key_and_conflicting_crls_never_fall_back_to_older_good_evidence() {
    let pki = pki();
    for (start, end, wrong_key, expected) in [
        (now() - 30, now() - 1, false, RevocationStatus::Expired),
        (now() + 60, now() + 86400, false, RevocationStatus::Future),
        (now() - 30, now() + 86400, true, RevocationStatus::Invalid),
    ] {
        let (mut workspace, id) = workspace(signed_pdf());
        anchor(&mut workspace, id);
        full_crls(&mut workspace, id, &[]);
        let wrong = key();
        add(
            &mut workspace,
            id,
            EvidenceKind::Crl,
            &crl(
                &pki.issuer,
                if wrong_key { &wrong } else { &pki.issuer_key },
                2,
                &[],
                start,
                end,
            ),
        );
        assert_eq!(
            workspace.signature_report(id, 0).unwrap().signatures[0]
                .revocation
                .status,
            expected
        );
    }
    let (mut workspace, id) = workspace(signed_pdf());
    anchor(&mut workspace, id);
    full_crls(&mut workspace, id, &[]);
    add(
        &mut workspace,
        id,
        EvidenceKind::Crl,
        &crl(
            &pki.issuer,
            &pki.issuer_key,
            1,
            &[&pki.signer],
            now() - 60,
            now() + 86400,
        ),
    );
    assert_eq!(
        workspace.signature_report(id, 0).unwrap().signatures[0]
            .revocation
            .status,
        RevocationStatus::Invalid
    );
}

#[test]
fn unsupported_delta_indirect_unknown_extensions_duplicate_serials_and_nonexact_inputs_are_atomic_failures()
 {
    let pki = pki();
    let original = crl(
        &pki.issuer,
        &pki.issuer_key,
        1,
        &[],
        now() - 60,
        now() + 86400,
    );
    let (mut workspace, id) = workspace(signed_pdf());
    anchor(&mut workspace, id);
    let before = serde_json::to_value(workspace.signature_trust_store().unwrap()).unwrap();
    let mut controls = Vec::new();
    let mut trailing = original.clone();
    trailing.push(0);
    controls.push(trailing);
    for (oid, critical) in [
        ("2.5.29.27", true),
        ("2.5.29.28", true),
        ("1.2.3.4.5", false),
    ] {
        let mut parsed = CertificateList::from_der(&original).unwrap();
        parsed
            .tbs_cert_list
            .crl_extensions
            .as_mut()
            .unwrap()
            .push(extension(oid, critical, &1u8));
        controls.push(sign_crl(parsed.tbs_cert_list, &pki.issuer_key));
    }
    controls.push(crl(
        &pki.issuer,
        &pki.issuer_key,
        2,
        &[&pki.signer, &pki.signer],
        now() - 60,
        now() + 86400,
    ));
    let mut remove = CertificateList::from_der(&crl(
        &pki.issuer,
        &pki.issuer_key,
        2,
        &[&pki.signer],
        now() - 60,
        now() + 86400,
    ))
    .unwrap();
    remove.tbs_cert_list.revoked_certificates.as_mut().unwrap()[0].crl_entry_extensions = Some(
        vec![extension("2.5.29.21", false, &CrlReason::RemoveFromCRL)],
    );
    controls.push(sign_crl(remove.tbs_cert_list, &pki.issuer_key));
    controls.push(b"-----BEGIN PRIVATE KEY-----\nYWJj\n-----END PRIVATE KEY-----".to_vec());
    for bytes in controls {
        let report = workspace.signature_report(id, 0).unwrap();
        assert!(
            workspace
                .add_signature_evidence(report.evidence_context(), EvidenceKind::Crl, None, &bytes)
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(workspace.signature_trust_store().unwrap()).unwrap(),
            before
        );
    }
}

#[test]
fn authentic_bound_timestamp_requires_both_custom_paths_and_complete_current_tsa_crls() {
    let bytes = signed_pdf();
    let value = signature_value(&bytes);
    let timestamp = token(&value, now() - 10, &pki().tsa, &pki().tsa_key, false);
    let (mut workspace, id) = workspace(bytes.clone());
    add(&mut workspace, id, EvidenceKind::Timestamp, &timestamp);
    assert_eq!(
        workspace.signature_report(id, 0).unwrap().signatures[0]
            .trusted_time
            .status,
        TrustedTimeStatus::Untrusted
    );
    anchor(&mut workspace, id);
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        report.signatures[0].trusted_time.status,
        TrustedTimeStatus::Incomplete
    );
    assert_eq!(
        report.signatures[0].trusted_time.timestamps[0].cryptographic_integrity,
        Integrity::Intact
    );
    full_crls(&mut workspace, id, &[]);
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        report.signatures[0].trusted_time.status,
        TrustedTimeStatus::VerifiedCustomStore,
        "{:?}",
        report.signatures[0].trusted_time
    );
    let timestamp = &report.signatures[0].trusted_time.timestamps[0];
    assert_eq!(timestamp.imprint_matches, Some(true));
    assert_eq!(timestamp.tsa_current_chain.status, "anchored_custom_store");
    assert_eq!(
        timestamp.tsa_chain_at_gen_time.status,
        "anchored_custom_store"
    );
    assert_eq!(
        timestamp.tsa_revocation.status,
        RevocationStatus::GoodCurrentEvidence
    );
    assert_eq!(workspace.document(id).unwrap().bytes(), bytes);
}

#[test]
fn a_verified_timestamp_never_clears_current_signer_revocation_and_tsa_revocation_is_separate() {
    let bytes = signed_pdf();
    let timestamp = token(
        &signature_value(&bytes),
        now() - 10,
        &pki().tsa,
        &pki().tsa_key,
        false,
    );
    let (mut workspace, id) = workspace(bytes);
    anchor(&mut workspace, id);
    full_crls(&mut workspace, id, &[&pki().signer]);
    add(&mut workspace, id, EvidenceKind::Timestamp, &timestamp);
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        report.signatures[0].trusted_time.status,
        TrustedTimeStatus::VerifiedCustomStore
    );
    assert_eq!(
        report.signatures[0].revocation.status,
        RevocationStatus::Revoked
    );
    assert_eq!(report.signatures[0].chain.status, "anchored_custom_store");
    add(
        &mut workspace,
        id,
        EvidenceKind::Crl,
        &crl(
            &pki().issuer,
            &pki().issuer_key,
            2,
            &[&pki().signer, &pki().tsa],
            now() - 30,
            now() + 86400,
        ),
    );
    let report = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        report.signatures[0].trusted_time.status,
        TrustedTimeStatus::Revoked
    );
    assert_eq!(
        report.signatures[0].revocation.status,
        RevocationStatus::Revoked
    );
}

#[test]
fn genuine_future_untrusted_wrong_imprint_bad_ess_and_unknown_embedded_attributes_do_not_become_time_assurance()
 {
    let bytes = signed_pdf();
    let value = signature_value(&bytes);
    for (timestamp, expected) in [
        (
            token(&value, now() + 86400, &pki().tsa, &pki().tsa_key, false),
            TrustedTimeStatus::Future,
        ),
        (
            token(
                b"not the actual CMS value",
                now() - 10,
                &pki().tsa,
                &pki().tsa_key,
                false,
            ),
            TrustedTimeStatus::Invalid,
        ),
        (
            token(&value, now() - 10, &pki().tsa, &pki().tsa_key, true),
            TrustedTimeStatus::Invalid,
        ),
    ] {
        let control = embedded(bytes.clone(), &timestamp, "1.2.840.113549.1.9.16.2.14");
        let (mut workspace, id) = workspace(control.clone());
        anchor(&mut workspace, id);
        full_crls(&mut workspace, id, &[]);
        add(
            &mut workspace,
            id,
            EvidenceKind::Timestamp,
            &token(&value, now() - 10, &pki().tsa, &pki().tsa_key, false),
        );
        let report = workspace.signature_report(id, 0).unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Intact);
        assert_eq!(report.signatures[0].trusted_time.status, expected);
        assert!(
            report.signatures[0]
                .trusted_time
                .timestamps
                .iter()
                .any(|timestamp| timestamp.source == "imported"
                    && timestamp.status == TrustedTimeStatus::VerifiedCustomStore)
        );
        assert_eq!(workspace.document(id).unwrap().bytes(), control);
    }
    let timestamp = token(&value, now() - 10, &pki().tsa, &pki().tsa_key, false);
    let (mut workspace, id) = workspace(embedded(
        bytes.clone(),
        &timestamp,
        "1.2.840.113549.1.9.16.2.14",
    ));
    anchor(&mut workspace, id);
    full_crls(&mut workspace, id, &[]);
    assert_eq!(
        workspace.signature_report(id, 0).unwrap().signatures[0]
            .trusted_time
            .status,
        TrustedTimeStatus::VerifiedCustomStore
    );
    let session = Session::from_bytes(
        embedded(bytes, &timestamp, "1.2.3.4.5"),
        "Unsupported unsigned attribute.pdf".into(),
        None,
        1,
    )
    .unwrap();
    assert_eq!(
        session.signature_report().unwrap().signatures[0].integrity,
        Integrity::Unsupported
    );
}

#[test]
fn wrong_detached_imprints_stale_epochs_revisions_and_removal_preserve_source_and_all_pending_drafts()
 {
    let bytes = signed_pdf();
    let timestamp = token(
        &signature_value(&bytes),
        now() - 10,
        &pki().tsa,
        &pki().tsa_key,
        false,
    );
    let (mut workspace, id) = workspace(bytes.clone());
    anchor(&mut workspace, id);
    let before = workspace.signature_report(id, 0).unwrap();
    let wrong = token(
        b"wrong actual value",
        now() - 10,
        &pki().tsa,
        &pki().tsa_key,
        false,
    );
    assert!(
        workspace
            .add_signature_evidence(
                before.evidence_context(),
                EvidenceKind::Timestamp,
                Some(&before.signatures[0].id),
                &wrong
            )
            .unwrap_err()
            .to_string()
            .contains("imprint")
    );
    assert_eq!(
        workspace.signature_trust_store().unwrap().revision,
        before.trust_store.revision
    );
    workspace.set_draft(id, true).unwrap();
    assert!(
        workspace
            .add_signature_evidence(
                before.evidence_context(),
                EvidenceKind::Timestamp,
                Some(&before.signatures[0].id),
                &timestamp
            )
            .is_err()
    );
    assert!(workspace.has_unsaved_changes());
    add(&mut workspace, id, EvidenceKind::Timestamp, &timestamp);
    let report = workspace.signature_report(id, 0).unwrap();
    assert!(
        workspace
            .remove_signature_evidence(
                folio_core::signatures::SignatureEvidenceContext {
                    trust_revision: before.trust_store.revision,
                    ..report.evidence_context()
                },
                &report.trust_store.evidence[0].fingerprint
            )
            .is_err()
    );
    workspace
        .remove_signature_evidence(
            report.evidence_context(),
            &report.trust_store.evidence[0].fingerprint,
        )
        .unwrap();
    assert!(
        workspace
            .signature_trust_store()
            .unwrap()
            .evidence
            .is_empty()
    );
    assert!(workspace.has_unsaved_changes());
    assert_eq!(workspace.document(id).unwrap().bytes(), bytes);
    assert_eq!(
        Workspace::default()
            .signature_trust_store()
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn exact_pem_decoding_does_not_canonicalize_a_cryptographically_valid_ber_crl_into_der() {
    let pki = pki();
    let der = crl(
        &pki.issuer,
        &pki.issuer_key,
        1,
        &[],
        now() - 60,
        now() + 86400,
    );
    let header = if der[1] < 128 {
        2
    } else {
        2 + (der[1] & 0x7f) as usize
    };
    let mut ber = vec![0x30, 0x80];
    ber.extend_from_slice(&der[header..]);
    ber.extend_from_slice(&[0, 0]);
    assert!(
        openssl::x509::X509Crl::from_der(&ber)
            .unwrap()
            .verify(&pki.issuer.public_key().unwrap())
            .unwrap()
    );
    let pem = der::pem::encode_string("X509 CRL", der::pem::LineEnding::LF, &ber)
        .unwrap()
        .into_bytes();
    let (mut workspace, id) = workspace(signed_pdf());
    let before = workspace.signature_report(id, 0).unwrap();
    for input in [ber.as_slice(), pem.as_slice()] {
        assert!(
            workspace
                .add_signature_evidence(before.evidence_context(), EvidenceKind::Crl, None, input)
                .is_err()
        );
        assert_eq!(workspace.signature_trust_store().unwrap().revision, 0);
    }
    let valid = openssl::x509::X509Crl::from_der(&der)
        .unwrap()
        .to_pem()
        .unwrap();
    add(&mut workspace, id, EvidenceKind::Crl, &valid);
    assert_eq!(
        workspace.signature_trust_store().unwrap().evidence[0].bytes,
        der.len()
    );
}

#[test]
fn actual_crl_expiry_bounds_cached_reports_and_revokes_current_time_assurance_without_mutating_source()
 {
    let bytes = signed_pdf();
    let timestamp = token(
        &signature_value(&bytes),
        now() - 10,
        &pki().tsa,
        &pki().tsa_key,
        false,
    );
    let (mut workspace, id) = workspace(bytes.clone());
    anchor(&mut workspace, id);
    let expires = now() + 3;
    add(
        &mut workspace,
        id,
        EvidenceKind::Crl,
        &crl(
            &pki().issuer,
            &pki().issuer_key,
            1,
            &[],
            now() - 60,
            expires,
        ),
    );
    add(
        &mut workspace,
        id,
        EvidenceKind::Crl,
        &crl(
            &pki().root,
            &pki().root_key,
            1,
            &[],
            now() - 60,
            now() + 86400,
        ),
    );
    add(&mut workspace, id, EvidenceKind::Timestamp, &timestamp);
    let current = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        current.signatures[0].trusted_time.status,
        TrustedTimeStatus::VerifiedCustomStore
    );
    assert!(current.recheck_at_unix_ms <= expires * 1000 + 1);
    assert!(current.recheck_at_unix_ms > current.checked_at_unix_ms);
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::thread::sleep(Duration::from_millis(
        (expires * 1000 + 20).saturating_sub(milliseconds),
    ));
    let expired = workspace.signature_report(id, 0).unwrap();
    assert_eq!(
        expired.signatures[0].revocation.status,
        RevocationStatus::Expired
    );
    assert_eq!(
        expired.signatures[0].trusted_time.status,
        TrustedTimeStatus::Incomplete
    );
    assert_eq!(
        expired.signatures[0].trusted_time.timestamps[0].cryptographic_integrity,
        Integrity::Intact
    );
    assert_eq!(expired.trust_store.revision, current.trust_store.revision);
    assert_eq!(workspace.document(id).unwrap().bytes(), bytes);
}

fn numbered_entries(count: usize, wide: bool, issued: u64) -> Vec<RevokedCert> {
    (0..count)
        .map(|index| {
            let mut serial = vec![0u8; if wide { 20 } else { 8 }];
            if wide {
                serial[0] = 1;
            }
            let end = serial.len();
            serial[end - 8..].copy_from_slice(&(index as u64 + 1).to_be_bytes());
            RevokedCert {
                serial_number: SerialNumber::new(&serial).unwrap(),
                revocation_date: time(issued - 1),
                crl_entry_extensions: None,
            }
        })
        .collect()
}

#[test]
fn actual_signed_der_enforces_inclusive_32768_entries_and_32_store_items() {
    let pki = pki();
    let issued = now() - 60;
    let mut tbs = CertificateList::from_der(&crl(
        &pki.issuer,
        &pki.issuer_key,
        1,
        &[],
        issued,
        now() + 86400,
    ))
    .unwrap()
    .tbs_cert_list;
    tbs.revoked_certificates = Some(numbered_entries(32_768, false, issued));
    let accepted = sign_crl(tbs.clone(), &pki.issuer_key);
    assert!(accepted.len() < folio_core::signatures::MAX_CRL_BYTES);
    let (mut workspace, id) = workspace(signed_pdf());
    add(&mut workspace, id, EvidenceKind::Crl, &accepted);
    tbs.revoked_certificates = Some(numbered_entries(32_769, false, issued));
    let excessive = sign_crl(tbs, &pki.issuer_key);
    assert!(excessive.len() < folio_core::signatures::MAX_CRL_BYTES);
    let before = workspace.signature_report(id, 0).unwrap();
    assert!(
        workspace
            .add_signature_evidence(
                before.evidence_context(),
                EvidenceKind::Crl,
                None,
                &excessive
            )
            .unwrap_err()
            .to_string()
            .contains("32,768")
    );
    for number in 2..=32 {
        add(
            &mut workspace,
            id,
            EvidenceKind::Crl,
            &crl(
                &pki.issuer,
                &pki.issuer_key,
                number,
                &[],
                issued,
                now() + 86400,
            ),
        );
    }
    let before = workspace.signature_report(id, 0).unwrap();
    assert_eq!(before.trust_store.evidence.len(), 32);
    let extra = crl(&pki.issuer, &pki.issuer_key, 33, &[], issued, now() + 86400);
    assert!(
        workspace
            .add_signature_evidence(before.evidence_context(), EvidenceKind::Crl, None, &extra)
            .is_err()
    );
    assert_eq!(
        workspace.signature_trust_store().unwrap().revision,
        before.trust_store.revision
    );
}

#[test]
fn eight_mib_store_bound_counts_real_verified_der_not_file_names_or_metadata_estimates() {
    let pki = pki();
    let issued = now() - 60;
    let mut tbs = CertificateList::from_der(&crl(
        &pki.issuer,
        &pki.issuer_key,
        1,
        &[],
        issued,
        now() + 86400,
    ))
    .unwrap()
    .tbs_cert_list;
    tbs.revoked_certificates = Some(numbered_entries(24_000, true, issued));
    let (mut workspace, id) = workspace(signed_pdf());
    let mut exact_bytes = 0;
    let mut hit_limit = false;
    for number in 1..=10 {
        tbs.crl_extensions.as_mut().unwrap()[1] =
            extension("2.5.29.20", false, &Uint::new(&[number]).unwrap());
        let der = sign_crl(tbs.clone(), &pki.issuer_key);
        assert!(der.len() <= folio_core::signatures::MAX_CRL_BYTES);
        assert!(
            openssl::x509::X509Crl::from_der(&der)
                .unwrap()
                .verify(&pki.issuer.public_key().unwrap())
                .unwrap()
        );
        let before = workspace.signature_report(id, 0).unwrap();
        let result = workspace.add_signature_evidence(
            before.evidence_context(),
            EvidenceKind::Crl,
            None,
            &der,
        );
        if exact_bytes + der.len() > folio_core::signatures::MAX_EVIDENCE_STORE_BYTES {
            assert!(result.unwrap_err().to_string().contains("8 MiB"));
            let after = workspace.signature_trust_store().unwrap();
            assert_eq!(after.evidence_bytes, exact_bytes);
            assert_eq!(after.revision, before.trust_store.revision);
            hit_limit = true;
            break;
        }
        exact_bytes += der.len();
        assert_eq!(result.unwrap().evidence_bytes, exact_bytes);
    }
    assert!(hit_limit);
}
