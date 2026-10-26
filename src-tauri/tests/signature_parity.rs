use std::{path::Path, process::Command, sync::OnceLock};

use folio_core::{
    document::{Edit, Session},
    signatures::{
        CertificateValidity, Coverage, Integrity, MAX_ANCHOR_BYTES, TrustStore, read_anchor_file,
    },
    workspace::{OpenOutcome, Workspace},
};
use lopdf::{Document, IncrementalDocument, Object, ObjectId, Stream, dictionary, xref::XrefType};
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
    stack::Stack,
    x509::{
        X509, X509NameBuilder,
        extension::{
            AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage,
            SubjectKeyIdentifier,
        },
    },
};

const PASSWORD: &str = "runtime-only signature parity password";

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

#[test]
fn the_custom_store_accepts_exactly_32_anchors_and_never_silently_drops_or_persists_any() {
    let mut trust = TrustStore::default();
    for index in 0..32 {
        let key = key();
        let ca = certificate(
            &format!("Runtime CA {index}"),
            &key,
            None,
            true,
            Variant::Normal,
        );
        let store = trust.add(index, &ca.to_der().unwrap()).unwrap();
        assert_eq!(store.anchors.len(), index as usize + 1);
        assert!(!store.persistent);
    }
    let before = serde_json::to_value(trust.info().unwrap()).unwrap();
    let key = key();
    let extra = certificate("Runtime CA 33", &key, None, true, Variant::Normal);
    assert!(
        trust
            .add(32, &extra.to_der().unwrap())
            .unwrap_err()
            .to_string()
            .contains("32 anchors")
    );
    assert_eq!(serde_json::to_value(trust.info().unwrap()).unwrap(), before);
}

fn key() -> PKey<Private> {
    PKey::from_ec_key(
        EcKey::generate(&EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap()).unwrap(),
    )
    .unwrap()
}

#[derive(Clone, Copy, Default)]
enum Variant {
    #[default]
    Normal,
    Expired,
    Future,
    NotCa,
    WrongCaUsage,
    TlsOnly,
    PathLengthZero,
    Sha1,
}

fn certificate(
    name: &str,
    key: &PKey<Private>,
    issuer: Option<(&X509, &PKey<Private>)>,
    ca: bool,
    variant: Variant,
) -> X509 {
    let mut subject = X509NameBuilder::new().unwrap();
    subject.append_entry_by_text("CN", name).unwrap();
    let subject = subject.build();
    let mut builder = X509::builder().unwrap();
    builder.set_version(2).unwrap();
    let mut serial = BigNum::new().unwrap();
    serial
        .rand(96, openssl::bn::MsbOption::MAYBE_ZERO, false)
        .unwrap();
    builder
        .set_serial_number(&Asn1Integer::from_bn(&serial).unwrap())
        .unwrap();
    builder.set_subject_name(&subject).unwrap();
    builder
        .set_issuer_name(issuer.map_or(subject.as_ref(), |(cert, _)| cert.subject_name()))
        .unwrap();
    builder.set_pubkey(key).unwrap();
    let (before, after) = match variant {
        Variant::Expired => (now() - 86400, now() - 3600),
        Variant::Future => (now() + 3600, now() + 86400),
        _ => (now() - 3600, now() + 86400),
    };
    builder
        .set_not_before(&Asn1Time::from_unix(before).unwrap())
        .unwrap();
    builder
        .set_not_after(&Asn1Time::from_unix(after).unwrap())
        .unwrap();
    let mut constraints = BasicConstraints::new();
    constraints.critical();
    if ca && !matches!(variant, Variant::NotCa) {
        constraints.ca();
        if matches!(variant, Variant::PathLengthZero) {
            constraints.pathlen(0);
        }
    }
    builder
        .append_extension(constraints.build().unwrap())
        .unwrap();
    let mut usage = KeyUsage::new();
    usage.critical();
    if ca && !matches!(variant, Variant::WrongCaUsage) {
        usage.key_cert_sign().crl_sign();
    } else if matches!(variant, Variant::WrongCaUsage) {
        usage.key_encipherment();
    } else {
        usage.digital_signature().non_repudiation();
    }
    builder.append_extension(usage.build().unwrap()).unwrap();
    if matches!(variant, Variant::TlsOnly) {
        builder
            .append_extension(ExtendedKeyUsage::new().server_auth().build().unwrap())
            .unwrap();
    } else if !ca {
        builder
            .append_extension(
                ExtendedKeyUsage::new()
                    .other("1.3.6.1.5.5.7.3.36")
                    .build()
                    .unwrap(),
            )
            .unwrap();
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
        .sign(
            issuer.map_or(key, |(_, key)| key),
            if matches!(variant, Variant::Sha1) {
                MessageDigest::sha1()
            } else {
                MessageDigest::sha256()
            },
        )
        .unwrap();
    builder.build()
}

fn pfx(key: &PKey<Private>, certificate: &X509, chain: &[X509]) -> Vec<u8> {
    let mut certificates = Stack::new().unwrap();
    for certificate in chain {
        certificates.push(certificate.clone()).unwrap();
    }
    Pkcs12::builder()
        .name("Runtime test only")
        .pkey(key)
        .cert(certificate)
        .ca(certificates)
        .build2(PASSWORD)
        .unwrap()
        .to_der()
        .unwrap()
}

fn identity() -> &'static [u8] {
    static IDENTITY: OnceLock<Vec<u8>> = OnceLock::new();
    IDENTITY.get_or_init(|| {
        let key = key();
        pfx(
            &key,
            &certificate(
                "Runtime approval signer",
                &key,
                None,
                false,
                Variant::Normal,
            ),
            &[],
        )
    })
}

fn open(bytes: Vec<u8>) -> Session {
    Session::from_bytes(bytes, "Runtime approval.pdf".into(), None, 1).unwrap()
}

fn save(mut document: Document) -> Vec<u8> {
    let mut bytes = vec![];
    document.save_to(&mut bytes).unwrap();
    bytes
}

struct Slots {
    bytes: Vec<u8>,
    fields: Vec<ObjectId>,
    widgets: Vec<ObjectId>,
    text: ObjectId,
    attachment: ObjectId,
}

fn slots(rotated: bool, merged: bool, count: usize, table: bool) -> Slots {
    let mut document =
        Document::load_mem(Session::blank(500.0, 720.0, 1).unwrap().bytes()).unwrap();
    if table {
        document.reference_table.cross_reference_type = XrefType::CrossReferenceTable;
    }
    let page = document.get_pages()[&1];
    if rotated {
        let page = document.get_dictionary_mut(page).unwrap();
        page.set(
            "MediaBox",
            vec![(-50).into(), 20.into(), 450.into(), 720.into()],
        );
        page.set(
            "CropBox",
            vec![(-30).into(), 70.into(), 390.into(), 610.into()],
        );
        page.set("Rotate", 90);
        page.set("UserUnit", 1.5);
    }
    let original_ap = document.add_object(Stream::new(dictionary! {
        "Type" => "XObject", "Subtype" => "Form", "BBox" => vec![0.into(), 0.into(), 260.into(), 70.into()],
        "Resources" => dictionary! { "Font" => dictionary! { "F0" => dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" } } },
    }, b"BT /F0 10 Tf 4 25 Td (Awaiting local approval) Tj ET".to_vec()));
    let text = document.add_object(dictionary! {
        "FT" => "Tx", "Subtype" => "Widget", "Type" => "Annot",
        "T" => Object::string_literal("Preserved text"), "V" => Object::string_literal("Original form value"),
        "P" => page, "Rect" => vec![50.into(), 300.into(), 250.into(), 340.into()], "AP" => dictionary! { "N" => original_ap },
    });
    let mut fields = vec![];
    let mut widgets = vec![];
    for index in 0..count {
        let field = document.add_object(dictionary! { "FT" => "Sig", "T" => Object::string_literal(format!("Approval {}", index + 1)), "Ff" => 0 });
        let widget = if merged {
            field
        } else {
            document.add_object(dictionary! { "Parent" => field })
        };
        let dictionary = document.get_dictionary_mut(widget).unwrap();
        dictionary.set("Type", "Annot");
        dictionary.set("Subtype", "Widget");
        dictionary.set("P", page);
        dictionary.set("Rect", vec![40.into(), 130.into(), 300.into(), 200.into()]);
        dictionary.set("F", 4);
        dictionary.set("AP", dictionary! { "N" => original_ap });
        if !merged {
            document
                .get_dictionary_mut(field)
                .unwrap()
                .set("Kids", vec![Object::Reference(widget)]);
        }
        fields.push(field);
        widgets.push(widget);
    }
    let roots = document.add_object(Object::Array(
        std::iter::once(text)
            .chain(fields.iter().copied())
            .map(Object::Reference)
            .collect(),
    ));
    let form = document
        .add_object(dictionary! { "Fields" => roots, "SigFlags" => 0, "NeedAppearances" => true });
    document.catalog_mut().unwrap().set("AcroForm", form);
    document.get_dictionary_mut(page).unwrap().set(
        "Annots",
        std::iter::once(text)
            .chain(widgets.iter().copied())
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    let attachment = document.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"Original workflow attachment".to_vec(),
    ));
    let specification = document.add_object(dictionary! {
        "Type" => "Filespec", "F" => Object::string_literal("workflow.txt"), "EF" => dictionary! { "F" => attachment },
    });
    document.catalog_mut().unwrap().set("Names", dictionary! { "EmbeddedFiles" => dictionary! {
        "Names" => vec![Object::string_literal("workflow.txt"), Object::Reference(specification)],
    } });
    Slots {
        bytes: save(document),
        fields,
        widgets,
        text,
        attachment,
    }
}

fn id(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}

fn signatures(document: &Document) -> Vec<&lopdf::Dictionary> {
    document
        .objects
        .values()
        .filter_map(|object| object.as_dict().ok())
        .filter(|dictionary| dictionary.get(b"Type").and_then(Object::as_name).ok() == Some(b"Sig"))
        .collect()
}

fn byte_range(dictionary: &lopdf::Dictionary) -> [usize; 4] {
    let values = dictionary.get(b"ByteRange").unwrap().as_array().unwrap();
    std::array::from_fn(|index| values[index].as_i64().unwrap() as usize)
}

#[test]
fn selected_merged_and_separate_widgets_keep_identity_geometry_other_fields_and_a_real_appearance()
{
    for (rotated, merged, table) in [(false, true, true), (true, false, false)] {
        let fixture = slots(rotated, merged, 2, table);
        let mut source = open(fixture.bytes.clone());
        assert!(source.info().unwrap().read_only_reason.is_none());
        assert!(source.signature_report().unwrap().signatures.is_empty());
        source
            .apply(Edit::Note {
                page: 0,
                point: [60.0, 160.0],
                text: "Included unsaved review note".into(),
            })
            .unwrap();
        let original = source.bytes().to_vec();
        let state = serde_json::to_value(source.info().unwrap()).unwrap();
        let before = Document::load_mem(&original).unwrap();
        let bytes = source
            .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
            .unwrap();
        assert!(bytes.starts_with(&original));
        assert_eq!(source.bytes(), original);
        assert_eq!(serde_json::to_value(source.info().unwrap()).unwrap(), state);
        let after = Document::load_mem(&bytes).unwrap();
        assert_eq!(
            after.get_object(fixture.text).unwrap(),
            before.get_object(fixture.text).unwrap()
        );
        assert_eq!(
            after.get_object(fixture.attachment).unwrap(),
            before.get_object(fixture.attachment).unwrap()
        );
        assert_eq!(
            after.get_object(fixture.fields[1]).unwrap(),
            before.get_object(fixture.fields[1]).unwrap()
        );
        for name in [b"T".as_slice(), b"Kids", b"FT"] {
            assert_eq!(
                after
                    .get_dictionary(fixture.fields[0])
                    .unwrap()
                    .get(name)
                    .ok(),
                before
                    .get_dictionary(fixture.fields[0])
                    .unwrap()
                    .get(name)
                    .ok()
            );
        }
        let widget = after.get_dictionary(fixture.widgets[0]).unwrap();
        for name in [b"Rect".as_slice(), b"P", b"Parent"] {
            assert_eq!(
                widget.get(name).ok(),
                before
                    .get_dictionary(fixture.widgets[0])
                    .unwrap()
                    .get(name)
                    .ok()
            );
        }
        let appearance = widget
            .get(b"AP")
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"N")
            .unwrap()
            .as_reference()
            .unwrap();
        let appearance = after.get_object(appearance).unwrap().as_stream().unwrap();
        assert!(
            !appearance
                .content
                .windows(8)
                .any(|value| value == b"Awaiting")
        );
        assert!(appearance.dict.has(b"Resources"));
        assert!(!appearance.content.is_empty());
        let form = after
            .dereference(after.catalog().unwrap().get(b"AcroForm").unwrap())
            .unwrap()
            .1
            .as_dict()
            .unwrap();
        assert_eq!(form.get(b"SigFlags").unwrap().as_i64().unwrap(), 3);
        assert!(form.get(b"NeedAppearances").unwrap().as_bool().unwrap());
        let report = open(bytes).signature_report().unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Intact);
        assert_eq!(report.signatures[0].coverage, Coverage::WholeDocument);
        assert!(
            report.signatures[0]
                .claimed_signing_time
                .as_ref()
                .unwrap()
                .starts_with("D:")
        );
        assert_eq!(report.signatures[0].revocation, "not_checked");
        assert_eq!(report.signatures[0].trusted_time, "not_checked");
        assert_eq!(report.empty_fields[0].id, id(fixture.fields[1]));
        source.undo().unwrap();
        assert!(source.info().unwrap().annotations.is_empty());
    }
}

fn successive() -> Vec<Vec<u8>> {
    let fixture = slots(true, false, 2, false);
    let first = open(fixture.bytes.clone())
        .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
        .unwrap();
    let second = open(first.clone())
        .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[1])))
        .unwrap();
    let third = open(second.clone())
        .signed_copy(identity(), PASSWORD)
        .unwrap();
    vec![fixture.bytes, first, second, third]
}

#[test]
fn successive_approvals_are_exact_prefixes_and_all_prior_signatures_and_appearances_survive() {
    let revisions = successive();
    for pair in revisions.windows(2) {
        assert!(pair[1].starts_with(&pair[0]));
    }
    for (index, bytes) in revisions.iter().enumerate().skip(1) {
        let mut session = open(bytes.clone());
        let report = session.signature_report().unwrap();
        assert!(report.signing_blocked_reason.is_none());
        assert_eq!(report.signatures.len(), index);
        for signature in &report.signatures {
            assert_eq!(signature.integrity, Integrity::Intact);
            assert_eq!(
                signature.coverage,
                if signature.signed_bytes == Some(bytes.len()) {
                    Coverage::WholeDocument
                } else {
                    Coverage::LaterChanges
                }
            );
        }
        assert!(
            session
                .apply(Edit::Rotate {
                    page: 0,
                    degrees: 90
                })
                .is_err()
        );
        assert!(session.extract_pages(&[0]).is_err());
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("unchanged-signed.pdf");
        session.save_as(&path).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
    }
    let first = Document::load_mem(&revisions[1]).unwrap();
    let last = Document::load_mem(&revisions[3]).unwrap();
    for (object_id, object) in &first.objects {
        if object
            .as_dict()
            .is_ok_and(|dict| dict.get(b"Type").and_then(Object::as_name).ok() == Some(b"Sig"))
        {
            assert_eq!(last.get_object(*object_id).unwrap(), object);
            match (
                last.reference_table.entries.get(&object_id.0),
                first.reference_table.entries.get(&object_id.0),
            ) {
                (
                    Some(lopdf::xref::XrefEntry::Normal {
                        offset: after,
                        generation: after_generation,
                    }),
                    Some(lopdf::xref::XrefEntry::Normal {
                        offset: before,
                        generation: before_generation,
                    }),
                ) => assert_eq!((after, after_generation), (before, before_generation)),
                _ => panic!("An earlier signature no longer has its original ordinary xref entry"),
            }
        }
        if object.as_stream().is_ok_and(|stream| {
            stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form")
        }) && first
            .objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .any(|dict| {
                dict.get(b"AP")
                    .and_then(Object::as_dict)
                    .and_then(|ap| ap.get(b"N"))
                    .and_then(Object::as_reference)
                    .ok()
                    == Some(*object_id)
            })
        {
            assert_eq!(last.get_object(*object_id).unwrap(), object);
        }
    }
}

#[test]
#[ignore = "requires an independent OpenSSL command-line executable"]
fn successive_pdfs_verify_every_cms_with_independent_openssl_cli() {
    let folder = tempfile::tempdir().unwrap();
    for (revision, bytes) in successive().iter().enumerate().skip(1) {
        let document = Document::load_mem(bytes).unwrap();
        for (index, signature) in signatures(&document).iter().enumerate() {
            let [_, first, second, length] = byte_range(signature);
            let mut data = bytes[..first].to_vec();
            data.extend_from_slice(&bytes[second..second + length]);
            let cms =
                CmsContentInfo::from_der(signature.get(b"Contents").unwrap().as_str().unwrap())
                    .unwrap()
                    .to_der()
                    .unwrap();
            let data_path = folder.path().join(format!("{revision}-{index}.data"));
            let cms_path = folder.path().join(format!("{revision}-{index}.cms"));
            let verified = folder.path().join(format!("{revision}-{index}.verified"));
            std::fs::write(&data_path, &data).unwrap();
            std::fs::write(&cms_path, cms).unwrap();
            let output = Command::new("openssl")
                .args([
                    "cms",
                    "-verify",
                    "-binary",
                    "-inform",
                    "DER",
                    "-noverify",
                    "-in",
                ])
                .arg(cms_path)
                .arg("-content")
                .arg(data_path)
                .arg("-out")
                .arg(&verified)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "Independent OpenSSL verification failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(std::fs::read(verified).unwrap(), data);
        }
    }
}

#[test]
fn stale_wrong_locked_ambiguous_or_already_filled_slots_and_unsigned_appends_never_export() {
    let fixture = slots(false, false, 2, true);
    let source = open(fixture.bytes.clone());
    for field in [
        id(fixture.widgets[0]),
        id(fixture.text),
        "Approval 1".into(),
        "999999:0".into(),
        "01:0".into(),
    ] {
        assert!(
            source
                .signed_copy_in_field(identity(), PASSWORD, Some(&field))
                .is_err()
        );
    }
    assert_eq!(source.bytes(), fixture.bytes);
    let first = source
        .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
        .unwrap();
    assert!(
        open(first.clone())
            .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
            .is_err()
    );
    let mut trailing = first.clone();
    trailing.extend_from_slice(b"\n% unsigned comment\n");
    assert!(
        open(trailing.clone())
            .signed_copy(identity(), PASSWORD)
            .is_err()
    );
    assert_eq!(
        open(trailing).signature_report().unwrap().signatures[0].coverage,
        Coverage::LaterChanges
    );
    let mut increment =
        IncrementalDocument::create_from(first.clone(), Document::load_mem(&first).unwrap());
    increment
        .new_document
        .add_object(Object::string_literal("Unsigned incremental changes"));
    let mut bytes = vec![];
    increment.save_to(&mut bytes).unwrap();
    let unchanged = bytes.clone();
    let session = open(bytes);
    assert!(
        session
            .signed_copy(identity(), PASSWORD)
            .unwrap_err()
            .to_string()
            .contains("entire current file")
    );
    assert_eq!(session.bytes(), unchanged);

    for case in 0..6 {
        let mut document = Document::load_mem(&fixture.bytes).unwrap();
        match case {
            0 => {
                document
                    .get_dictionary_mut(fixture.fields[0])
                    .unwrap()
                    .set("Ff", 1);
            }
            1 => {
                document
                    .get_dictionary_mut(fixture.fields[0])
                    .unwrap()
                    .set("Lock", dictionary! { "Action" => "All" });
            }
            2 => {
                document
                    .get_dictionary_mut(fixture.fields[0])
                    .unwrap()
                    .set("SV", dictionary! { "Ff" => 1 });
            }
            3 => {
                document
                    .get_dictionary_mut(fixture.widgets[0])
                    .unwrap()
                    .set("Parent", fixture.fields[1]);
            }
            4 => {
                document
                    .get_dictionary_mut(fixture.widgets[0])
                    .unwrap()
                    .set("V", Object::Null);
            }
            5 => {
                let form = document
                    .catalog()
                    .unwrap()
                    .get(b"AcroForm")
                    .unwrap()
                    .as_reference()
                    .unwrap();
                document.get_dictionary_mut(form).unwrap().set(
                    "Fields",
                    vec![
                        Object::Reference(fixture.fields[0]),
                        Object::Reference(fixture.fields[0]),
                    ],
                );
            }
            _ => unreachable!(),
        }
        let bytes = save(document);
        let session = open(bytes.clone());
        assert!(
            session
                .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
                .is_err(),
            "case {case}"
        );
        assert_eq!(session.bytes(), bytes);
    }
}

#[test]
fn visible_widget_physical_thresholds_and_geometry_are_enforced_before_any_output() {
    let fixture = slots(false, false, 1, true);
    for (width, height, valid) in [
        (72.0, 32.0, true),
        (71.99, 32.0, false),
        (72.0, 31.99, false),
        (0.0, 0.0, true),
        (0.0, 32.0, false),
    ] {
        let mut document = Document::load_mem(&fixture.bytes).unwrap();
        document
            .get_dictionary_mut(fixture.widgets[0])
            .unwrap()
            .set(
                "Rect",
                vec![
                    40.0.into(),
                    130.0.into(),
                    (40.0 + width).into(),
                    (130.0 + height).into(),
                ],
            );
        let bytes = save(document);
        let session = open(bytes.clone());
        assert_eq!(
            session
                .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
                .is_ok(),
            valid,
            "{width} x {height}"
        );
        assert_eq!(session.bytes(), bytes);
    }
    let fixture = slots(false, false, 65, false);
    let source = open(fixture.bytes.clone());
    assert!(
        source
            .signed_copy(identity(), PASSWORD)
            .unwrap_err()
            .to_string()
            .contains("64 signature fields")
    );
    assert_eq!(source.bytes(), fixture.bytes);
    let fixture = slots(false, false, 64, true);
    let source = open(fixture.bytes.clone());
    assert!(
        source
            .signed_copy(identity(), PASSWORD)
            .unwrap_err()
            .to_string()
            .contains("Select a supported existing empty field")
    );
    assert!(
        source
            .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
            .is_ok()
    );
}

#[test]
fn an_existing_cms_cannot_be_rebound_to_another_or_multiple_logical_fields() {
    let fixture = slots(false, false, 2, true);
    let bytes = open(fixture.bytes)
        .signed_copy_in_field(identity(), PASSWORD, Some(&id(fixture.fields[0])))
        .unwrap();
    for duplicate in [false, true] {
        let document = Document::load_mem(&bytes).unwrap();
        let signature = document
            .get_dictionary(fixture.fields[0])
            .unwrap()
            .get(b"V")
            .unwrap()
            .clone();
        let mut incremental = IncrementalDocument::create_from(bytes.clone(), document);
        incremental
            .opt_clone_object_to_new_document(fixture.fields[1])
            .unwrap();
        incremental
            .new_document
            .get_dictionary_mut(fixture.fields[1])
            .unwrap()
            .set("V", signature);
        if !duplicate {
            incremental
                .opt_clone_object_to_new_document(fixture.fields[0])
                .unwrap();
            incremental
                .new_document
                .get_dictionary_mut(fixture.fields[0])
                .unwrap()
                .set("V", Object::Null);
        }
        let mut changed = vec![];
        incremental.save_to(&mut changed).unwrap();
        let session = open(changed.clone());
        let report = session.signature_report().unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Invalid);
        assert!(report.signing_blocked_reason.is_some());
        assert!(session.signed_copy(identity(), PASSWORD).is_err());
        assert_eq!(session.bytes(), changed);
    }
}

struct Chain {
    root: X509,
    intermediate: X509,
    leaf: X509,
    key: PKey<Private>,
}

impl Chain {
    fn new(
        root_variant: Variant,
        intermediate_variant: Variant,
        leaf_variant: Variant,
        weak_intermediate: bool,
    ) -> Self {
        let root_key = key();
        let root = certificate("Runtime custom CA", &root_key, None, true, root_variant);
        let intermediate_key = if weak_intermediate {
            PKey::from_rsa(Rsa::generate(1024).unwrap()).unwrap()
        } else {
            key()
        };
        let intermediate = certificate(
            "Runtime intermediate",
            &intermediate_key,
            Some((&root, &root_key)),
            true,
            intermediate_variant,
        );
        let key = key();
        let leaf = certificate(
            "Runtime document signer",
            &key,
            Some((&intermediate, &intermediate_key)),
            false,
            leaf_variant,
        );
        Self {
            root,
            intermediate,
            leaf,
            key,
        }
    }

    fn pdf(&self, intermediate: bool, root: bool) -> Vec<u8> {
        let chain: Vec<_> = [(intermediate, &self.intermediate), (root, &self.root)]
            .into_iter()
            .filter_map(|(include, certificate)| include.then_some(certificate.clone()))
            .collect();
        open(Session::blank(100.0, 100.0, 1).unwrap().bytes().to_vec())
            .signed_copy(&pfx(&self.key, &self.leaf, &chain), PASSWORD)
            .unwrap()
    }
}

#[test]
fn explicit_custom_root_with_embedded_intermediate_is_never_system_or_embedded_signer_trust() {
    let chain = Chain::new(Variant::Normal, Variant::Normal, Variant::Normal, false);
    let bytes = chain.pdf(true, true);
    let session = open(bytes.clone());
    let default = session.signature_report().unwrap();
    assert_eq!(
        default.signatures[0]
            .certificate
            .as_ref()
            .unwrap()
            .identity_trust,
        "not_checked"
    );
    assert!(default.trust_store.anchors.is_empty());
    assert!(!default.trust_store.persistent);
    let mut trust = TrustStore::default();
    trust.add(0, &chain.root.to_der().unwrap()).unwrap();
    let report = session.signature_report_with_trust(&trust).unwrap();
    let signature = &report.signatures[0];
    assert_eq!(signature.integrity, Integrity::Intact);
    assert_eq!(signature.coverage, Coverage::WholeDocument);
    assert_eq!(
        signature.chain.status, "anchored_custom_store",
        "{}",
        signature.chain.message
    );
    assert_eq!(signature.chain.chain_fingerprints.len(), 3);
    assert_eq!(
        signature.chain.anchor_fingerprint.as_ref().unwrap(),
        &report.trust_store.anchors[0].sha256_fingerprint
    );
    assert_eq!(signature.revocation, "not_checked");
    assert_eq!(signature.trusted_time, "not_checked");
    assert!(
        signature
            .chain
            .message
            .contains("not Adobe AATL/EUTL or OS trust")
    );
    assert_eq!(session.bytes(), bytes);
    let absent = open(chain.pdf(false, false))
        .signature_report_with_trust(&trust)
        .unwrap();
    assert_eq!(absent.signatures[0].chain.status, "untrusted");
    assert_eq!(absent.signatures[0].integrity, Integrity::Intact);
    let other_key = key();
    let other_root = certificate(
        "Unrelated runtime CA",
        &other_key,
        None,
        true,
        Variant::Normal,
    );
    let mut other = TrustStore::default();
    other.add(0, &other_root.to_pem().unwrap()).unwrap();
    assert_eq!(
        session
            .signature_report_with_trust(&other)
            .unwrap()
            .signatures[0]
            .chain
            .status,
        "untrusted"
    );
    let json = serde_json::to_value(report).unwrap();
    assert_eq!(
        json.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "documentId",
            "emptyFields",
            "revision",
            "signatures",
            "signingBlockedReason",
            "trustStore",
            "warnings"
        ]
    );
    assert_eq!(
        json["trustStore"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["anchors", "persistent", "revision"]
    );
    assert_eq!(
        json["signatures"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "algorithm",
            "certificate",
            "chain",
            "claimedSigningTime",
            "coverage",
            "currentBytes",
            "digest",
            "id",
            "integrity",
            "message",
            "revocation",
            "signedBytes",
            "trustedTime"
        ]
    );
    assert_eq!(
        json["signatures"][0]["certificate"]["identityTrust"],
        "anchored_custom_store"
    );
    assert_eq!(json["signatures"][0]["revocation"], "not_checked");
    assert_eq!(json["signatures"][0]["trustedTime"], "not_checked");
}

#[test]
fn expiry_future_weak_ca_constraints_usage_and_path_length_fail_separately_from_cms_integrity() {
    for (root, intermediate, weak) in [
        (Variant::Expired, Variant::Normal, false),
        (Variant::Future, Variant::Normal, false),
        (Variant::Normal, Variant::Expired, false),
        (Variant::Normal, Variant::Future, false),
        (Variant::Normal, Variant::NotCa, false),
        (Variant::Normal, Variant::WrongCaUsage, false),
        (Variant::PathLengthZero, Variant::Normal, false),
        (Variant::Normal, Variant::Normal, true),
        (Variant::Normal, Variant::TlsOnly, false),
    ] {
        let chain = Chain::new(root, intermediate, Variant::Normal, weak);
        let mut trust = TrustStore::default();
        trust.add(0, &chain.root.to_pem().unwrap()).unwrap();
        let report = open(chain.pdf(true, false))
            .signature_report_with_trust(&trust)
            .unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Intact);
        assert_eq!(
            report.signatures[0].certificate.as_ref().unwrap().validity,
            CertificateValidity::ValidNow
        );
        assert_eq!(report.signatures[0].chain.status, "untrusted");
        assert!(report.signatures[0].chain.anchor_fingerprint.is_none());
    }
}

fn replace_cms(mut bytes: Vec<u8>, chain: &Chain) -> Vec<u8> {
    let document = Document::load_mem(&bytes).unwrap();
    let [_, first, second, length] = byte_range(signatures(&document)[0]);
    let mut data = bytes[..first].to_vec();
    data.extend_from_slice(&bytes[second..second + length]);
    let mut embedded = Stack::new().unwrap();
    embedded.push(chain.intermediate.clone()).unwrap();
    let cms = CmsContentInfo::sign(
        Some(&chain.leaf),
        Some(&chain.key),
        Some(&embedded),
        Some(&data),
        CMSOptions::BINARY | CMSOptions::DETACHED | CMSOptions::NOSMIMECAP,
    )
    .unwrap()
    .to_der()
    .unwrap();
    assert!(cms.len() * 2 < second - first - 2);
    bytes[first + 1..second - 1].fill(b'0');
    for (index, byte) in cms.iter().enumerate() {
        bytes[first + 1 + index * 2..first + 3 + index * 2]
            .copy_from_slice(format!("{byte:02X}").as_bytes());
    }
    bytes
}

#[test]
fn leaf_current_validity_and_document_purpose_never_become_historical_or_timestamp_assurance() {
    let blank = Session::blank(100.0, 100.0, 1).unwrap();
    let signed = blank.signed_copy(identity(), PASSWORD).unwrap();
    for (variant, validity) in [
        (Variant::Expired, CertificateValidity::Expired),
        (Variant::Future, CertificateValidity::NotYetValid),
        (Variant::TlsOnly, CertificateValidity::ValidNow),
    ] {
        let chain = Chain::new(Variant::Normal, Variant::Normal, variant, false);
        assert!(
            blank
                .signed_copy(
                    &pfx(&chain.key, &chain.leaf, &[chain.intermediate.clone()]),
                    PASSWORD
                )
                .is_err()
        );
        let mut trust = TrustStore::default();
        trust.add(0, &chain.root.to_der().unwrap()).unwrap();
        let report = open(replace_cms(signed.clone(), &chain))
            .signature_report_with_trust(&trust)
            .unwrap();
        assert_eq!(report.signatures[0].integrity, Integrity::Intact);
        assert_eq!(report.signatures[0].chain.status, "untrusted");
        assert_eq!(
            report.signatures[0].certificate.as_ref().unwrap().validity,
            validity
        );
        assert_eq!(report.signatures[0].trusted_time, "not_checked");
    }
}

#[test]
fn custom_store_rejects_private_keys_leafs_bundles_trailing_der_weak_and_oversized_material() {
    let chain = Chain::new(Variant::Normal, Variant::Normal, Variant::Normal, false);
    let private = chain.key.private_key_to_pem_pkcs8().unwrap();
    let mut bundle = chain.root.to_pem().unwrap();
    bundle.extend_from_slice(&private);
    let mut trailing = chain.root.to_der().unwrap();
    trailing.push(0);
    let weak = PKey::from_rsa(Rsa::generate(1024).unwrap()).unwrap();
    let weak_ca = certificate("Weak runtime root", &weak, None, true, Variant::Normal);
    let non_ca = certificate("Non CA root", &chain.key, None, false, Variant::Normal);
    let sha1_ca = certificate("SHA-1 runtime root", &chain.key, None, true, Variant::Sha1);
    for bytes in [
        vec![],
        vec![b' '; MAX_ANCHOR_BYTES + 1],
        private.clone(),
        chain.key.private_key_to_der().unwrap(),
        chain.leaf.to_pem().unwrap(),
        chain.intermediate.to_der().unwrap(),
        non_ca.to_pem().unwrap(),
        weak_ca.to_der().unwrap(),
        sha1_ca.to_der().unwrap(),
        bundle,
        trailing,
        pfx(&chain.key, &chain.leaf, &[]),
    ] {
        let mut trust = TrustStore::default();
        let error = trust.add(0, &bytes).unwrap_err().to_string();
        assert!(!error.contains(PASSWORD));
        assert!(!error.contains(std::str::from_utf8(&private).unwrap()));
        assert_eq!(trust.info().unwrap().revision, 0);
        assert!(trust.info().unwrap().anchors.is_empty());
    }
    let mut padded = chain.root.to_pem().unwrap();
    padded.resize(MAX_ANCHOR_BYTES, b' ');
    let mut trust = TrustStore::default();
    let store = trust.add(0, &padded).unwrap();
    assert_eq!(store.anchors.len(), 1);
    assert_eq!(store.revision, 1);
    assert!(trust.add(1, &chain.root.to_der().unwrap()).is_err());
    assert!(
        trust
            .remove(0, &store.anchors[0].sha256_fingerprint)
            .is_err()
    );
    assert_eq!(trust.info().unwrap().revision, 1);
    trust
        .remove(1, &store.anchors[0].sha256_fingerprint)
        .unwrap();
    assert_eq!(trust.info().unwrap().revision, 2);
    assert!(trust.info().unwrap().anchors.is_empty());
}

#[test]
fn anchor_files_are_bounded_and_private_material_is_not_read_into_any_persistent_store() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("runtime-only.PEM");
    std::fs::write(&path, vec![b' '; MAX_ANCHOR_BYTES]).unwrap();
    assert_eq!(read_anchor_file(&path).unwrap().len(), MAX_ANCHOR_BYTES);
    std::fs::write(&path, vec![b' '; MAX_ANCHOR_BYTES + 1]).unwrap();
    assert!(read_anchor_file(&path).is_err());
    std::fs::write(&path, []).unwrap();
    assert!(read_anchor_file(&path).is_err());
    assert!(read_anchor_file(folder.path()).is_err());
    assert!(read_anchor_file(Path::new("identity.pfx")).is_err());
}

fn open_workspace(workspace: &mut Workspace, bytes: Vec<u8>) -> folio_core::document::DocumentInfo {
    match workspace
        .request_open_bytes(bytes, "Runtime.pdf".into())
        .unwrap()
    {
        OpenOutcome::Opened { document, .. } => document,
        _ => panic!("Runtime fixture did not open"),
    }
}

#[test]
fn removing_anchors_changes_reports_in_every_tab_without_changing_any_document_revision_or_draft() {
    let chain = Chain::new(Variant::Normal, Variant::Normal, Variant::Normal, false);
    let bytes = chain.pdf(true, true);
    let mut workspace = Workspace::default();
    let first = open_workspace(&mut workspace, bytes.clone());
    let second = open_workspace(&mut workspace, bytes.clone());
    workspace.set_draft(first.id, true).unwrap();
    let store = workspace
        .add_signature_anchor(first.id, first.revision, 0, &chain.root.to_pem().unwrap())
        .unwrap();
    for document in [&first, &second] {
        let report = workspace
            .signature_report(document.id, document.revision)
            .unwrap();
        assert_eq!(report.trust_store.revision, 1);
        assert_eq!(report.signatures[0].chain.status, "anchored_custom_store");
    }
    assert!(
        workspace
            .signed_copy(first.id, first.revision, identity(), PASSWORD)
            .is_err()
    );
    assert!(
        workspace
            .add_signature_anchor(
                first.id,
                first.revision + 1,
                1,
                &chain.root.to_der().unwrap()
            )
            .is_err()
    );
    assert!(
        workspace
            .remove_signature_anchor(
                second.id,
                second.revision,
                0,
                &store.anchors[0].sha256_fingerprint
            )
            .is_err()
    );
    workspace
        .remove_signature_anchor(
            second.id,
            second.revision,
            1,
            &store.anchors[0].sha256_fingerprint,
        )
        .unwrap();
    for document in [&first, &second] {
        let report = workspace
            .signature_report(document.id, document.revision)
            .unwrap();
        assert_eq!(report.revision, document.revision);
        assert_eq!(report.trust_store.revision, 2);
        assert_eq!(report.signatures[0].chain.status, "not_checked");
        assert_eq!(workspace.document(document.id).unwrap().bytes(), bytes);
    }
    assert!(
        workspace
            .ensure_export_ready(first.id, first.revision)
            .is_err()
    );
    let json = serde_json::to_string(&workspace.signature_trust_store().unwrap()).unwrap();
    assert!(!json.contains(PASSWORD));
    assert!(!json.contains("PRIVATE KEY"));
    assert!(
        Workspace::default()
            .signature_trust_store()
            .unwrap()
            .anchors
            .is_empty()
    );
}
