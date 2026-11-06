use super::*;
use crate::document::annotation_entries;

// These operations never rewrite field nodes, widget rectangles or appearances.
// Validate both sides of geometry changes so no value becomes partly cropped.
pub(crate) fn check(document: &Document) -> Result<()> {
    let form = read(document)?;
    if let Some(reason) = &form.reason {
        return Err(invalid(reason));
    }
    if form.need_appearances {
        return Err(invalid(
            "Form-aware page operations require existing appearances; NeedAppearances forms are unsupported.",
        ));
    }
    if !form.fields.is_empty() {
        let catalog = document.catalog()?;
        if catalog.has(b"StructTreeRoot") || catalog.has(b"MarkInfo") {
            return Err(invalid(
                "Form-aware page operations do not support tagged or structure-declared PDFs.",
            ));
        }
        crate::preflight::walk(document, |object| {
            let dict = match object {
                Object::Dictionary(dict) => dict,
                Object::Stream(stream) => &stream.dict,
                _ => return Ok(()),
            };
            if [
                b"A".as_slice(),
                b"AA",
                b"JS",
                b"JavaScript",
                b"StructParent",
                b"StructParents",
            ]
            .iter()
            .any(|key| dict.has(key))
            {
                return Err(invalid(
                    "Form-aware page operations do not support actions, scripts or structure ownership. No actions are executed.",
                ));
            }
            Ok(())
        })?;
        for finding in crate::preflight::inspect(document)? {
            if finding.category == "declarations"
                && (finding.state == crate::preflight::State::Unsupported
                    || finding.property.starts_with("pdfaid:")
                    || finding.property.starts_with("pdfuaid:"))
            {
                return Err(invalid(
                    "Form-aware page operations do not rewrite PDF/A, PDF/UA or unreadable conformance declarations.",
                ));
            }
        }
    }
    designer::check_names(&form)?;
    if let Some(acroform) = optional(document, document.catalog()?, b"AcroForm")?
        && let Some(roots) = optional(document, acroform.as_dict()?, b"Fields")?
    {
        let mut pending = roots.as_array()?.clone();
        let mut visited = HashSet::new();
        while let Some(node) = pending.pop() {
            let id = node.as_reference()?;
            if !visited.insert(id) || visited.len() > MAX_NODES {
                return Err(invalid(
                    "The form field tree is cyclic, shared or exceeds the node limit.",
                ));
            }
            if let Some(children) = optional(document, document.get_dictionary(id)?, b"Kids")? {
                for child in children.as_array()? {
                    if document
                        .get_dictionary(child.as_reference()?)?
                        .get(b"Parent")
                        .and_then(Object::as_reference)
                        .ok()
                        != Some(id)
                    {
                        return Err(invalid(
                            "Every child field/widget must reference its actual parent in the field tree.",
                        ));
                    }
                    pending.push(child.clone());
                }
            }
        }
    }
    let mut widgets = HashSet::new();
    for field in &form.fields {
        if field.widgets.len() != field.info.widgets.len() {
            return Err(invalid(
                "Every form widget must belong to exactly one page with valid geometry wholly inside the visible crop. Crops that clip widgets are refused; no pages were changed.",
            ));
        }
        designer::supported(document, field)?;
        if integer(document, &field.attributes, b"Ff", 0)? & 1 != 0 {
            return Err(invalid(
                "Form-aware page operations do not support read-only fields.",
            ));
        }
        for widget in &field.widgets {
            let dict = document.get_dictionary(*widget)?;
            if integer(document, dict, b"F", 0)? & (64 | 128 | 512) != 0 {
                return Err(invalid(
                    "Form-aware page operations do not support read-only or locked widgets.",
                ));
            }
            let appearance = optional(document, dict, b"AP")?
                .ok_or_else(|| invalid("Every widget needs an existing normal appearance for form-aware page operations."))?
                .as_dict()?;
            let normal = optional(document, appearance, b"N")?
                .ok_or_else(|| invalid("Every widget needs an existing normal appearance for form-aware page operations."))?;
            if matches!(field.info.kind, FieldKind::Checkbox | FieldKind::Radio) {
                let state = optional(document, dict, b"AS")?
                    .ok_or_else(|| invalid("A button widget has no appearance state."))?
                    .as_name()?;
                let expected = optional(document, &field.attributes, b"V")?
                    .map(Object::as_name)
                    .transpose()?
                    .unwrap_or(b"Off");
                let on = button_state(document, *widget)?;
                let expected = if expected == on { expected } else { b"Off" };
                if state != expected {
                    return Err(invalid(
                        "A button's appearance state disagrees with its field value. Apply form values before page operations.",
                    ));
                }
                optional(document, normal.as_dict()?, state)?
                    .ok_or_else(|| invalid("A button's current appearance is missing."))?
                    .as_stream()?;
            } else {
                normal.as_stream()?;
            }
            widgets.insert(*widget);
        }
    }
    let mut count = 0;
    for page in document.get_pages().into_values() {
        for annotation in annotation_entries(document, page)? {
            count += 1;
            if count > 65_536 {
                return Err(invalid(
                    "Form-aware page operations exceed the 65,536-annotation limit.",
                ));
            }
            let dict = resolve(document, &annotation)?.as_dict()?;
            if dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
                && annotation
                    .as_reference()
                    .ok()
                    .is_none_or(|id| !widgets.contains(&id))
            {
                return Err(invalid(
                    "A page has an orphan, direct or unrecognized widget. Form-aware page operations cannot preserve its field association.",
                ));
            }
        }
    }
    Ok(())
}

#[derive(PartialEq)]
pub(crate) struct Snapshot {
    root: Option<Object>,
    objects: BTreeMap<ObjectId, Object>,
}

impl Snapshot {
    fn capture(document: &Document) -> Result<Self> {
        let root = document.catalog()?.get(b"AcroForm").ok().cloned();
        let pages: HashSet<_> = document.get_pages().into_values().collect();
        let mut objects = BTreeMap::new();
        let mut stack: Vec<_> = root.iter().map(|value| (value, 0)).collect();
        let mut nodes = 0;
        while let Some((value, depth)) = stack.pop() {
            nodes += 1;
            if nodes > 50_000 || depth > 64 {
                return Err(invalid(
                    "The form resource graph exceeds the bounded inspection needed for page deletion.",
                ));
            }
            match value {
                Object::Reference(id) if !pages.contains(id) && !objects.contains_key(id) => {
                    let value = document.get_object(*id)?;
                    objects.insert(*id, value.clone());
                    stack.push((value, depth));
                }
                Object::Array(values) => {
                    stack.extend(values.iter().map(|value| (value, depth + 1)))
                }
                Object::Dictionary(dict) => {
                    stack.extend(dict.iter().map(|(_, value)| (value, depth + 1)))
                }
                Object::Stream(stream) => {
                    stack.extend(stream.dict.iter().map(|(_, value)| (value, depth + 1)))
                }
                _ => {}
            }
        }
        Ok(Self { root, objects })
    }

    pub(crate) fn verify(self, document: &Document) -> Result<()> {
        if self != Self::capture(document)? {
            return Err(invalid(
                "Deleting this page would change the form's field tree, values or appearance resources. No page or field was removed.",
            ));
        }
        Ok(())
    }
}

pub(crate) fn check_delete(document: &Document, page: ObjectId) -> Result<Snapshot> {
    check(document)?;
    for annotation in annotation_entries(document, page)? {
        if resolve(document, &annotation)?
            .as_dict()?
            .get(b"Subtype")
            .and_then(Object::as_name)
            .ok()
            == Some(b"Widget")
        {
            return Err(invalid(
                "Only pages without form widgets can be deleted from supported AcroForms. Pages containing widgets cannot yet be removed safely; no fields or values were discarded.",
            ));
        }
    }
    Snapshot::capture(document)
}
