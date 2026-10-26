use std::{collections::BTreeMap, sync::Arc};

use lopdf::{
    Document, Object, Stream, dictionary,
    encryption::{
        EncryptionState, EncryptionVersion, Permissions,
        crypt_filters::{Aes128CryptFilter, Aes256CryptFilter, CryptFilter},
    },
};

pub fn synthetic_pdf(page_count: usize, signed: bool) -> Vec<u8> {
    let mut pdf = Document::with_version("1.7");
    let pages = pdf.new_object_id();
    let font = pdf.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources = pdf.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mut ids = Vec::new();
    for index in 0..page_count {
        let mut content = format!(
            "q 0.85 0.18 0.1 rg 36 72 24 24 re f Q\nBT /F1 16 Tf 36 130 Td (SYNTHETIC PAGE {:03}) Tj 0 -24 Td (Selectable page text only.) Tj ET\n",
            index + 1,
        ).into_bytes();
        content.extend(
            b"% Repeated synthetic comments make lossless compression measurable.\n".repeat(1500),
        );
        let content = pdf.add_object(Stream::new(dictionary! {}, content));
        let mut page = dictionary! { "Type" => "Page", "Parent" => pages, "Contents" => content };
        if index == 1 {
            page.set(
                "MediaBox",
                vec![(-20).into(), (-10).into(), 400.into(), 250.into()],
            );
            page.set(
                "CropBox",
                vec![10.into(), 20.into(), 350.into(), 220.into()],
            );
            page.set("Rotate", 90);
            page.set("UserUnit", 2);
        } else if index == 2 {
            page.set("CropBox", vec![0.into(), 0.into(), 300.into(), 400.into()]);
            page.set("Rotate", 270);
            page.set("UserUnit", 1.5);
        }
        ids.push(pdf.add_object(page));
    }
    pdf.objects.insert(
        pages,
        dictionary! {
            "Type" => "Pages", "Count" => page_count as i64,
            "Kids" => ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources,
        }
        .into(),
    );
    let mut metadata = Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        b"<synthetic>Retained metadata and existing compression</synthetic>".repeat(30),
    );
    metadata.compress().unwrap();
    let metadata = pdf.add_object(metadata);
    let attachment = pdf.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile" },
        b"Synthetic attachment retained exactly.\n".to_vec(),
    ));
    let file = pdf.add_object(dictionary! {
        "Type" => "Filespec", "F" => lopdf::text_string("synthetic.txt"),
        "EF" => dictionary! { "F" => attachment },
    });
    let outlines = pdf.new_object_id();
    let bookmark = pdf.add_object(dictionary! {
        "Title" => lopdf::text_string("Synthetic first page"),
        "Parent" => outlines, "Dest" => vec![Object::Reference(ids[0]), Object::Name(b"Fit".to_vec())],
    });
    pdf.objects.insert(
        outlines,
        dictionary! {
            "Type" => "Outlines", "First" => bookmark, "Last" => bookmark, "Count" => 1,
        }
        .into(),
    );
    let root = pdf.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages, "Metadata" => metadata, "Outlines" => outlines,
        "Names" => dictionary! { "EmbeddedFiles" => dictionary! {
            "Names" => vec![lopdf::text_string("synthetic.txt"), Object::Reference(file)],
        }},
    });
    if signed {
        let marker = pdf.add_object(dictionary! {
            "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
            "Contents" => Object::string_literal("TEST MARKER ONLY - not a cryptographic signature"),
        });
        pdf.get_dictionary_mut(root)
            .unwrap()
            .set("SyntheticSignature", marker);
    }
    let info = pdf.add_object(dictionary! {
        "Title" => lopdf::text_string("Synthetic Folio output fixture"),
        "Author" => lopdf::text_string("Folio synthetic fixture generator"),
    });
    pdf.add_object(Stream::new(
        dictionary! {},
        b"Unreachable synthetic object.\n".repeat(5000),
    ));
    pdf.trailer.set("Root", root);
    pdf.trailer.set("Info", info);
    pdf.trailer.set(
        "ID",
        vec![
            Object::string_literal("folio-output-fixture"),
            Object::string_literal("folio-output-fixture"),
        ],
    );
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

pub fn encrypted(bytes: &[u8], version: u8, permissions: Permissions) -> Vec<u8> {
    let mut pdf = Document::load_mem(bytes).unwrap();
    let version = match version {
        1 => EncryptionVersion::V1 {
            document: &pdf,
            owner_password: "owner",
            user_password: "user",
            permissions,
        },
        2 => EncryptionVersion::V2 {
            document: &pdf,
            owner_password: "owner",
            user_password: "user",
            permissions,
            key_length: 128,
        },
        4 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes128CryptFilter);
            EncryptionVersion::V4 {
                document: &pdf,
                owner_password: "owner",
                user_password: "user",
                permissions,
                encrypt_metadata: true,
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        5 => {
            let filter: Arc<dyn CryptFilter> = Arc::new(Aes256CryptFilter);
            EncryptionVersion::V5 {
                owner_password: "owner",
                user_password: "user",
                permissions,
                encrypt_metadata: true,
                file_encryption_key: &[42; 32],
                crypt_filters: BTreeMap::from([(b"StdCF".to_vec(), filter)]),
                stream_filter: b"StdCF".to_vec(),
                string_filter: b"StdCF".to_vec(),
            }
        }
        _ => panic!("unsupported test encryption"),
    };
    pdf.encrypt(&EncryptionState::try_from(version).unwrap())
        .unwrap();
    let mut bytes = Vec::new();
    pdf.save_to(&mut bytes).unwrap();
    bytes
}

#[cfg(not(test))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("Pass an output directory for synthetic PDF fixtures.")?;
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    let bytes = synthetic_pdf(24, false);
    for (name, bytes) in [
        ("output-24-pages.pdf", bytes.clone()),
        ("output-signed-marker.pdf", synthetic_pdf(3, true)),
        (
            "output-protected.pdf",
            encrypted(&bytes, 4, Permissions::all()),
        ),
        (
            "output-print-only.pdf",
            encrypted(&bytes, 4, Permissions::PRINTABLE),
        ),
        (
            "output-copy-only.pdf",
            encrypted(&bytes, 4, Permissions::COPYABLE),
        ),
    ] {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(name))?;
        std::io::Write::write_all(&mut file, &bytes)?;
    }
    println!(
        "Created synthetic output fixtures. Protected fixtures use user / owner passwords; the signature marker is not a real signature."
    );
    Ok(())
}
