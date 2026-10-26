use super::*;
use crate::{
    document::{annotation_entries, page_id},
    page_geometry::PageGeometry,
};

pub const MAX_DESIGN_CHANGES: usize = 64;
pub const MAX_NEW_WIDGETS: usize = 64;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Text,
    Checkbox,
    Radio,
    List,
    Combo,
    Signature,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionDefinition {
    pub label: String,
    pub export_value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Definition {
    pub kind: ControlKind,
    pub name: String,
    pub label: String,
    pub required: bool,
    pub read_only: bool,
    pub multiline: bool,
    pub password: bool,
    pub comb: bool,
    pub max_length: usize,
    pub multiple: bool,
    pub editable_choice: bool,
    pub no_toggle_to_off: bool,
    pub options: Vec<OptionDefinition>,
    pub default_value: Option<FieldValue>,
    pub font_size: f32,
    pub alignment: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetPlacement {
    pub page: usize,
    /// Physical points from the rotated visible page's top-left: x, y, w, h.
    pub placement: [f32; 4],
    pub option: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Change {
    Create {
        definition: Definition,
        widgets: Vec<WidgetPlacement>,
    },
    Update {
        id: String,
        definition: Definition,
    },
    PlaceWidget {
        id: String,
        widget_id: String,
        widget: WidgetPlacement,
    },
    RemoveWidget {
        id: String,
        widget_id: String,
    },
    RemoveField {
        id: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignField {
    pub field: FormField,
    pub definition: Option<Definition>,
    pub editable: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignerInfo {
    pub fields: Vec<DesignField>,
    pub authorable: bool,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
}

fn control_kind(field: &Field) -> Result<ControlKind> {
    Ok(match field.info.kind {
        FieldKind::Text => ControlKind::Text,
        FieldKind::Checkbox => ControlKind::Checkbox,
        FieldKind::Radio => ControlKind::Radio,
        FieldKind::Choice if field.info.combo => ControlKind::Combo,
        FieldKind::Choice => ControlKind::List,
        FieldKind::Signature => ControlKind::Signature,
        _ => {
            return Err(invalid(
                "Push buttons and unknown field types cannot be authored.",
            ));
        }
    })
}

pub(super) fn acroform(document: &Document) -> Result<Dictionary> {
    Ok(optional(document, document.catalog()?, b"AcroForm")?
        .map(Object::as_dict)
        .transpose()?
        .cloned()
        .unwrap_or_else(|| dictionary! { "Fields" => Vec::<Object>::new() }))
}

pub(super) fn set_acroform(document: &mut Document, form: Dictionary) -> Result<()> {
    let location = document.catalog()?.get(b"AcroForm").ok().cloned();
    if let Some(Object::Reference(id)) = location {
        let mut current = id;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current) || visited.len() > 32 {
                return Err(invalid("The AcroForm reference is cyclic."));
            }
            if let Object::Reference(next) = document.get_object(current)? {
                current = *next;
            } else {
                document.set_object(current, form);
                return Ok(());
            }
        }
    }
    document.catalog_mut()?.set("AcroForm", form);
    Ok(())
}

fn ensure_form(document: &mut Document) -> Result<Form> {
    set_acroform(document, acroform(document)?)?;
    let form = read(document)?;
    if let Some(reason) = &form.reason {
        return Err(invalid(reason));
    }
    check_names(&form)?;
    Ok(form)
}

pub(super) fn check_names(form: &Form) -> Result<()> {
    let mut names = HashSet::new();
    if form
        .fields
        .iter()
        .any(|field| !names.insert(&field.info.name))
    {
        return Err(invalid(
            "Distinct logical fields share a qualified name. Ambiguous duplicate fields cannot be authored or flattened.",
        ));
    }
    Ok(())
}

// Read-only is a filling constraint, not a prohibition on an owner-authorized
// static copy or property edit. All other capabilities are checked separately.
pub(super) fn supported(document: &Document, field: &Field) -> Result<()> {
    control_kind(field)?;
    let flags = integer(document, &field.attributes, b"Ff", 0)?;
    let common = 1 | 2 | 4;
    let allowed = common
        | match field.info.kind {
            FieldKind::Text => (1 << 12) | (1 << 13) | (1 << 22) | (1 << 23) | (1 << 24),
            FieldKind::Choice => {
                (1 << 17) | (1 << 18) | (1 << 19) | (1 << 21) | (1 << 22) | (1 << 26)
            }
            FieldKind::Radio => (1 << 14) | (1 << 15) | (1 << 25),
            _ => 0,
        };
    if flags < 0 || flags & !allowed != 0 {
        return Err(invalid(
            "This field has unsupported rich-text, file-selection, button, or other flags.",
        ));
    }
    let mut nodes = field_ancestors(document, field.id)?;
    nodes.push(field.id);
    nodes.extend(field.widgets.iter().copied());
    for node in nodes {
        let dict = document.get_dictionary(node)?;
        if [b"A".as_slice(), b"AA", b"RV", b"RichText", b"Lock", b"SV"]
            .iter()
            .any(|key| dict.has(key))
        {
            return Err(invalid(
                "Fields with actions, scripts, rich text, signature locks or seed policies cannot be authored or flattened. No scripts are executed.",
            ));
        }
    }
    if field.widgets.is_empty() || field.widgets.len() != field.info.widgets.len() {
        return Err(invalid(
            "Every field needs an unambiguous, visible widget on exactly one page.",
        ));
    }
    if field.info.password && field.info.multiline
        || field.info.comb
            && (field.info.multiline
                || field.info.password
                || !field.attributes.has(b"MaxLen")
                || integer(document, &field.attributes, b"MaxLen", 0)? > 256)
        || field.info.combo && field.info.multiple
        || field.info.kind == FieldKind::Choice && !field.info.combo && flags & (1 << 18) != 0
    {
        return Err(invalid(
            "This field combines unsupported text or choice constraints.",
        ));
    }
    if integer(
        document,
        &field.attributes,
        b"MaxLen",
        MAX_FIELD_CHARACTERS as i64,
    )? > MAX_FIELD_CHARACTERS as i64
    {
        return Err(invalid(
            "Authoring and static copies support at most 4,000 characters per field.",
        ));
    }
    for widget in &field.widgets {
        let annotation = document.get_dictionary(*widget)?;
        if *widget != field.id
            && [
                b"FT".as_slice(),
                b"Ff",
                b"V",
                b"DV",
                b"Opt",
                b"MaxLen",
                b"DA",
                b"Q",
            ]
            .iter()
            .any(|key| annotation.has(key))
        {
            return Err(invalid(
                "A separate widget contains overriding field attributes. This ambiguous field/widget structure is not supported.",
            ));
        }
        let flags = integer(document, annotation, b"F", 0)?;
        if flags & (1 | 2 | 8 | 16 | 32 | 256) != 0 || annotation.has(b"OC") {
            return Err(invalid(
                "Hidden, invisible, no-view, no-zoom/no-rotate, toggle-view and optional-content widgets are not supported. Their values will not be silently omitted.",
            ));
        }
        let info = field
            .info
            .widgets
            .iter()
            .find(|item| item.id == key(*widget))
            .ok_or_else(|| invalid("The widget's page mapping is ambiguous."))?;
        let page = page_id(document, info.page)?;
        if let Ok(value) = annotation.get(b"P")
            && value.as_reference()? != page
        {
            return Err(invalid("A widget's page reference is inconsistent."));
        }
        let rect = widget_rect(document, annotation, page)?;
        let bounds = page_bounds(document, page)?;
        if rect[0] < bounds[0] || rect[1] < bounds[1] || rect[2] > bounds[2] || rect[3] > bounds[3]
        {
            return Err(invalid(
                "The whole widget must lie inside the visible crop. Partly cropped values are not silently discarded.",
            ));
        }
        if matches!(field.info.kind, FieldKind::Text | FieldKind::Choice) {
            style(document, *widget, &field.attributes)?;
        }
    }
    if matches!(field.info.kind, FieldKind::Checkbox | FieldKind::Radio) {
        let mut states = HashMap::new();
        let exports = optional(document, &field.attributes, b"Opt")?
            .map(Object::as_array)
            .transpose()?;
        if exports.is_some_and(|exports| exports.len() != field.widgets.len()) {
            return Err(invalid(
                "Button export options must map one-to-one to widgets.",
            ));
        }
        let mut by_state = HashMap::new();
        for (index, widget) in field.widgets.iter().enumerate() {
            let on = button_state(document, *widget)?;
            let export = exports
                .map(|exports| text(document, &exports[index], &mut 0))
                .transpose()?;
            if let Some(previous) = by_state.insert(on.clone(), export.clone())
                && previous != export
            {
                return Err(invalid(
                    "Repeated button widgets disagree on their export values.",
                ));
            }
            *states.entry(on).or_insert(0) += 1;
        }
        if field.info.kind == FieldKind::Checkbox && states.len() != 1
            || field.info.kind == FieldKind::Radio
                && flags & (1 << 25) == 0
                && states.values().any(|count| *count > 1)
        {
            return Err(invalid(
                "The button widgets have ambiguous repeated export states.",
            ));
        }
        let value = optional(document, &field.attributes, b"V")?
            .map(Object::as_name)
            .transpose()?
            .unwrap_or(b"Off");
        if value != b"Off" && !states.contains_key(value) {
            return Err(invalid(
                "The button value is not represented by its widgets.",
            ));
        }
    }
    if field.info.kind == FieldKind::Choice {
        let mut exports = HashSet::new();
        if field.info.options.is_empty() && !field.info.editable_choice
            || field.info.options.iter().any(|option| {
                option.label.chars().any(char::is_control)
                    || !exports.insert(option.export_value.clone())
            })
        {
            return Err(invalid(
                "Choices need unambiguous, unique export values and single-line display labels.",
            ));
        }
    }
    if field.info.kind == FieldKind::Signature
        && optional(document, &field.attributes, b"V")?.is_some()
    {
        return Err(invalid("Signed fields cannot be rewritten or flattened."));
    }
    Ok(())
}

fn default_value(document: &Document, field: &Field) -> Result<Option<FieldValue>> {
    let Some(value) = optional(document, &field.attributes, b"DV")? else {
        return Ok(None);
    };
    Ok(Some(match field.info.kind {
        FieldKind::Text => FieldValue::Text {
            text: normalized(&text(document, value, &mut 0)?),
        },
        FieldKind::Checkbox | FieldKind::Radio => FieldValue::Button {
            state: match value.as_name()? {
                b"Off" => None,
                state => Some(state_key(state)),
            },
        },
        FieldKind::Choice => {
            let values = if let Object::Array(values) = value {
                values.iter().collect::<Vec<_>>()
            } else {
                vec![value]
            };
            let mut indices = vec![];
            let mut custom = None;
            for value in values {
                let value = text(document, value, &mut 0)?;
                if let Some(index) = field
                    .info
                    .options
                    .iter()
                    .position(|option| option.export_value == value)
                {
                    indices.push(index);
                } else if field.info.editable_choice && indices.is_empty() && custom.is_none() {
                    custom = Some(value);
                } else if !value.is_empty() {
                    return Err(invalid(
                        "The default choice is not represented by this field's options.",
                    ));
                }
            }
            indices.sort_unstable();
            indices.dedup();
            FieldValue::Choice { indices, custom }
        }
        _ => {
            return Err(invalid(
                "Signature fields must not contain default signature values.",
            ));
        }
    }))
}

fn definition(document: &Document, field: &Field) -> Result<Definition> {
    let dict = document.get_dictionary(field.id)?;
    let kind = control_kind(field)?;
    let options = field
        .info
        .options
        .iter()
        .map(|option| {
            let export_value = if matches!(kind, ControlKind::Checkbox | ControlKind::Radio) {
                let widget = field
                    .widgets
                    .iter()
                    .find(|widget| {
                        button_state(document, **widget)
                            .is_ok_and(|state| state_key(&state) == option.id)
                    })
                    .ok_or_else(|| invalid("A button export state is missing."))?;
                String::from_utf8(button_state(document, *widget)?).map_err(|_| {
                    invalid("Authoring button export names requires portable ASCII state names.")
                })?
            } else {
                option.export_value.clone()
            };
            Ok(OptionDefinition {
                label: option.label.clone(),
                export_value,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let styling = if matches!(
        kind,
        ControlKind::Text | ControlKind::List | ControlKind::Combo
    ) {
        Some(style(document, field.widgets[0], &field.attributes)?)
    } else {
        None
    };
    Ok(Definition {
        kind,
        name: optional(document, dict, b"T")?
            .map(|value| text(document, value, &mut 0))
            .transpose()?
            .unwrap_or_default(),
        label: field.info.label.clone(),
        required: field.info.required,
        read_only: integer(document, &field.attributes, b"Ff", 0)? & 1 != 0,
        multiline: field.info.multiline,
        password: field.info.password,
        comb: field.info.comb,
        max_length: field.info.max_length,
        multiple: field.info.multiple,
        editable_choice: field.info.editable_choice,
        no_toggle_to_off: field.info.no_toggle_to_off,
        options,
        default_value: default_value(document, field)?,
        font_size: styling.as_ref().map_or(0.0, |style| style.size),
        alignment: styling.as_ref().map_or(0, |style| style.align),
    })
}

pub(crate) fn inspect(
    document: &Document,
    allowed: bool,
    permission_reason: Option<&str>,
) -> Result<DesignerInfo> {
    let mut inspected = document.clone();
    set_acroform(&mut inspected, acroform(document)?)?;
    let form = read(&inspected)?;
    let reason = form
        .reason
        .clone()
        .or_else(|| check_names(&form).err().map(|error| error.to_string()))
        .or_else(|| {
            (!allowed).then(|| {
                permission_reason.unwrap_or(
        "Form design requires authorized general editing, not only permission to fill fields."
    ).to_owned()
            })
        });
    let mut fields = vec![];
    for field in form.fields {
        let properties = supported(document, &field).and_then(|()| definition(document, &field));
        let (definition, field_reason) = match properties {
            Ok(definition) => (Some(definition), reason.clone()),
            Err(error) => (None, Some(error.to_string())),
        };
        fields.push(DesignField {
            field: field.info,
            editable: field_reason.is_none(),
            definition,
            reason: field_reason,
        });
    }
    Ok(DesignerInfo {
        fields, authorable: reason.is_none(), reason,
        warnings: vec![
            "Creates text, checkbox, radio, list, combo and empty digital-signature fields. Empty signature fields are unsigned, not signatures.".into(),
            "Names are partial names within the existing field tree. Property changes affect every widget of that logical field; layout/removal can target one widget.".into(),
            "Text/choice authoring rebuilds appearances with embedded Noto Sans. Current values are preserved; incompatible constraints/options fail instead of discarding values.".into(),
            "Geometry uses physical points from the visible rotated page's top-left. Widgets must remain inside the crop and at least 4 PDF units wide/high. Batch limit: 64 changes; 64 widgets per new field.".into(),
            "Rich text, XFA, scripts/calculations, ambiguous trees, hidden/optional-content widgets and signature seed/lock policies are unsupported. Existing page-operation form guards remain in place.".into(),
        ],
    })
}

fn single_line(value: &str, limit: usize, name: &str, empty: bool) -> Result<()> {
    if (!empty && value.trim().is_empty())
        || value.chars().count() > limit
        || value.chars().any(char::is_control)
    {
        return Err(invalid(format!(
            "{name} must be a single line of {} to {limit} characters.",
            if empty { 0 } else { 1 }
        )));
    }
    Ok(())
}

fn validate_definition(definition: &Definition) -> Result<()> {
    single_line(&definition.name, 256, "Field name", false)?;
    single_line(&definition.label, 256, "Field label", true)?;
    if definition.name.contains('.') {
        return Err(invalid(
            "A partial field name cannot contain a period. Existing parent namespaces are preserved.",
        ));
    }
    if definition.max_length == 0
        || definition.max_length > MAX_FIELD_CHARACTERS
        || definition.comb
            && (definition.max_length > 256 || definition.multiline || definition.password)
        || definition.multiline && definition.password
    {
        return Err(invalid(
            "Text fields support 1-4,000 characters; comb fields support up to 256 and cannot be multiline or password fields.",
        ));
    }
    if !definition.font_size.is_finite()
        || definition.font_size != 0.0 && !(4.0..=256.0).contains(&definition.font_size)
        || !(0..=2).contains(&definition.alignment)
    {
        return Err(invalid(
            "Font size must be 0 (automatic) or 4-256 PDF units; alignment must be left, center or right.",
        ));
    }
    if definition.kind != ControlKind::Text
        && (definition.multiline || definition.password || definition.comb)
        || definition.kind != ControlKind::List && definition.multiple
        || definition.kind != ControlKind::Combo && definition.editable_choice
        || definition.kind != ControlKind::Radio && definition.no_toggle_to_off
    {
        return Err(invalid(
            "These constraints do not apply to the selected control type.",
        ));
    }
    let needs_options = matches!(
        definition.kind,
        ControlKind::Checkbox | ControlKind::Radio | ControlKind::List | ControlKind::Combo
    );
    if needs_options && definition.options.is_empty()
        || !needs_options && !definition.options.is_empty()
        || definition.options.len() > MAX_OPTIONS
        || definition.kind == ControlKind::Checkbox && definition.options.len() != 1
    {
        return Err(invalid(
            "Provide options for button/choice controls only, exactly one checkbox option, and at most 4,096 options.",
        ));
    }
    let mut seen = HashSet::new();
    for option in &definition.options {
        single_line(&option.label, 256, "Option label", false)?;
        single_line(&option.export_value, 256, "Option export value", false)?;
        if !seen.insert(option.export_value.clone()) {
            return Err(invalid("Option export values must be unique."));
        }
        if matches!(definition.kind, ControlKind::Checkbox | ControlKind::Radio)
            && (option.export_value == "Off"
                || !option
                    .export_value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte)))
        {
            return Err(invalid(
                "Button export states use ASCII letters, numbers, underscore, hyphen or period, and cannot be Off.",
            ));
        }
    }
    if definition.kind == ControlKind::Signature && definition.default_value.is_some() {
        return Err(invalid(
            "An unsigned digital-signature field cannot have a default signature value.",
        ));
    }
    Ok(())
}

fn ensure_unique(form: &Form, name: &str, except: Option<ObjectId>) -> Result<()> {
    if form.fields.iter().any(|field| {
        Some(field.id) != except
            && (field.info.name == name
                || field.info.name.starts_with(&format!("{name}."))
                || name.starts_with(&format!("{}.", field.info.name)))
    }) {
        return Err(invalid(
            "That field name conflicts with an existing field or namespace. Use a unique name.",
        ));
    }
    Ok(())
}

fn set_properties(
    document: &mut Document,
    id: ObjectId,
    definition: &Definition,
    old: Option<&Field>,
) -> Result<()> {
    let old_flags = old
        .map(|field| integer(document, &field.attributes, b"Ff", 0))
        .transpose()?
        .unwrap_or(0);
    let managed = 1
        | 2
        | (1 << 12)
        | (1 << 13)
        | (1 << 14)
        | (1 << 15)
        | (1 << 17)
        | (1 << 18)
        | (1 << 21)
        | (1 << 24);
    let mut flags = old_flags & !managed;
    for (enabled, flag) in [
        (definition.read_only, 1),
        (definition.required, 2),
        (definition.multiline, 1 << 12),
        (definition.password, 1 << 13),
        (definition.no_toggle_to_off, 1 << 14),
        (definition.kind == ControlKind::Radio, 1 << 15),
        (definition.kind == ControlKind::Combo, 1 << 17),
        (definition.editable_choice, 1 << 18),
        (definition.multiple, 1 << 21),
        (definition.comb, 1 << 24),
    ] {
        if enabled {
            flags |= flag;
        }
    }
    let color = old
        .filter(|field| matches!(field.info.kind, FieldKind::Text | FieldKind::Choice))
        .map(|field| style(document, field.widgets[0], &field.attributes).map(|style| style.color))
        .transpose()?
        .unwrap_or([0.0; 3]);
    let dictionary = document.get_dictionary_mut(id)?;
    dictionary.set("T", text_string(&definition.name));
    dictionary.set("TU", text_string(&definition.label));
    dictionary.set(
        "FT",
        match definition.kind {
            ControlKind::Text => "Tx",
            ControlKind::Checkbox | ControlKind::Radio => "Btn",
            ControlKind::List | ControlKind::Combo => "Ch",
            ControlKind::Signature => "Sig",
        },
    );
    dictionary.set("Ff", flags);
    if definition.kind == ControlKind::Text {
        dictionary.set("MaxLen", definition.max_length as i64);
    }
    if matches!(
        definition.kind,
        ControlKind::Text | ControlKind::List | ControlKind::Combo
    ) {
        dictionary.set("Q", definition.alignment);
        dictionary.set(
            "DA",
            Object::string_literal(format!(
                "/FolioPending {} Tf {} {} {} rg",
                definition.font_size, color[0], color[1], color[2]
            )),
        );
    }
    if matches!(definition.kind, ControlKind::List | ControlKind::Combo) {
        dictionary.set(
            "Opt",
            definition
                .options
                .iter()
                .map(|option| {
                    Object::Array(vec![
                        text_string(&option.export_value),
                        text_string(&option.label),
                    ])
                })
                .collect::<Vec<_>>(),
        );
        dictionary.remove(b"I");
    }
    Ok(())
}

fn default_object(field: &Field, value: &FieldValue) -> Result<Object> {
    let mut stored = field.clone();
    stored.info.read_only = false;
    stored.info.required = false;
    stored.info.no_toggle_to_off = false;
    Ok(match validate_value(&stored, value)? {
        FieldValue::Text { text } => text_string(&text.replace('\n', "\r")),
        FieldValue::Button { state } => {
            let bytes = if let Some(state) = state {
                let mut bytes = vec![];
                for pair in state.as_bytes().chunks(2) {
                    if pair.len() != 2 {
                        return Err(invalid("Invalid button state identifier."));
                    }
                    let pair = std::str::from_utf8(pair)
                        .map_err(|_| invalid("Invalid button state identifier."))?;
                    bytes.push(
                        u8::from_str_radix(pair, 16)
                            .map_err(|_| invalid("Invalid button state identifier."))?,
                    );
                }
                bytes
            } else {
                b"Off".to_vec()
            };
            Object::Name(bytes)
        }
        FieldValue::Choice { indices, custom } => {
            if let Some(custom) = custom {
                text_string(&custom)
            } else if field.info.multiple {
                Object::Array(
                    indices
                        .iter()
                        .map(|index| field.exports[*index].clone())
                        .collect(),
                )
            } else {
                indices
                    .first()
                    .map(|index| field.exports[*index].clone())
                    .unwrap_or(Object::Null)
            }
        }
    })
}

fn set_default(
    document: &mut Document,
    field: &Field,
    value: Option<&FieldValue>,
    form: &Form,
) -> Result<()> {
    let value = value
        .map(|value| default_object(field, value))
        .transpose()?
        .filter(|value| !matches!(value, Object::Null));
    if let Some(value) = value {
        document.get_dictionary_mut(field.id)?.set("DV", value);
    } else {
        let ancestors = field_ancestors(document, field.id)?
            .into_iter()
            .filter(|id| {
                document
                    .get_dictionary(*id)
                    .is_ok_and(|dict| dict.has(b"DV"))
            })
            .collect::<HashSet<_>>();
        for sibling in &form.fields {
            if field_ancestors(document, sibling.id)?
                .iter()
                .any(|id| ancestors.contains(id))
                && let Ok(value) = sibling.attributes.get(b"DV")
            {
                document
                    .get_dictionary_mut(sibling.id)?
                    .set("DV", value.clone());
            }
        }
        for ancestor in ancestors {
            document.get_dictionary_mut(ancestor)?.remove(b"DV");
        }
        document.get_dictionary_mut(field.id)?.remove(b"DV");
    }
    Ok(())
}

fn empty_value(kind: ControlKind) -> Option<FieldValue> {
    match kind {
        ControlKind::Text => Some(FieldValue::Text {
            text: String::new(),
        }),
        ControlKind::Checkbox | ControlKind::Radio => Some(FieldValue::Button { state: None }),
        ControlKind::List | ControlKind::Combo => Some(FieldValue::Choice {
            indices: vec![],
            custom: None,
        }),
        ControlKind::Signature => None,
    }
}

fn button_appearance(document: &mut Document, radio: bool, on: bool, matrix: [f32; 6]) -> ObjectId {
    let border = if radio {
        "10 1 m 14.9706 1 19 5.0294 19 10 c 19 14.9706 14.9706 19 10 19 c 5.0294 19 1 14.9706 1 10 c 1 5.0294 5.0294 1 10 1 c"
    } else {
        "1 1 18 18 re"
    };
    let mark = if !on {
        ""
    } else if radio {
        "10 5 m 12.7614 5 15 7.2386 15 10 c 15 12.7614 12.7614 15 10 15 c 7.2386 15 5 12.7614 5 10 c 5 7.2386 7.2386 5 10 5 c f"
    } else {
        "3 w 4 10 m 8 5 l 16 15 l S"
    };
    document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, 20.0, 20.0]),
            "Resources" => dictionary! {}, "Matrix" => numbers(&matrix),
        },
        format!("q 1 g {border} f 0 G 1 w {border} S 0 g {mark} Q").into_bytes(),
    ))
}

fn append_widget(
    document: &mut Document,
    id: ObjectId,
    definition: &Definition,
    placement: &WidgetPlacement,
) -> Result<ObjectId> {
    let page = page_id(document, placement.page)?;
    let geometry = PageGeometry::for_page(document, page)?;
    let rect = geometry.display_rect(placement.placement)?;
    let mut widget = dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Parent" => id, "P" => page,
        "Rect" => numbers(&rect), "F" => 4,
        "MK" => dictionary! { "R" => if definition.kind == ControlKind::Signature { 0 } else { geometry.rotation }, "BG" => numbers(&[1.0; 3]), "BC" => numbers(&[0.25; 3]) },
        "BS" => dictionary! { "W" => 1, "S" => "S" },
    };
    widget_rect(document, &widget, page)?;
    if matches!(definition.kind, ControlKind::Checkbox | ControlKind::Radio) {
        let index = if definition.kind == ControlKind::Checkbox {
            0
        } else {
            placement
                .option
                .ok_or_else(|| invalid("Every radio widget needs an option index."))?
        };
        let option = definition
            .options
            .get(index)
            .ok_or_else(|| invalid("The radio option index is out of range."))?;
        let matrix = geometry.appearance_matrix(20.0, 20.0);
        let off = button_appearance(
            document,
            definition.kind == ControlKind::Radio,
            false,
            matrix,
        );
        let on = button_appearance(
            document,
            definition.kind == ControlKind::Radio,
            true,
            matrix,
        );
        let mut normal = dictionary! { "Off" => off };
        normal.set(option.export_value.as_bytes(), on);
        widget.set("AP", dictionary! { "N" => normal });
        widget.set("AS", "Off");
    } else if definition.kind == ControlKind::Signature {
        let appearance = document.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form", "BBox" => numbers(&[0.0, 0.0, 1.0, 1.0]),
                "Resources" => dictionary! {}, "Matrix" => numbers(&geometry.appearance_matrix(1.0, 1.0)),
            },
            b"q 0.6 G 0.01 w 0.005 0.005 0.99 0.99 re S Q".to_vec(),
        ));
        widget.set("AP", dictionary! { "N" => appearance });
    }
    let widget = document.add_object(widget);
    let mut annotations = annotation_entries(document, page)?;
    annotations.push(widget.into());
    document
        .get_dictionary_mut(page)?
        .set("Annots", annotations);
    Ok(widget)
}

fn update_button_labels(
    document: &mut Document,
    field: &Field,
    definition: &Definition,
) -> Result<()> {
    if !matches!(definition.kind, ControlKind::Checkbox | ControlKind::Radio) {
        return Ok(());
    }
    let labels = field.widgets.iter().map(|widget| {
        let on = button_state(document, *widget)?;
        let option = definition.options.iter().find(|option| option.export_value.as_bytes() == on)
            .ok_or_else(|| invalid("Existing button export states cannot be changed or removed. Create a new group instead."))?;
        Ok((*widget, text_string(&option.label), text_string(&option.export_value)))
    }).collect::<Result<Vec<_>>>()?;
    let states = field
        .info
        .options
        .iter()
        .map(|option| option.id.clone())
        .collect::<HashSet<_>>();
    if definition.options.len() != states.len()
        || definition
            .options
            .iter()
            .any(|option| !states.contains(&state_key(option.export_value.as_bytes())))
    {
        return Err(invalid(
            "Existing button options may be relabeled, but their export-state identities must remain unchanged.",
        ));
    }
    for (widget, label, _) in &labels {
        document
            .get_dictionary_mut(*widget)?
            .set("Contents", label.clone());
    }
    // Opt holds submission exports, not display captions. Preserve existing
    // mappings, including inherited exports different from appearance states.
    if !field.attributes.has(b"Opt") {
        document.get_dictionary_mut(field.id)?.set(
            "Opt",
            labels
                .into_iter()
                .map(|(_, _, export)| export)
                .collect::<Vec<_>>(),
        );
    }
    Ok(())
}

fn refresh(document: &mut Document, id: ObjectId, value: Option<FieldValue>) -> Result<()> {
    if let Some(value) = value {
        write_values(document, vec![FieldChange { id: key(id), value }], false)?;
    }
    Ok(())
}

fn create(
    document: &mut Document,
    definition: Definition,
    widgets: Vec<WidgetPlacement>,
) -> Result<()> {
    validate_definition(&definition)?;
    let form = read(document)?;
    ensure_unique(&form, &definition.name, None)?;
    if widgets.is_empty()
        || widgets.len() > MAX_NEW_WIDGETS
        || definition.kind == ControlKind::Signature && widgets.len() != 1
        || definition.kind == ControlKind::Radio
            && (widgets.len() != definition.options.len()
                || widgets
                    .iter()
                    .filter_map(|widget| widget.option)
                    .collect::<HashSet<_>>()
                    .len()
                    != widgets.len())
        || definition.kind != ControlKind::Radio
            && widgets.iter().any(|widget| widget.option.is_some())
    {
        return Err(invalid(
            "Use 1-64 widgets, exactly one per radio option, one for an unsigned signature, and no option index on other controls.",
        ));
    }
    let id = document.add_object(Dictionary::new());
    set_properties(document, id, &definition, None)?;
    let widgets = widgets
        .iter()
        .map(|widget| append_widget(document, id, &definition, widget))
        .collect::<Result<Vec<_>>>()?;
    document.get_dictionary_mut(id)?.set(
        "Kids",
        widgets
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    let mut acroform = acroform(document)?;
    let mut roots = optional(document, &acroform, b"Fields")?
        .map(Object::as_array)
        .transpose()?
        .cloned()
        .unwrap_or_default();
    roots.push(id.into());
    acroform.set("Fields", roots);
    set_acroform(document, acroform)?;
    let form = read(document)?;
    let field = form
        .fields
        .iter()
        .find(|field| field.id == id)
        .ok_or_else(|| invalid("The created field could not be inspected."))?;
    if matches!(definition.kind, ControlKind::Checkbox | ControlKind::Radio) {
        update_button_labels(document, field, &definition)?;
    }
    set_default(document, field, definition.default_value.as_ref(), &form)?;
    refresh(
        document,
        id,
        definition
            .default_value
            .clone()
            .or_else(|| empty_value(definition.kind)),
    )?;
    Ok(())
}

fn field<'a>(form: &'a Form, id: &str) -> Result<&'a Field> {
    form.fields
        .iter()
        .find(|field| field.info.id == id)
        .ok_or_else(|| {
            invalid(
                "This logical field no longer exists. Reload the designer; your draft is retained.",
            )
        })
}

fn update(document: &mut Document, form: &Form, id: String, definition: Definition) -> Result<()> {
    validate_definition(&definition)?;
    let field = field(form, &id)?;
    supported(document, field)?;
    if control_kind(field)? != definition.kind {
        return Err(invalid(
            "Changing an existing control's type is not supported. Create a new field instead.",
        ));
    }
    let ancestors = field_ancestors(document, field.id)?;
    let mut namespace = vec![];
    for ancestor in ancestors.iter().rev() {
        if let Some(value) = optional(document, document.get_dictionary(*ancestor)?, b"T")? {
            let partial = text(document, value, &mut 0)?;
            if !partial.is_empty() {
                namespace.push(partial);
            }
        }
    }
    namespace.push(definition.name.clone());
    ensure_unique(form, &namespace.join("."), Some(field.id))?;
    let mut value = field.info.value.clone();
    if let Some(FieldValue::Choice {
        indices,
        custom: None,
    }) = &mut value
    {
        let mut mapped = vec![];
        for index in indices.iter() {
            let export = &field.info.options[*index].export_value;
            mapped.push(definition.options.iter().position(|option| &option.export_value == export)
                .ok_or_else(|| invalid("An option holding a current value cannot be removed. Clear or change the value in Fill forms first."))?);
        }
        mapped.sort_unstable();
        *indices = mapped;
    }
    set_properties(document, field.id, &definition, Some(field))?;
    update_button_labels(document, field, &definition)?;
    let updated = read(document)?;
    let changed = self::field(&updated, &id)?;
    set_default(document, changed, definition.default_value.as_ref(), form)?;
    refresh(document, field.id, value)
}

fn remove_annotation(document: &mut Document, page: usize, id: ObjectId) -> Result<()> {
    let page = page_id(document, page)?;
    let annotations = annotation_entries(document, page)?
        .into_iter()
        .filter(|entry| entry.as_reference().ok() != Some(id))
        .collect::<Vec<_>>();
    document
        .get_dictionary_mut(page)?
        .set("Annots", annotations);
    Ok(())
}

fn check_removal_references(document: &Document, removed: &HashSet<ObjectId>) -> Result<()> {
    fn count(
        value: &Object,
        removed: &HashSet<ObjectId>,
        counts: &mut HashMap<ObjectId, usize>,
        depth: usize,
        budget: &mut usize,
    ) -> Result<()> {
        *budget += 1;
        if depth > 64 || *budget > 500_000 {
            return Err(invalid(
                "The PDF exceeds the bounded reference inspection needed for safe field removal.",
            ));
        }
        match value {
            Object::Reference(id) if removed.contains(id) => {
                *counts.entry(*id).or_default() += 1;
            }
            Object::Array(values) => {
                for value in values {
                    count(value, removed, counts, depth + 1, budget)?;
                }
            }
            Object::Dictionary(dictionary) => {
                for (_, value) in dictionary {
                    count(value, removed, counts, depth + 1, budget)?;
                }
            }
            Object::Stream(stream) => {
                for (_, value) in &stream.dict {
                    count(value, removed, counts, depth + 1, budget)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut allowed: HashMap<ObjectId, HashMap<ObjectId, usize>> = HashMap::new();
    let mut containers = HashSet::new();
    let mut allow_array = |owner: ObjectId, dictionary: &Dictionary, key: &[u8]| -> Result<()> {
        let Ok(value) = dictionary.get(key) else {
            return Ok(());
        };
        let (location, value) = document.dereference(value)?;
        let owner = location.unwrap_or(owner);
        let identity = (
            owner,
            if location.is_some() {
                vec![]
            } else {
                key.to_vec()
            },
        );
        if !containers.insert(identity) {
            return Ok(());
        }
        for value in value.as_array()? {
            if let Object::Reference(id) = value
                && removed.contains(id)
            {
                *allowed.entry(owner).or_default().entry(*id).or_default() += 1;
            }
        }
        Ok(())
    };
    let catalog_id = document.trailer.get(b"Root")?.as_reference()?;
    let catalog = document.catalog()?;
    if let Ok(value) = catalog.get(b"AcroForm") {
        let (id, form) = document.dereference(value)?;
        allow_array(id.unwrap_or(catalog_id), form.as_dict()?, b"Fields")?;
    }
    let form = read(document)?;
    let mut nodes = HashSet::new();
    for field in &form.fields {
        nodes.insert(field.id);
        nodes.extend(field_ancestors(document, field.id)?);
    }
    for node in nodes {
        allow_array(node, document.get_dictionary(node)?, b"Kids")?;
    }
    for page in document.get_pages().into_values() {
        allow_array(page, document.get_dictionary(page)?, b"Annots")?;
    }
    let mut budget = 0;
    for (id, object) in &document.objects {
        if removed.contains(id) {
            continue;
        }
        let mut actual = HashMap::new();
        count(object, removed, &mut actual, 0, &mut budget)?;
        if actual.iter().any(|(target, count)| {
            *count
                > allowed
                    .get(id)
                    .and_then(|counts| counts.get(target))
                    .copied()
                    .unwrap_or(0)
        }) {
            return Err(invalid(
                "A field/widget is referenced outside its form tree or page annotations (for example tagged structure). Removal is unsupported; no references were broken.",
            ));
        }
    }
    let mut trailer = HashMap::new();
    count(
        &Object::Dictionary(document.trailer.clone()),
        removed,
        &mut trailer,
        0,
        &mut budget,
    )?;
    if !trailer.is_empty() {
        return Err(invalid(
            "The PDF trailer references a field/widget being removed. No references were broken.",
        ));
    }
    Ok(())
}

fn remove_field(document: &mut Document, field: &Field) -> Result<()> {
    let mut removed: HashSet<_> = field.widgets.iter().copied().collect();
    removed.insert(field.id);
    let mut node = field.id;
    for parent in field_ancestors(document, field.id)? {
        let dictionary = document.get_dictionary(parent)?;
        let children = resolve(document, dictionary.get(b"Kids")?)?.as_array()?;
        if children.len() != 1 || children[0].as_reference()? != node {
            break;
        }
        removed.insert(parent);
        node = parent;
    }
    check_removal_references(document, &removed)?;
    for widget in &field.info.widgets {
        let id = crate::pdf_objects::parse_key(&widget.id)?;
        remove_annotation(document, widget.page, id)?;
        if id != field.id {
            document.objects.remove(&id);
        }
    }
    let mut current = field.id;
    loop {
        let parent = document
            .get_dictionary(current)?
            .get(b"Parent")
            .ok()
            .map(Object::as_reference)
            .transpose()?;
        document.objects.remove(&current);
        if let Some(parent) = parent {
            let dictionary = document.get_dictionary(parent)?;
            let kids = resolve(document, dictionary.get(b"Kids")?)?
                .as_array()?
                .iter()
                .filter(|entry| entry.as_reference().ok() != Some(current))
                .cloned()
                .collect::<Vec<_>>();
            if kids.is_empty() {
                current = parent;
            } else {
                document.get_dictionary_mut(parent)?.set("Kids", kids);
                break;
            }
        } else {
            let mut acroform = acroform(document)?;
            let roots = resolve(document, acroform.get(b"Fields")?)?
                .as_array()?
                .iter()
                .filter(|entry| entry.as_reference().ok() != Some(current))
                .cloned()
                .collect::<Vec<_>>();
            acroform.set("Fields", roots);
            set_acroform(document, acroform)?;
            break;
        }
    }
    Ok(())
}

fn remove_widget(document: &mut Document, field: &Field, widget_id: &str) -> Result<()> {
    let index = field
        .info
        .widgets
        .iter()
        .position(|widget| widget.id == widget_id)
        .ok_or_else(|| invalid("This widget does not belong to the selected logical field."))?;
    if field.widgets.len() == 1 {
        return remove_field(document, field);
    }
    let id = crate::pdf_objects::parse_key(widget_id)?;
    check_removal_references(document, &HashSet::from([id]))?;
    if field.info.kind == FieldKind::Radio {
        let state = button_state(document, id)?;
        let remaining = field
            .widgets
            .iter()
            .filter(|widget| **widget != id)
            .any(|widget| button_state(document, *widget).is_ok_and(|on| on == state));
        let referenced = [b"V".as_slice(), b"DV"].iter().any(|key| {
            optional(document, &field.attributes, key)
                .ok()
                .flatten()
                .and_then(|value| value.as_name().ok())
                == Some(state.as_slice())
        });
        if !remaining && referenced {
            return Err(invalid(
                "This radio widget holds the current/default selection. Change those values before removing its last matching widget.",
            ));
        }
        if let Some(options) = optional(document, &field.attributes, b"Opt")? {
            let mut options = options.as_array()?.clone();
            if options.len() != field.widgets.len() {
                return Err(invalid(
                    "The radio export options do not map one-to-one to its widgets.",
                ));
            }
            options.remove(index);
            document.get_dictionary_mut(field.id)?.set("Opt", options);
        }
    }
    remove_annotation(document, field.info.widgets[index].page, id)?;
    let kids = field
        .widgets
        .iter()
        .filter(|widget| **widget != id)
        .copied()
        .map(Object::Reference)
        .collect::<Vec<_>>();
    document.get_dictionary_mut(field.id)?.set("Kids", kids);
    document.objects.remove(&id);
    Ok(())
}

fn rotate_appearance(
    document: &mut Document,
    value: &Object,
    delta: i32,
    rect: [f32; 4],
) -> Result<Object> {
    let mut stream = resolve(document, value)?.as_stream()?.clone();
    super::flatten::placement_matrix(document, &stream, rect)?;
    let [a, b, c, d, e, f] = super::flatten::matrix(document, &stream)?;
    let rotated = match delta.rem_euclid(360) {
        0 => [a, b, c, d, e, f],
        90 => [-b, a, -d, c, -f, e],
        180 => [-a, -b, -c, -d, -e, -f],
        270 => [b, -a, d, -c, f, -e],
        _ => {
            return Err(invalid(
                "Widget appearance rotation must be a multiple of 90 degrees.",
            ));
        }
    };
    stream.dict.set("Matrix", numbers(&rotated));
    Ok(document.add_object(stream).into())
}

fn rotate_widget_appearances(
    document: &mut Document,
    widget: ObjectId,
    delta: i32,
    rect: [f32; 4],
) -> Result<()> {
    if delta.rem_euclid(360) == 0 {
        return Ok(());
    }
    let dictionary = document.get_dictionary(widget)?.clone();
    let Some(appearances) = optional(document, &dictionary, b"AP")? else {
        return Ok(());
    };
    let mut appearances = appearances.as_dict()?.clone();
    for key in [b"N".as_slice(), b"R", b"D"] {
        let Some(value) = optional(document, &appearances, key)?.cloned() else {
            continue;
        };
        let value = if let Object::Dictionary(states) = value {
            if states.len() > 16 {
                return Err(invalid(
                    "A widget has too many rollover/down appearance states to rotate safely.",
                ));
            }
            let mut rotated = Dictionary::new();
            for (name, value) in states {
                rotated.set(name, rotate_appearance(document, &value, delta, rect)?);
            }
            Object::Dictionary(rotated)
        } else {
            rotate_appearance(document, &value, delta, rect)?
        };
        appearances.set(key, value);
    }
    document.get_dictionary_mut(widget)?.set("AP", appearances);
    Ok(())
}

fn place_widget(
    document: &mut Document,
    field: &Field,
    widget_id: &str,
    placement: WidgetPlacement,
) -> Result<()> {
    if placement.option.is_some() {
        return Err(invalid(
            "Layout changes cannot change a radio export state.",
        ));
    }
    let widget = field
        .info
        .widgets
        .iter()
        .find(|widget| widget.id == widget_id)
        .ok_or_else(|| invalid("This widget does not belong to the selected logical field."))?;
    let id = crate::pdf_objects::parse_key(widget_id)?;
    let page = page_id(document, placement.page)?;
    let geometry = PageGeometry::for_page(document, page)?;
    let previous = PageGeometry::for_page(document, page_id(document, widget.page)?)?;
    let rect = geometry.display_rect(placement.placement)?;
    if matches!(
        field.info.kind,
        FieldKind::Checkbox | FieldKind::Radio | FieldKind::Signature
    ) {
        rotate_widget_appearances(document, id, geometry.rotation - previous.rotation, rect)?;
    }
    let dictionary = document.get_dictionary(id)?.clone();
    let mut mk = optional(document, &dictionary, b"MK")?
        .map(Object::as_dict)
        .transpose()?
        .cloned()
        .unwrap_or_default();
    let rotation = integer(document, &mk, b"R", 0)?;
    if rotation % 90 != 0 {
        return Err(invalid(
            "The widget rotation must be a multiple of 90 degrees.",
        ));
    }
    mk.set(
        "R",
        if field.info.kind == FieldKind::Signature {
            0
        } else {
            (rotation + i64::from(geometry.rotation - previous.rotation)).rem_euclid(360)
        },
    );
    let changed = document.get_dictionary_mut(id)?;
    changed.set("Rect", numbers(&rect));
    changed.set("P", page);
    changed.set("MK", mk);
    widget_rect(document, document.get_dictionary(id)?, page)?;
    if widget.page != placement.page {
        remove_annotation(document, widget.page, id)?;
        let mut annotations = annotation_entries(document, page)?;
        annotations.push(id.into());
        document
            .get_dictionary_mut(page)?
            .set("Annots", annotations);
    }
    refresh(document, field.id, field.info.value.clone())
}

pub(crate) fn apply(document: &mut Document, changes: Vec<Change>) -> Result<()> {
    if changes.is_empty() || changes.len() > MAX_DESIGN_CHANGES {
        return Err(invalid(
            "Apply between 1 and 64 form-design changes in one transaction.",
        ));
    }
    ensure_form(document)?;
    for change in changes {
        let form = read(document)?;
        match change {
            Change::Create {
                definition,
                widgets,
            } => create(document, definition, widgets)?,
            Change::Update { id, definition } => update(document, &form, id, definition)?,
            Change::PlaceWidget {
                id,
                widget_id,
                widget,
            } => {
                let field = field(&form, &id)?;
                supported(document, field)?;
                place_widget(document, field, &widget_id, widget)?;
            }
            Change::RemoveWidget { id, widget_id } => {
                let field = field(&form, &id)?;
                supported(document, field)?;
                remove_widget(document, field, &widget_id)?;
            }
            Change::RemoveField { id } => {
                let field = field(&form, &id)?;
                supported(document, field)?;
                remove_field(document, field)?;
            }
        }
    }
    check_names(&read(document)?)?;
    Ok(())
}
