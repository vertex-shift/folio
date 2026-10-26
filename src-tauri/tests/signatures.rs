use std::sync::OnceLock;

use folio_core::{
    document::{Edit, Session},
    redaction::Region,
    signatures::{CertificateValidity, Coverage, Integrity, read_identity_file},
    workspace::Workspace,
};
use lopdf::{
    Document, Object, dictionary,
    encryption::{EncryptionState, EncryptionVersion, Permissions},
};
use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::BigNum,
    cms::{CMSOptions, CmsContentInfo},
    ec::{EcGroup, EcKey},
    hash::MessageDigest,
    nid::Nid,
    pkcs12::Pkcs12,
    pkey::{PKey, Private},
    rsa::Rsa,
    x509::{X509, X509NameBuilder, extension::KeyUsage},
};

const PASSWORD: &str = "synthetic runtime password";

fn certificate(key: &PKey<Private>, before: i64, after: i64, signing: bool) -> X509 {
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "Folio runtime test signer")
        .unwrap();
    name.append_entry_by_text("O", "Synthetic tests only")
        .unwrap();
    let name = name.build();
    let mut certificate = X509::builder().unwrap();
    certificate.set_version(2).unwrap();
    certificate
        .set_serial_number(&Asn1Integer::from_bn(&BigNum::from_u32(42).unwrap()).unwrap())
        .unwrap();
    certificate.set_subject_name(&name).unwrap();
    certificate.set_issuer_name(&name).unwrap();
    certificate.set_pubkey(key).unwrap();
    certificate
        .set_not_before(&Asn1Time::from_unix(before).unwrap())
        .unwrap();
    certificate
        .set_not_after(&Asn1Time::from_unix(after).unwrap())
        .unwrap();
    let mut usage = KeyUsage::new();
    if signing {
        usage.digital_signature();
    } else {
        usage.key_encipherment();
    }
    certificate
        .append_extension(usage.build().unwrap())
        .unwrap();
    certificate.sign(key, MessageDigest::sha256()).unwrap();
    certificate.build()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn pfx(key: &PKey<Private>, certificate: &X509) -> Vec<u8> {
    Pkcs12::builder()
        .name("Runtime only")
        .pkey(key)
        .cert(certificate)
        .build2(PASSWORD)
        .unwrap()
        .to_der()
        .unwrap()
}

fn identity() -> &'static [u8] {
    static IDENTITY: OnceLock<Vec<u8>> = OnceLock::new();
    IDENTITY.get_or_init(|| {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        pfx(&key, &certificate(&key, now() - 3600, now() + 86400, true))
    })
}

fn signed() -> Vec<u8> {
    Session::blank(100.0, 100.0, 1)
        .unwrap()
        .signed_copy(identity(), PASSWORD)
        .unwrap()
}

fn open(bytes: Vec<u8>) -> Session {
    Session::from_bytes(bytes, "Signed.pdf".into(), None, 1).unwrap()
}

fn ranges(bytes: &[u8]) -> [usize; 4] {
    let document = Document::load_mem(bytes).unwrap();
    let dictionary = document
        .objects
        .values()
        .find_map(|object| {
            object
                .as_dict()
                .ok()
                .filter(|dict| dict.get(b"Type").and_then(Object::as_name).ok() == Some(b"Sig"))
        })
        .unwrap();
    let array = dictionary.get(b"ByteRange").unwrap().as_array().unwrap();
    std::array::from_fn(|index| array[index].as_i64().unwrap() as usize)
}

fn replace_range(bytes: &mut [u8], replacement: [usize; 4]) {
    let original = ranges(bytes);
    let original = format!(
        "[{} {} {} {}]",
        original[0], original[1], original[2], original[3]
    );
    let offset = bytes
        .windows(original.len())
        .position(|window| window == original.as_bytes())
        .unwrap();
    let replacement = format!(
        "[{} {} {} {}]",
        replacement[0], replacement[1], replacement[2], replacement[3]
    );
    assert_eq!(replacement.len(), original.len());
    bytes[offset..offset + replacement.len()].copy_from_slice(replacement.as_bytes());
}

fn cms_bytes(bytes: &[u8]) -> Vec<u8> {
    let [_, start, end, _] = ranges(bytes);
    let hex = &bytes[start + 1..end - 1];
    let padded: Vec<_> = hex
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    CmsContentInfo::from_der(&padded).unwrap().to_der().unwrap()
}

fn replace_cms(bytes: &mut [u8], cms: &[u8]) {
    let [_, start, end, _] = ranges(bytes);
    assert!(cms.len() * 2 < end - start - 2);
    bytes[start + 1..end - 1].fill(b'0');
    for (index, byte) in cms.iter().enumerate() {
        bytes[start + 1 + index * 2..start + 3 + index * 2]
            .copy_from_slice(format!("{byte:02X}").as_bytes());
    }
}

#[test]
fn rsa_signed_copy_verifies_in_an_independent_cms_call_and_never_claims_identity_trust() {
    let bytes = signed();
    let [_, first, second, length] = ranges(&bytes);
    let mut signed_data = bytes[..first].to_vec();
    signed_data.extend_from_slice(&bytes[second..second + length]);
    CmsContentInfo::from_der(&cms_bytes(&bytes))
        .unwrap()
        .verify(
            None,
            None,
            Some(&signed_data),
            None,
            CMSOptions::BINARY | CMSOptions::NO_SIGNER_CERT_VERIFY,
        )
        .unwrap();
    let session = open(bytes);
    assert!(session.info().unwrap().read_only_reason.is_some());
    let report = session.signature_report().unwrap();
    assert!(report.signing_blocked_reason.is_some());
    assert_eq!(report.signatures.len(), 1);
    let signature = &report.signatures[0];
    assert_eq!(signature.integrity, Integrity::Intact);
    assert_eq!(signature.coverage, Coverage::WholeDocument);
    assert_eq!(signature.digest.as_deref(), Some("SHA-256"));
    let certificate = signature.certificate.as_ref().unwrap();
    assert_eq!(certificate.validity, CertificateValidity::ValidNow);
    assert_eq!(certificate.identity_trust, "not_checked");
    assert!(certificate.self_signed);
    assert!(certificate.subject.contains("Folio runtime test signer"));
    assert_eq!(certificate.sha256_fingerprint.len(), 95);
}

#[test]
fn ecdsa_identity_is_supported_without_any_external_executable() {
    for curve in [Nid::X9_62_PRIME256V1, Nid::SECP384R1, Nid::SECP521R1] {
        let group = EcGroup::from_curve_name(curve).unwrap();
        let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
        let identity = pfx(&key, &certificate(&key, now() - 3600, now() + 86400, true));
        let bytes = Session::blank(72.0, 72.0, 1)
            .unwrap()
            .signed_copy(&identity, PASSWORD)
            .unwrap();
        let report = open(bytes).signature_report().unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Intact);
        assert_eq!(report.signatures[0].algorithm.as_deref(), Some("ECDSA"));
    }
}

#[test]
fn signing_exports_current_unsaved_content_without_changing_the_working_session() {
    let mut session = Session::demo(7).unwrap();
    session
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    session
        .apply(Edit::Note {
            page: 1,
            point: [30.0, 40.0],
            text: "Unsaved signed content".into(),
        })
        .unwrap();
    let before = session.bytes().to_vec();
    let revision = session.revision();
    let bytes = session.signed_copy(identity(), PASSWORD).unwrap();
    assert_eq!(session.bytes(), before);
    assert_eq!(session.revision(), revision);
    assert!(session.is_dirty());
    assert!(session.info().unwrap().can_undo);
    let reopened = open(bytes);
    assert_eq!(reopened.info().unwrap().pages[0].rotation, 90);
    assert_eq!(
        reopened.info().unwrap().annotations[0].text,
        "Unsaved signed content"
    );
    assert_eq!(
        reopened.signature_report().unwrap().signatures[0].integrity,
        Integrity::Intact
    );
    session.undo().unwrap();
    assert!(session.info().unwrap().annotations.is_empty());
}

#[test]
fn byte_tampering_is_not_mistaken_for_a_certificate_trust_failure() {
    let mut bytes = signed();
    bytes[7] = if bytes[7] == b'7' { b'6' } else { b'7' };
    let report = open(bytes).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Invalid);
    assert!(report.signatures[0].message.contains("does not match"));
    assert!(report.signatures[0].certificate.is_some());
}

#[test]
fn later_bytes_and_incremental_changes_never_get_whole_document_assurance() {
    let bytes = signed();
    let length = bytes.len();
    let mut trailing = bytes.clone();
    trailing.extend_from_slice(b"\nUnsigned trailing bytes");
    let report = open(trailing).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Intact);
    assert_eq!(report.signatures[0].coverage, Coverage::LaterChanges);
    assert_eq!(report.signatures[0].signed_bytes, Some(length));

    let document = Document::load_mem(&bytes).unwrap();
    let root = document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap();
    let id = document.max_id + 1;
    let mut updated = bytes;
    let offset = updated.len();
    updated
        .extend_from_slice(format!("\n{id} 0 obj\n(later unsigned revision)\nendobj\n").as_bytes());
    let xref = updated.len();
    updated.extend_from_slice(format!(
        "xref\n{id} 1\n{:010} 00000 n \ntrailer\n<< /Size {} /Root {} {} R /Prev {} >>\nstartxref\n{xref}\n%%EOF\n",
        offset + 1, id + 1, root.0, root.1, document.xref_start,
    ).as_bytes());
    let report = open(updated).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Intact);
    assert_eq!(report.signatures[0].coverage, Coverage::LaterChanges);
}

#[test]
fn ranges_must_start_at_zero_and_exclude_exactly_the_own_contents_token() {
    for case in 0..4 {
        let mut bytes = signed();
        let mut range = ranges(&bytes);
        match case {
            0 => range[0] = 1,
            1 => range[1] -= 1,
            2 => {
                range[2] += 1;
                range[3] -= 1;
            }
            3 => range[3] += 1,
            _ => unreachable!(),
        }
        replace_range(&mut bytes, range);
        let report = open(bytes).signature_report().unwrap();
        assert_eq!(
            report.signatures[0].integrity,
            Integrity::Invalid,
            "case {case}"
        );
    }
}

#[test]
fn unsigned_contents_padding_cannot_smuggle_nonzero_data() {
    let mut bytes = signed();
    let end = ranges(&bytes)[2];
    bytes[end - 2] = b'F';
    let report = open(bytes).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Invalid);
    assert!(report.signatures[0].message.contains("Nonzero"));
}

#[test]
fn unsupported_algorithms_and_attached_cms_are_explicitly_rejected() {
    use cms::{content_info::ContentInfo, signed_data::SignedData};
    use der::{
        Decode, Encode,
        asn1::{ObjectIdentifier, SetOfVec},
    };
    let mut bytes = signed();
    let mut info = ContentInfo::from_der(&cms_bytes(&bytes)).unwrap();
    let mut data: SignedData = info.content.decode_as().unwrap();
    let mut signer = data.signer_infos.0.iter().next().unwrap().clone();
    signer.digest_alg.oid = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
    data.digest_algorithms = SetOfVec::try_from(vec![signer.digest_alg.clone()]).unwrap();
    data.signer_infos.0 = SetOfVec::try_from(vec![signer]).unwrap();
    info.content = der::Any::from_der(&data.to_der().unwrap()).unwrap();
    replace_cms(&mut bytes, &info.to_der().unwrap());
    let report = open(bytes).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Unsupported);
    assert!(report.signatures[0].message.contains("SHA-256"));

    let parsed = Pkcs12::from_der(identity())
        .unwrap()
        .parse2(PASSWORD)
        .unwrap();
    let attached = CmsContentInfo::sign(
        parsed.cert.as_deref(),
        parsed.pkey.as_deref(),
        None,
        Some(b"attached data"),
        CMSOptions::BINARY | CMSOptions::NOSMIMECAP,
    )
    .unwrap()
    .to_der()
    .unwrap();
    let mut bytes = signed();
    replace_cms(&mut bytes, &attached);
    let report = open(bytes).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Unsupported);
    assert!(report.signatures[0].message.contains("detached"));
}

#[test]
fn wrong_passwords_expired_future_weak_and_inappropriate_keys_do_not_export() {
    let session = Session::blank(72.0, 72.0, 1).unwrap();
    let before = session.bytes().to_vec();
    let dirty = session.is_dirty();
    assert!(
        session
            .signed_copy(identity(), "incorrect")
            .unwrap_err()
            .to_string()
            .contains("could not be unlocked")
    );
    assert!(session.signed_copy(b"malformed PFX", PASSWORD).is_err());
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    for (before, after, usage) in [
        (now() - 86400, now() - 3600, true),
        (now() + 3600, now() + 86400, true),
        (now() - 3600, now() + 86400, false),
    ] {
        let identity = pfx(&key, &certificate(&key, before, after, usage));
        assert!(session.signed_copy(&identity, PASSWORD).is_err());
    }
    let weak = PKey::from_rsa(Rsa::generate(1024).unwrap()).unwrap();
    let identity = pfx(
        &weak,
        &certificate(&weak, now() - 3600, now() + 86400, true),
    );
    assert!(
        session
            .signed_copy(&identity, PASSWORD)
            .unwrap_err()
            .to_string()
            .contains("2048")
    );
    assert_eq!(session.is_dirty(), dirty);
    assert_eq!(session.bytes(), before);
}

#[test]
fn expired_signer_content_integrity_and_current_validity_remain_separate() {
    let parsed = Pkcs12::from_der(identity())
        .unwrap()
        .parse2(PASSWORD)
        .unwrap();
    let key = parsed.pkey.unwrap();
    let expired = certificate(&key, now() - 86400, now() - 3600, true);
    let mut bytes = signed();
    let [_, first, second, length] = ranges(&bytes);
    let mut data = bytes[..first].to_vec();
    data.extend_from_slice(&bytes[second..second + length]);
    let cms = CmsContentInfo::sign(
        Some(&expired),
        Some(&key),
        None,
        Some(&data),
        CMSOptions::BINARY | CMSOptions::DETACHED | CMSOptions::NOSMIMECAP,
    )
    .unwrap()
    .to_der()
    .unwrap();
    replace_cms(&mut bytes, &cms);
    let report = open(bytes).signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Intact);
    assert_eq!(
        report.signatures[0].certificate.as_ref().unwrap().validity,
        CertificateValidity::Expired
    );
    assert_eq!(
        report.signatures[0]
            .certificate
            .as_ref()
            .unwrap()
            .identity_trust,
        "not_checked"
    );
}

#[test]
fn encrypted_signed_certified_and_existing_signature_field_inputs_are_not_rewritten() {
    let session = open(signed());
    assert!(session.signed_copy(identity(), PASSWORD).is_err());
    assert!(session.extract_pages(&[0]).is_err());
    let mut document = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    let policy = document.add_object(dictionary! { "DocMDP" => Object::Null });
    document.catalog_mut().unwrap().set("Perms", policy);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let certified = open(bytes);
    assert!(certified.info().unwrap().read_only_reason.is_some());
    assert!(certified.signed_copy(identity(), PASSWORD).is_err());

    let mut document = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    document.add_object(
        dictionary! { "FT" => "Sig", "T" => Object::string_literal("Empty signature") },
    );
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    assert!(
        open(bytes)
            .signed_copy(identity(), PASSWORD)
            .unwrap_err()
            .to_string()
            .contains("Existing signature fields")
    );

    let mut document = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    document.trailer.set(
        "ID",
        vec![
            Object::string_literal("test"),
            Object::string_literal("test"),
        ],
    );
    let state = EncryptionState::try_from(EncryptionVersion::V2 {
        document: &document,
        owner_password: "owner",
        user_password: "user",
        permissions: Permissions::all(),
        key_length: 128,
    })
    .unwrap();
    document.encrypt(&state).unwrap();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let protected = Session::from_bytes_with_password(
        bytes.clone(),
        "Encrypted.pdf".into(),
        None,
        1,
        Some("owner"),
    )
    .unwrap();
    assert!(
        protected
            .signed_copy(identity(), PASSWORD)
            .unwrap_err()
            .to_string()
            .contains("encrypted")
    );
    assert_eq!(protected.bytes(), bytes);
}

#[test]
fn direct_nested_signature_markers_are_read_only_and_honestly_unsupported() {
    let mut document = Document::load_mem(Session::blank(72.0, 72.0, 1).unwrap().bytes()).unwrap();
    document.catalog_mut().unwrap().set(
        "Nested",
        dictionary! {
            "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
            "Contents" => Object::string_literal("not a real signature"),
        },
    );
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let mut session = open(bytes);
    assert!(
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90
            })
            .is_err()
    );
    let report = session.signature_report().unwrap();
    assert_eq!(report.signatures[0].integrity, Integrity::Unsupported);
    assert!(report.signatures[0].message.contains("Direct"));
}

#[test]
fn workspace_guards_pending_drafts_redaction_jobs_stale_revisions_and_closed_documents() {
    let mut workspace = Workspace::default();
    let first = workspace.create_document(72.0, 72.0).unwrap();
    workspace.set_draft(first.id, true).unwrap();
    assert!(
        workspace
            .signed_copy(first.id, first.revision, identity(), PASSWORD)
            .is_err()
    );
    workspace.set_draft(first.id, false).unwrap();
    let plan = workspace
        .begin_redaction(
            first.id,
            first.revision,
            150,
            &[Region {
                page: 0,
                rect: [1.0, 1.0, 10.0, 10.0],
            }],
        )
        .unwrap();
    assert!(
        workspace
            .signed_copy(first.id, first.revision, identity(), PASSWORD)
            .is_err()
    );
    workspace.cancel_redaction(plan.job_id);
    let edited = workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    assert!(
        workspace
            .signed_copy(first.id, first.revision, identity(), PASSWORD)
            .is_err()
    );
    assert!(
        workspace
            .signed_copy(first.id, edited.revision, identity(), PASSWORD)
            .is_ok()
    );
    let second = workspace.open_demo().unwrap();
    assert!(
        workspace
            .signed_copy(first.id, edited.revision, identity(), PASSWORD)
            .is_ok()
    );
    assert_eq!(workspace.current().unwrap().id, second.id);
    workspace
        .close_document(first.id, edited.revision, true)
        .unwrap();
    assert!(
        workspace
            .signed_copy(first.id, edited.revision, identity(), PASSWORD)
            .is_err()
    );
}

#[test]
fn separate_atomic_export_protects_every_tabs_source_and_saved_copy_and_keeps_unsaved_state() {
    let folder = tempfile::tempdir().unwrap();
    let first_path = folder.path().join("First.pdf");
    let second_path = folder.path().join("Second.pdf");
    let saved_path = folder.path().join("First-edited.pdf");
    let output_path = folder.path().join("First-signed.pdf");
    let original = Session::blank(72.0, 72.0, 1).unwrap().bytes().to_vec();
    std::fs::write(&first_path, &original).unwrap();
    std::fs::write(&second_path, &original).unwrap();
    let mut workspace = Workspace::default();
    let first = workspace.open_path(first_path.clone()).unwrap();
    workspace.open_path(second_path.clone()).unwrap();
    workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .save_as(&saved_path)
        .unwrap();
    let edited = workspace
        .current_mut(first.id, first.revision)
        .unwrap()
        .apply(Edit::Rotate {
            page: 0,
            degrees: 90,
        })
        .unwrap();
    let before = workspace.document(first.id).unwrap().bytes().to_vec();
    for path in [&first_path, &second_path, &saved_path] {
        assert!(
            workspace
                .export_signed_copy(first.id, edited.revision, identity(), PASSWORD, path)
                .is_err()
        );
    }
    #[cfg(unix)]
    {
        let alias = folder.path().join("Alias.pdf");
        std::os::unix::fs::symlink(&first_path, &alias).unwrap();
        assert!(
            workspace
                .export_signed_copy(first.id, edited.revision, identity(), PASSWORD, &alias)
                .is_err()
        );
    }
    workspace
        .export_signed_copy(
            first.id,
            edited.revision,
            identity(),
            PASSWORD,
            &output_path,
        )
        .unwrap();
    assert_eq!(std::fs::read(&first_path).unwrap(), original);
    assert_eq!(std::fs::read(&second_path).unwrap(), original);
    assert_eq!(workspace.document(first.id).unwrap().bytes(), before);
    assert!(workspace.document(first.id).unwrap().is_dirty());
    let reopened = Session::from_path(output_path, 99).unwrap();
    assert_eq!(reopened.info().unwrap().pages[0].rotation, 90);
    assert_eq!(
        reopened.signature_report().unwrap().signatures[0].integrity,
        Integrity::Intact
    );
}

#[test]
fn runtime_identity_files_are_bounded_and_not_kept_by_the_workspace() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("Runtime-only.PFX");
    std::fs::write(&path, identity()).unwrap();
    assert_eq!(read_identity_file(&path).unwrap().as_slice(), identity());
    assert!(read_identity_file(&folder.path().join("not-pfx.pdf")).is_err());
    std::fs::write(&path, []).unwrap();
    assert!(read_identity_file(&path).is_err());
}

#[test]
fn cms_version_mismatches_and_missing_signer_certificates_are_not_accepted() {
    use cms::{
        content_info::{CmsVersion, ContentInfo},
        signed_data::SignedData,
    };
    use der::{Decode, Encode};
    for malformed_version in [true, false] {
        let mut bytes = signed();
        let mut info = ContentInfo::from_der(&cms_bytes(&bytes)).unwrap();
        let mut data: SignedData = info.content.decode_as().unwrap();
        if malformed_version {
            data.version = CmsVersion::V3;
        } else {
            data.certificates = None;
        }
        info.content = der::Any::from_der(&data.to_der().unwrap()).unwrap();
        replace_cms(&mut bytes, &info.to_der().unwrap());
        let report = open(bytes).signature_report().unwrap();
        assert_eq!(
            report.signatures[0].integrity,
            if malformed_version {
                Integrity::Invalid
            } else {
                Integrity::Unsupported
            },
        );
    }
}

#[test]
fn existing_sha384_and_sha512_detached_cms_are_verified_with_real_signatures() {
    use cms::{content_info::ContentInfo, signed_data::SignedData};
    use der::{
        Decode, Encode,
        asn1::{ObjectIdentifier, OctetString, SetOfVec},
    };
    use openssl::{hash::hash, sign::Signer};
    let identity = Pkcs12::from_der(identity())
        .unwrap()
        .parse2(PASSWORD)
        .unwrap();
    let key = identity.pkey.unwrap();
    for (digest, oid, cades) in [
        (MessageDigest::sha384(), "2.16.840.1.101.3.4.2.2", false),
        (MessageDigest::sha512(), "2.16.840.1.101.3.4.2.3", true),
    ] {
        let mut bytes = signed();
        if cades {
            let original = b"adbe.pkcs7.detached";
            let index = bytes
                .windows(original.len())
                .position(|value| value == original)
                .unwrap();
            bytes[index..index + original.len()].copy_from_slice(b"ETSI.CAdES.detached");
        }
        let [_, first, second, length] = ranges(&bytes);
        let mut content = bytes[..first].to_vec();
        content.extend_from_slice(&bytes[second..second + length]);
        let mut info = ContentInfo::from_der(&cms_bytes(&bytes)).unwrap();
        let mut data: SignedData = info.content.decode_as().unwrap();
        let mut signer = data.signer_infos.0.iter().next().unwrap().clone();
        signer.digest_alg.oid = ObjectIdentifier::new(oid).unwrap();
        let mut attributes: Vec<_> = signer
            .signed_attrs
            .as_ref()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        let message_digest = attributes
            .iter_mut()
            .find(|attribute| attribute.oid == ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4"))
            .unwrap();
        let digest_value = OctetString::new(hash(digest, &content).unwrap().to_vec()).unwrap();
        message_digest.values = SetOfVec::try_from(vec![
            der::Any::from_der(&digest_value.to_der().unwrap()).unwrap(),
        ])
        .unwrap();
        let attributes = SetOfVec::try_from(attributes).unwrap();
        let mut signing = Signer::new(digest, &key).unwrap();
        signing.update(&attributes.to_der().unwrap()).unwrap();
        signer.signature = OctetString::new(signing.sign_to_vec().unwrap()).unwrap();
        signer.signed_attrs = Some(attributes);
        data.digest_algorithms = SetOfVec::try_from(vec![signer.digest_alg.clone()]).unwrap();
        data.signer_infos.0 = SetOfVec::try_from(vec![signer]).unwrap();
        info.content = der::Any::from_der(&data.to_der().unwrap()).unwrap();
        replace_cms(&mut bytes, &info.to_der().unwrap());
        let report = open(bytes).signature_report().unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Intact);
        assert_eq!(
            report.signatures[0].digest.as_deref(),
            Some(if cades { "SHA-512" } else { "SHA-384" })
        );
    }
}

#[test]
fn unsigned_normal_acroforms_keep_non_signature_field_values_in_the_signed_copy() {
    let original = Session::blank(72.0, 72.0, 1).unwrap();
    let mut document = Document::load_mem(original.bytes()).unwrap();
    let field = document.add_object(dictionary! {
        "FT" => "Tx",
        "T" => Object::string_literal("Existing form text"),
        "V" => Object::string_literal("Preserved value"),
    });
    let form = document.add_object(dictionary! { "Fields" => vec![Object::Reference(field)] });
    document.catalog_mut().unwrap().set("AcroForm", form);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    let session = open(bytes);
    let signed = session.signed_copy(identity(), PASSWORD).unwrap();
    let output = Document::load_mem(&signed).unwrap();
    assert_eq!(
        output
            .get_dictionary(field)
            .unwrap()
            .get(b"V")
            .unwrap()
            .as_str()
            .unwrap(),
        b"Preserved value",
    );
    assert_eq!(
        open(signed).signature_report().unwrap().signatures[0].integrity,
        Integrity::Intact
    );
}
