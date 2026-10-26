use super::*;
use crate::document::{annotation_entries, inherited, page_id};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlattenInfo {
    pub field_count: usize,
    pub widget_count: usize,
    pub unsigned_signature_count: usize,
    pub warnings: Vec<String>,
}

fn checked_form(document: &Document) -> Result<Form> {
    let form = read(document)?;
    if let Some(reason) = &form.reason {
        return Err(invalid(reason));
    }
    if form.fields.is_empty() {
        return Err(invalid("This PDF has no AcroForm fields to flatten."));
    }
    designer::check_names(&form)?;
    let mut widgets = HashSet::new();
    for field in &form.fields {
        designer::supported(document, field).map_err(|error| field_error(field, error))?;
        for widget in &field.widgets {
            widgets.insert(*widget);
        }
    }
    if widgets.len() > 1024 {
        return Err(invalid("Static form copies support at most 1,024 widgets."));
    }
    for page in document.get_pages().into_values() {
        for annotation in annotation_entries(document, page)? {
            let dictionary = resolve(document, &annotation)?.as_dict()?;
            if dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
                && annotation
                    .as_reference()
                    .ok()
                    .is_none_or(|id| !widgets.contains(&id))
            {
                return Err(invalid(
                    "A page has an orphan, direct or unrecognized widget. Static export will not silently drop its value.",
                ));
            }
        }
    }
    Ok(form)
}

pub(crate) fn inspect(document: &Document) -> Result<FlattenInfo> {
    let form = checked_form(document)?;
    Ok(FlattenInfo {
        field_count: form.fields.len(),
        widget_count: form.fields.iter().map(|field| field.widgets.len()).sum(),
        unsigned_signature_count: form.fields.iter().filter(|field| field.info.kind == FieldKind::Signature).count(),
        warnings: vec![
            "Exports a separate static PDF. The source tab, unsaved changes and undo/redo history stay unchanged; existing encryption is retained.".into(),
            "Text and choices are rebuilt from current PDF values with embedded Noto Sans, not the original field font. Fixed font sizes are preserved; values that do not fit stop the export.".into(),
            "Checkboxes and radios use the appearance matching their current value, not a stale widget state. All selected list values must be visible in every widget.".into(),
            "Password values become masked text. Empty digital-signature slots become unsigned static outlines, not signatures. Interactive widgets and their field actions are removed.".into(),
            "Not secure sanitization, redaction, PDF/A conversion or digital-signature preservation. Other page content, annotations, attachments and metadata remain. Signed/certified sources, scripts/XFA/calculations, hidden widgets and unsupported fields stop export.".into(),
        ],
    })
}

fn merged_resources(
    document: &Document,
    fallback: &Dictionary,
    local: Option<&Object>,
) -> Result<Dictionary> {
    let mut resources = fallback.clone();
    if let Some(local) = local {
        for (key, value) in resolve(document, local)?.as_dict()? {
            if let Some(existing) = optional(document, &resources, key)?
                && let (Ok(existing), Ok(added)) =
                    (existing.as_dict(), resolve(document, value)?.as_dict())
            {
                let mut combined = existing.clone();
                for (name, value) in added {
                    combined.set(name.as_slice(), value.clone());
                }
                resources.set(key.as_slice(), combined);
                continue;
            }
            resources.set(key.as_slice(), value.clone());
        }
    }
    Ok(resources)
}

fn resource<'a>(
    document: &'a Document,
    resources: &'a Dictionary,
    category: &[u8],
    name: &[u8],
) -> Result<&'a Object> {
    let group = optional(document, resources, category)?
        .ok_or_else(|| invalid("An appearance is missing a resource category; its visible value cannot be guaranteed."))?.as_dict()?;
    optional(document, group, name)?.ok_or_else(|| invalid(
        "An appearance references a missing font, XObject or graphics resource. No static copy was created."
    ))
}

struct AppearanceBudget {
    bytes: usize,
    nodes: usize,
    stack: HashSet<ObjectId>,
}

fn checked_content(bytes: &[u8]) -> Result<Content<Vec<Operation>>> {
    if bytes.len() > MAX_APPEARANCE_BYTES {
        return Err(invalid(
            "A page or appearance exceeds the 32 MiB decoded-content limit.",
        ));
    }
    let content = Content::decode(bytes)?;
    let mut graphics_depth = 0;
    let mut text_open = false;
    for operation in &content.operations {
        match operation.operator.as_str() {
            "q" => graphics_depth += 1,
            "Q" if graphics_depth > 0 => graphics_depth -= 1,
            "Q" => {
                return Err(invalid(
                    "A page or appearance has an unbalanced graphics-state stack.",
                ));
            }
            "BT" if !text_open => text_open = true,
            "ET" if text_open => text_open = false,
            "BT" | "ET" => {
                return Err(invalid("A page or appearance has unbalanced text objects."));
            }
            _ => {}
        }
        if graphics_depth > 128 {
            return Err(invalid(
                "A page or appearance exceeds 128 nested graphics states.",
            ));
        }
    }
    if graphics_depth != 0 || text_open {
        return Err(invalid(
            "A page or appearance has an unbalanced graphics/text state; it cannot be safely composed.",
        ));
    }
    Ok(content)
}

fn validate_resources(
    document: &Document,
    stream: &Stream,
    fallback: &Dictionary,
    budget: &mut AppearanceBudget,
    depth: usize,
) -> Result<Dictionary> {
    budget.nodes += 1;
    if depth > 16
        || budget.nodes > 4096
        || stream.dict.has(b"F")
        || stream.dict.has(b"Ref")
        || stream.dict.has(b"OC")
    {
        return Err(invalid(
            "An appearance has external/optional content or exceeds the bounded resource graph limits.",
        ));
    }
    let resources = merged_resources(document, fallback, stream.dict.get(b"Resources").ok())?;
    let bytes = crate::pdf_objects::stream_bytes(stream, MAX_APPEARANCE_BYTES)?;
    budget.bytes += bytes.len();
    if budget.bytes > MAX_APPEARANCE_BYTES {
        return Err(invalid(
            "Flattened appearances exceed the 32 MiB decoded resource limit.",
        ));
    }
    let content = checked_content(&bytes)?;
    for operation in content.operations {
        let name = operation
            .operands
            .first()
            .and_then(|value| value.as_name().ok());
        let category = match operation.operator.as_str() {
            "Tf" => Some(b"Font".as_slice()),
            "Do" => Some(b"XObject".as_slice()),
            "gs" => Some(b"ExtGState".as_slice()),
            "sh" => Some(b"Shading".as_slice()),
            "CS" | "cs"
                if !matches!(
                    name,
                    Some(b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" | b"Pattern")
                ) =>
            {
                Some(b"ColorSpace".as_slice())
            }
            "SCN" | "scn" => {
                if let Some(name) = operation
                    .operands
                    .last()
                    .and_then(|value| value.as_name().ok())
                {
                    resource(document, &resources, b"Pattern", name)?;
                }
                None
            }
            "BDC" | "DP" => {
                if let Some(name) = operation
                    .operands
                    .get(1)
                    .and_then(|value| value.as_name().ok())
                {
                    resource(document, &resources, b"Properties", name)?;
                }
                None
            }
            _ => None,
        };
        if let Some(category) = category {
            let name =
                name.ok_or_else(|| invalid("An appearance contains an invalid resource operand."))?;
            let value = resource(document, &resources, category, name)?;
            if category == b"XObject" {
                let child = value.as_stream()?;
                if child.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Form") {
                    let reference = optional(document, &resources, b"XObject")?
                        .expect("resource checked")
                        .as_dict()?
                        .get(name)?
                        .as_reference()
                        .ok();
                    if let Some(id) = reference
                        && !budget.stack.insert(id)
                    {
                        return Err(invalid("An appearance has a recursive XObject resource."));
                    }
                    validate_resources(document, child, &resources, budget, depth + 1)?;
                    if let Some(id) = reference {
                        budget.stack.remove(&id);
                    }
                } else if child.dict.get(b"Subtype").and_then(Object::as_name).ok()
                    != Some(b"Image")
                    || child.dict.has(b"F")
                {
                    return Err(invalid(
                        "An appearance contains an unsupported or external XObject.",
                    ));
                }
            } else if category == b"Font" {
                value.as_dict()?;
            }
        }
    }
    Ok(resources)
}

pub(super) fn matrix(document: &Document, stream: &Stream) -> Result<[f32; 6]> {
    let Some(value) = optional(document, &stream.dict, b"Matrix")? else {
        return Ok([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    };
    let values = value.as_array()?;
    if values.len() != 6 {
        return Err(invalid("An appearance has an invalid Matrix."));
    }
    let mut matrix = [0.0; 6];
    for (index, value) in values.iter().enumerate() {
        matrix[index] = resolve(document, value)?.as_float()?;
    }
    if matrix.iter().any(|value| !value.is_finite())
        || (matrix[0] * matrix[3] - matrix[1] * matrix[2]).abs() < 1e-10
    {
        return Err(invalid(
            "An appearance has a singular or non-finite Matrix.",
        ));
    }
    Ok(matrix)
}

pub(super) fn placement_matrix(
    document: &Document,
    stream: &Stream,
    rect: [f32; 4],
) -> Result<[f32; 6]> {
    if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form") {
        return Err(invalid(
            "A normal widget appearance must be a Form XObject.",
        ));
    }
    let bounds = crate::page_geometry::read_box(document, stream.dict.get(b"BBox")?)?;
    let [a, b, c, d, e, f] = matrix(document, stream)?;
    let mut transformed = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for [x, y] in [
        [bounds[0], bounds[1]],
        [bounds[2], bounds[1]],
        [bounds[2], bounds[3]],
        [bounds[0], bounds[3]],
    ] {
        let (x, y) = (a * x + c * y + e, b * x + d * y + f);
        transformed = [
            transformed[0].min(x),
            transformed[1].min(y),
            transformed[2].max(x),
            transformed[3].max(y),
        ];
    }
    let sx = (rect[2] - rect[0]) / (transformed[2] - transformed[0]);
    let sy = (rect[3] - rect[1]) / (transformed[3] - transformed[1]);
    let placement = [
        sx,
        0.0,
        0.0,
        sy,
        rect[0] - sx * transformed[0],
        rect[1] - sy * transformed[1],
    ];
    if placement.iter().any(|value| !value.is_finite()) {
        return Err(invalid(
            "An appearance cannot be fitted to its widget rectangle.",
        ));
    }
    Ok(placement)
}

fn normal_appearance(document: &Document, field: &Field, widget: ObjectId) -> Result<Stream> {
    let widget = document.get_dictionary(widget)?;
    let ap = resolve(document, widget.get(b"AP")?)?.as_dict()?;
    let normal = resolve(document, ap.get(b"N")?)?;
    let stream = if matches!(field.info.kind, FieldKind::Checkbox | FieldKind::Radio) {
        let state = widget.get(b"AS")?.as_name()?;
        resolve(document, normal.as_dict()?.get(state)?)?.as_stream()?
    } else {
        normal.as_stream()?
    };
    Ok(stream.clone())
}

fn selected_list_values_fit(
    document: &Document,
    field: &Field,
    widget: ObjectId,
    stream: &Stream,
) -> Result<()> {
    let Some(FieldValue::Choice { indices, .. }) = &field.info.value else {
        return Ok(());
    };
    if field.info.combo || indices.len() < 2 {
        return Ok(());
    }
    let style = style(document, widget, &field.attributes)?;
    let content = Content::decode(&crate::pdf_objects::stream_bytes(
        stream,
        MAX_APPEARANCE_BYTES,
    )?)?;
    let size = content
        .operations
        .iter()
        .find(|operation| operation.operator == "Tf")
        .and_then(|operation| operation.operands.get(1))
        .ok_or_else(|| invalid("The list appearance has no text size."))?
        .as_float()?;
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|_| invalid("The bundled form font is unavailable."))?;
    let units = f32::from(face.units_per_em());
    let line_height =
        size * ((f32::from(face.ascender()) - f32::from(face.descender())) / units).max(1.2);
    let padding = (style.border_width + 2.0).max(2.0);
    let visible = (((style.height - 2.0 * padding) / line_height).floor() as usize).max(1);
    if indices.last().expect("multiple selections") - indices[0] >= visible {
        return Err(invalid(
            "Some selected list values are outside the visible widget. Enlarge the list or change the selection before making a static copy.",
        ));
    }
    Ok(())
}

fn page_appearance(
    document: &Document,
    stream: Stream,
    rect: [f32; 4],
    placement: [f32; 6],
) -> Result<Stream> {
    let original_matrix = matrix(document, &stream)?;
    let mut bytes = Content {
        operations: vec![
            Operation::new("q", vec![]),
            Operation::new("cm", placement.into_iter().map(Object::Real).collect()),
            Operation::new(
                "cm",
                original_matrix.into_iter().map(Object::Real).collect(),
            ),
        ],
    }
    .encode()?;
    bytes.push(b'\n');
    bytes.extend(crate::pdf_objects::stream_bytes(
        &stream,
        MAX_APPEARANCE_BYTES,
    )?);
    bytes.extend(b"\nQ\n");
    if bytes.len() > MAX_APPEARANCE_BYTES {
        return Err(invalid(
            "A flattened widget exceeds the 32 MiB appearance limit.",
        ));
    }
    let mut dictionary = stream.dict;
    for key in [b"Filter".as_slice(), b"DecodeParms", b"Length", b"DL"] {
        dictionary.remove(key);
    }
    // Clip once in page coordinates, like a widget. Clipping both the fitted
    // Form BBox and Rect multiplies fractional-edge antialias coverage.
    dictionary.set("BBox", numbers(&rect));
    dictionary.set("Matrix", numbers(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    Ok(Stream::new(dictionary, bytes))
}

pub(crate) fn copy(source: &Document) -> Result<Vec<u8>> {
    let form = checked_form(source)?;
    let mut document = source.clone();
    let changes = form
        .fields
        .iter()
        .filter_map(|field| {
            field.info.value.clone().map(|value| FieldChange {
                id: field.info.id.clone(),
                value,
            })
        })
        .collect::<Vec<_>>();
    if !changes.is_empty() {
        write_values(&mut document, changes, false)?;
    }
    let form = checked_form(&document)?;
    let acroform = designer::acroform(&document)?;
    let fallback = optional(&document, &acroform, b"DR")?
        .map(Object::as_dict)
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let mut pages: BTreeMap<usize, Vec<(String, ObjectId)>> = BTreeMap::new();
    let mut budget = AppearanceBudget {
        bytes: 0,
        nodes: 0,
        stack: HashSet::new(),
    };
    for field in &form.fields {
        for widget in &field.info.widgets {
            let id = crate::pdf_objects::parse_key(&widget.id)?;
            let mut stream = normal_appearance(&document, field, id)
                .map_err(|error| field_error(field, error))?;
            selected_list_values_fit(&document, field, id, &stream)
                .map_err(|error| field_error(field, error))?;
            let placement = placement_matrix(&document, &stream, widget.rect)
                .map_err(|error| field_error(field, error))?;
            let resources = validate_resources(&document, &stream, &fallback, &mut budget, 0)
                .map_err(|error| field_error(field, error))?;
            stream.dict.set("Resources", resources);
            let appearance = page_appearance(&document, stream, widget.rect, placement)
                .map_err(|error| field_error(field, error))?;
            let appearance = document.add_object(appearance);
            pages
                .entry(widget.page)
                .or_default()
                .push((widget.id.clone(), appearance));
        }
    }
    for (index, appearances) in pages {
        let page = page_id(&document, index)?;
        let mut resources = inherited(&document, page, b"Resources")?
            .map(|value| resolve(&document, &value).and_then(|value| Ok(value.as_dict()?.clone())))
            .transpose()?
            .unwrap_or_default();
        let mut xobjects = optional(&document, &resources, b"XObject")?
            .map(Object::as_dict)
            .transpose()?
            .cloned()
            .unwrap_or_default();
        let original = page_content(&document, page)?;
        checked_content(&original)?;
        let mut operations = vec![];
        for (index, (_, appearance)) in appearances.iter().enumerate() {
            let mut suffix = index;
            let name = loop {
                let name = format!("FolioStaticForm{suffix}");
                if !xobjects.has(name.as_bytes()) {
                    break name;
                }
                suffix += 1;
            };
            xobjects.set(name.as_bytes(), *appearance);
            operations.extend([
                Operation::new("q", vec![]),
                Operation::new("Do", vec![Object::Name(name.into_bytes())]),
                Operation::new("Q", vec![]),
            ]);
        }
        let mut bytes = b"q\n".to_vec();
        bytes.extend(original);
        bytes.extend(b"\nQ\n");
        bytes.extend(Content { operations }.encode()?);
        if bytes.len() > MAX_APPEARANCE_BYTES {
            return Err(invalid(
                "A flattened page exceeds the 32 MiB content limit.",
            ));
        }
        let contents = document.add_object(Stream::new(Dictionary::new(), bytes));
        resources.set("XObject", xobjects);
        let annotations = annotation_entries(&document, page)?
            .into_iter()
            .filter(|entry| {
                entry
                    .as_reference()
                    .ok()
                    .is_none_or(|id| !appearances.iter().any(|(widget, ..)| *widget == key(id)))
            })
            .collect::<Vec<_>>();
        let page = document.get_dictionary_mut(page)?;
        page.set("Resources", resources);
        page.set("Contents", contents);
        page.set("Annots", annotations);
    }
    document.catalog_mut()?.remove(b"AcroForm");
    document.prune_objects();
    // AcroForm/field objects are no longer reachable. No widget, field action
    // or plaintext password value is retained merely as an orphan object.
    for object in document.objects.values() {
        if let Ok(dictionary) = object.as_dict()
            && dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
        {
            return Err(invalid(
                "A widget remains reachable outside the form tree. No static copy was created.",
            ));
        }
    }
    crate::protection::serialize(&document)
}

fn page_content(document: &Document, page: ObjectId) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    let Some(value) = optional(document, document.get_dictionary(page)?, b"Contents")? else {
        return Ok(bytes);
    };
    let streams = match value {
        Object::Array(values) if values.len() <= 4096 => values.iter().collect::<Vec<_>>(),
        Object::Stream(_) => vec![value],
        _ => return Err(invalid("A page has invalid or excessive content streams.")),
    };
    for value in streams {
        let stream = resolve(document, value)?.as_stream()?;
        if stream.dict.has(b"F") {
            return Err(invalid("External page content cannot be safely flattened."));
        }
        let content = crate::pdf_objects::stream_bytes(stream, MAX_APPEARANCE_BYTES)?;
        if content.len() + bytes.len() + 1 > MAX_APPEARANCE_BYTES {
            return Err(invalid("A page exceeds the 32 MiB decoded-content limit."));
        }
        bytes.extend(content);
        bytes.push(b'\n');
    }
    Ok(bytes)
}
