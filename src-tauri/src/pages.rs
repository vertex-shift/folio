use std::collections::{BTreeMap, HashSet};

use lopdf::{Document, Object, ObjectId, Stream, dictionary, text_string};

use crate::document::{
    MAX_DOCUMENT_BYTES, Result, annotation_entries, has_form_fields, inherited, invalid, numbers,
    page_id, rebuild_page_tree, retarget_destinations,
};

pub(crate) fn blank_page(document: &mut Document, width: f32, height: f32) -> Result<ObjectId> {
    if !width.is_finite()
        || !height.is_finite()
        || !(72.0..=14400.0).contains(&width)
        || !(72.0..=14400.0).contains(&height)
    {
        return Err(invalid(
            "Page dimensions must be between 72 and 14,400 PDF points.",
        ));
    }
    let contents = document.add_object(Stream::new(dictionary! {}, Vec::new()));
    Ok(document.add_object(dictionary! {
        "Type" => "Page", "MediaBox" => numbers(&[0.0, 0.0, width, height]),
        "Resources" => dictionary! {}, "Contents" => contents,
    }))
}

pub(crate) fn reject_forms(document: &Document) -> Result<()> {
    if has_form_fields(document)? {
        return Err(invalid(
            "This page operation does not yet support interactive forms or XFA. Use a flattened copy.",
        ));
    }
    for page in document.get_pages().into_values() {
        for annotation in document.get_page_annotations(page)? {
            if annotation.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget") {
                return Err(invalid(
                    "Page widgets cannot yet be imported, duplicated, or extracted safely. Use a flattened copy.",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn duplicate_page(document: &mut Document, index: usize) -> Result<()> {
    reject_forms(document)?;
    let id = page_id(document, index)?;
    let mut page = document.get_dictionary(id)?.clone();
    for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
        if let Some(value) = inherited(document, id, key)? {
            page.set(key, value);
        }
    }
    let entries = annotation_entries(document, id)?;
    let mut annotations = Vec::new();
    for entry in entries {
        let annotation = document.dereference(&entry)?.1.as_dict()?.clone();
        if annotation.has(b"Popup")
            || annotation.has(b"IRT")
            || annotation.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
        {
            return Err(invalid(
                "This page has popup, reply, or form annotations that cannot yet be duplicated safely.",
            ));
        }
        annotations.push(annotation);
    }
    page.remove(b"Annots");
    let duplicate = document.add_object(page);
    let mut entries = Vec::new();
    for mut annotation in annotations {
        annotation.set("P", duplicate);
        annotation.set(
            "NM",
            text_string(&format!("folio-copy-{}", document.max_id + 1)),
        );
        entries.push(Object::Reference(document.add_object(annotation)));
    }
    if !entries.is_empty() {
        document
            .get_dictionary_mut(duplicate)?
            .set("Annots", entries);
    }
    let mut order: Vec<_> = document.get_pages().into_values().collect();
    order.insert(index + 1, duplicate);
    rebuild_page_tree(document, &order)
}

pub(crate) fn import_pages(document: &mut Document, mut donor: Document, at: usize) -> Result<()> {
    reject_forms(document)?;
    reject_forms(&donor)?;
    if donor.catalog()?.has(b"OCProperties") {
        return Err(invalid(
            "Importing a PDF with optional-content layers is not supported yet. Use a flattened copy.",
        ));
    }
    let mut order: Vec<_> = document.get_pages().into_values().collect();
    if at > order.len() {
        return Err(invalid("Choose a valid page insertion position."));
    }
    inline_named_destinations(&mut donor)?;
    let incoming: Vec<_> = donor.get_pages().into_values().collect();
    for id in &incoming {
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if let Some(value) = inherited(&donor, *id, key)? {
                donor.get_dictionary_mut(*id)?.set(key, value);
            }
        }
    }
    if incoming.len() + order.len() > 10_000 {
        return Err(invalid("A combined document is limited to 10,000 pages."));
    }
    donor.renumber_objects_with(
        document
            .max_id
            .checked_add(1)
            .ok_or_else(|| invalid("This PDF has too many objects."))?,
    );
    let incoming: Vec<_> = donor.get_pages().into_values().collect();
    document.max_id = donor.max_id;
    document.objects.extend(donor.objects);
    order.splice(at..at, incoming);
    rebuild_page_tree(document, &order)
}

pub(crate) fn extract_pages(document: &Document, pages: &[usize]) -> Result<Vec<u8>> {
    reject_forms(document)?;
    let all: Vec<_> = document.get_pages().into_values().collect();
    let unique: HashSet<_> = pages.iter().collect();
    if pages.is_empty()
        || pages.len() != unique.len()
        || pages.iter().any(|index| *index >= all.len())
    {
        return Err(invalid("Select valid, non-repeating pages to extract."));
    }
    let mut extracted = document.clone();
    let order: Vec<_> = pages.iter().map(|index| all[*index]).collect();
    let kept: HashSet<_> = order.iter().copied().collect();
    for id in &all {
        if !kept.contains(id) {
            for object in extracted.objects.values_mut() {
                retarget_destinations(object, *id, order[0]);
            }
        }
    }
    rebuild_page_tree(&mut extracted, &order)?;
    // Page destinations and structural metadata must not keep discarded content reachable.
    for id in &all {
        if !kept.contains(id) {
            extracted.delete_object(*id);
        }
    }
    extracted.catalog_mut()?.remove(b"PageLabels");
    extracted.prune_objects();
    let mut bytes = Vec::new();
    extracted.save_to(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("The extracted PDF exceeds the file-size limit."));
    }
    Ok(bytes)
}

fn inline_named_destinations(document: &mut Document) -> Result<()> {
    let mut destinations = BTreeMap::new();
    let catalog = document.catalog()?.clone();
    if let Ok(old) = catalog.get(b"Dests") {
        for (name, value) in document.dereference(old)?.1.as_dict()?.iter() {
            destinations.insert(name.clone(), destination_array(document, value)?);
        }
    }
    if let Ok(names) = catalog.get(b"Names") {
        let names = document.dereference(names)?.1.as_dict()?;
        if let Ok(tree) = names.get(b"Dests") {
            collect_destinations(document, tree, &mut destinations, &mut HashSet::new(), 0)?;
        }
    }
    let ids: Vec<_> = document.get_pages().into_values().collect();
    for id in ids {
        let mut entries = annotation_entries(document, id)?;
        for entry in &mut entries {
            let mut annotation = document.dereference(entry)?.1.as_dict()?.clone();
            if let Ok(value) = annotation.get(b"Dest") {
                let replacement = resolve_named(value, &destinations)?;
                if let Some(value) = replacement {
                    annotation.set("Dest", value);
                }
            }
            if let Ok(action) = annotation.get(b"A") {
                let mut action = document.dereference(action)?.1.as_dict()?.clone();
                if action.get(b"S").and_then(Object::as_name).ok() == Some(b"GoTo") {
                    if let Some(value) = resolve_named(action.get(b"D")?, &destinations)? {
                        action.set("D", value);
                    }
                    annotation.set("A", action);
                }
            }
            *entry = Object::Reference(document.add_object(annotation));
        }
        if !entries.is_empty() {
            document.get_dictionary_mut(id)?.set("Annots", entries);
        }
    }
    Ok(())
}

fn destination_array(document: &Document, value: &Object) -> Result<Object> {
    let value = document.dereference(value)?.1;
    let value = if let Object::Dictionary(dict) = value {
        document.dereference(dict.get(b"D")?)?.1
    } else {
        value
    };
    let array = value.as_array()?;
    if array.len() < 2 {
        return Err(invalid("An imported PDF destination is invalid."));
    }
    Ok(Object::Array(array.clone()))
}

fn resolve_named(
    value: &Object,
    destinations: &BTreeMap<Vec<u8>, Object>,
) -> Result<Option<Object>> {
    let name = match value {
        Object::Name(name) | Object::String(name, _) => name,
        _ => return Ok(None),
    };
    destinations
        .get(name)
        .cloned()
        .map(Some)
        .ok_or_else(|| invalid("An imported page link points to an unresolved named destination."))
}

fn collect_destinations(
    document: &Document,
    tree: &Object,
    result: &mut BTreeMap<Vec<u8>, Object>,
    visited: &mut HashSet<ObjectId>,
    depth: usize,
) -> Result<()> {
    if depth > 32 || result.len() > 10_000 {
        return Err(invalid(
            "The imported PDF destination tree is too large or deep.",
        ));
    }
    if let Ok(id) = tree.as_reference()
        && !visited.insert(id)
    {
        return Err(invalid("The imported PDF has a cyclic destination tree."));
    }
    let tree = document.dereference(tree)?.1.as_dict()?;
    if let Ok(names) = tree.get(b"Names") {
        let names = document.dereference(names)?.1.as_array()?;
        if names.len() % 2 != 0 {
            return Err(invalid(
                "The imported PDF has a malformed destination name tree.",
            ));
        }
        for pair in names.chunks_exact(2) {
            result.insert(
                pair[0].as_str()?.to_vec(),
                destination_array(document, &pair[1])?,
            );
        }
    }
    if let Ok(kids) = tree.get(b"Kids") {
        for kid in document.dereference(kids)?.1.as_array()? {
            collect_destinations(document, kid, result, visited, depth + 1)?;
        }
    }
    Ok(())
}
