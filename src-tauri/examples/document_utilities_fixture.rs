use std::path::PathBuf;

use folio_core::{
    document::{Edit, Session},
    metadata::Fields,
    protection::{CopyPermissions, CopyProtection, PrintPermission},
};
use lopdf::{Document, Stream, dictionary};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("Pass an output directory for original synthetic fixtures.")?;
    std::fs::create_dir_all(&directory)?;
    let mut document = Document::load_mem(Session::demo(1)?.bytes())?;
    let xmp = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:t="urn:folio-synthetic">
<dc:title><rdf:Alt><rdf:li xml:lang="x-default">Synthetic document utilities</rdf:li></rdf:Alt></dc:title>
<t:Preserve>This original fixture property must remain.</t:Preserve></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let metadata = document.add_object(Stream::new(
        dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
        xmp.to_vec(),
    ));
    document.catalog_mut()?.set("Metadata", metadata);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes)?;
    let mut session = Session::from_bytes(bytes, "Document utilities.pdf".into(), None, 1)?;
    session.apply(Edit::Metadata {
        fields: Fields {
            title: "Document utilities · Café · 資料".into(),
            author: "Original Folio synthetic fixture".into(),
            subject: "Metadata, outline and attachment round-trip".into(),
            keywords: "synthetic; local; review".into(),
        },
    })?;
    session.apply(Edit::BookmarkCreate {
        parent: None,
        at: 0,
        title: "資料 · first page".into(),
        page: 0,
    })?;
    let parent = session.bookmarks()?[0].id.clone();
    session.apply(Edit::BookmarkCreate {
        parent: Some(parent),
        at: 0,
        title: "Chapter two".into(),
        page: 1,
    })?;
    session.add_attachment(
        "資料 café.txt",
        b"Original synthetic text attachment. This file is data, never execute it.".to_vec(),
    )?;
    std::fs::write(directory.join("document-utilities.pdf"), session.bytes())?;
    for (name, permissions) in [
        (
            "document-utilities-protected.pdf",
            CopyPermissions {
                print: PrintPermission::High,
                copy: true,
                edit: true,
                annotate: true,
                fill_forms: true,
                assemble: true,
            },
        ),
        (
            "document-utilities-restricted.pdf",
            CopyPermissions {
                print: PrintPermission::Low,
                copy: false,
                edit: false,
                annotate: false,
                fill_forms: true,
                assemble: false,
            },
        ),
    ] {
        let protection = CopyProtection::Protect {
            opening_password: "reader".into(),
            owner_password: "owner".into(),
            permissions,
        };
        std::fs::write(
            directory.join(name),
            session.protection_copy(&protection, None)?,
        )?;
    }
    println!(
        "Original fixtures written to {}. Synthetic protected passwords: reader / owner.",
        directory.display()
    );
    Ok(())
}
