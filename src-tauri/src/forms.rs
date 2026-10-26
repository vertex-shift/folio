use std::collections::{BTreeMap, HashMap, HashSet};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    decode_text_string, dictionary, text_string,
};
use serde::{Deserialize, Serialize};

use crate::document::{Error, Result, invalid, numbers, page_bounds};

pub mod designer;
pub mod flatten;

const MAX_NODES: usize = 4096;
const MAX_FIELDS: usize = 1024;
const MAX_OPTIONS: usize = 4096;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_APPEARANCE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FIELD_CHARACTERS: usize = 4000;
const FONT: &[u8] = include_bytes!("../resources/fonts/NotoSans-Regular.ttf");

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FieldValue {
    Text {
        text: String,
    },
    Button {
        state: Option<String>,
    },
    Choice {
        indices: Vec<usize>,
        custom: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FieldChange {
    pub id: String,
    pub value: FieldValue,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Checkbox,
    Radio,
    Choice,
    Signature,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormOption {
    pub id: String,
    pub label: String,
    pub export_value: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormWidget {
    pub id: String,
    pub page: usize,
    pub rect: [f32; 4],
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub id: String,
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    pub read_only: bool,
    pub reason: Option<String>,
    pub required: bool,
    pub multiline: bool,
    pub password: bool,
    pub comb: bool,
    pub max_length: usize,
    pub multiple: bool,
    pub combo: bool,
    pub editable_choice: bool,
    pub no_toggle_to_off: bool,
    pub options: Vec<FormOption>,
    pub value: Option<FieldValue>,
    pub widgets: Vec<FormWidget>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfo {
    pub fields: Vec<FormField>,
    pub fillable: bool,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone)]
struct Field {
    id: ObjectId,
    info: FormField,
    attributes: Dictionary,
    widgets: Vec<ObjectId>,
    exports: Vec<Object>,
}

struct Form {
    fields: Vec<Field>,
    reason: Option<String>,
    need_appearances: bool,
}

fn resolve<'a>(document: &'a Document, mut value: &'a Object) -> Result<&'a Object> {
    for _ in 0..32 {
        if let Object::Reference(id) = value {
            value = document.get_object(*id)?;
        } else {
            return Ok(value);
        }
    }
    Err(invalid(
        "The form contains cyclic or excessively deep object references.",
    ))
}

fn optional<'a>(
    document: &'a Document,
    dictionary: &'a Dictionary,
    key: &[u8],
) -> Result<Option<&'a Object>> {
    dictionary
        .get(key)
        .ok()
        .map(|value| resolve(document, value))
        .transpose()
        .map(|value| value.filter(|value| !matches!(value, Object::Null)))
}

fn integer(document: &Document, dict: &Dictionary, key: &[u8], default: i64) -> Result<i64> {
    optional(document, dict, key)?
        .map(Object::as_i64)
        .transpose()
        .map(|value| value.unwrap_or(default))
        .map_err(|_| invalid("A form field has an invalid numeric attribute."))
}

fn text(document: &Document, value: &Object, budget: &mut usize) -> Result<String> {
    let value = resolve(document, value)?;
    let bytes = value
        .as_str()
        .map_err(|_| invalid("A form field has an invalid text string."))?;
    *budget = budget
        .checked_add(bytes.len())
        .ok_or_else(|| invalid("The form exceeds the text resource limit."))?;
    if bytes.len() > MAX_FIELD_CHARACTERS * 4 + 2 || *budget > MAX_TEXT_BYTES {
        return Err(invalid(
            "The form exceeds the 4,000-character field or 1 MiB text limit.",
        ));
    }
    decode_text_string(value)
        .map_err(|_| invalid("A form field contains an unsupported text encoding."))
}

fn normalized(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn key(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}

fn state_key(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn state_label(bytes: &[u8], budget: &mut usize) -> Result<String> {
    text(
        &Document::new(),
        &Object::String(bytes.to_vec(), StringFormat::Literal),
        budget,
    )
}

struct Reader<'a> {
    document: &'a Document,
    page_ids: Vec<ObjectId>,
    pages: HashMap<ObjectId, Vec<usize>>,
    visited: HashSet<ObjectId>,
    fields: Vec<Field>,
    text_bytes: usize,
    option_count: usize,
}

impl Reader<'_> {
    fn walk(
        &mut self,
        id: ObjectId,
        parent: Option<ObjectId>,
        inherited: &Dictionary,
        prefix: &str,
        depth: usize,
        parent_actions: bool,
    ) -> Result<()> {
        if depth > 32 || !self.visited.insert(id) || self.visited.len() > MAX_NODES {
            return Err(invalid(
                "The form field tree is cyclic, shared, or exceeds 4,096 nodes / 32 levels.",
            ));
        }
        let dict = self.document.get_dictionary(id)?.clone();
        if let Ok(value) = dict.get(b"Parent")
            && Some(value.as_reference()?) != parent
        {
            return Err(invalid(
                "A form field's parent does not match its field tree.",
            ));
        }
        let mut attributes = inherited.clone();
        for name in [
            b"FT".as_slice(),
            b"Ff",
            b"V",
            b"DV",
            b"DA",
            b"Q",
            b"Opt",
            b"MaxLen",
            b"TI",
        ] {
            if let Ok(value) = dict.get(name)
                && !matches!(resolve(self.document, value)?, Object::Null)
            {
                attributes.set(name, value.clone());
            }
        }
        attributes.remove(b"I");
        if let Ok(indices) = dict.get(b"I") {
            attributes.set("I", indices.clone());
        }
        let partial = dict
            .get(b"T")
            .ok()
            .map(|value| text(self.document, value, &mut self.text_bytes))
            .transpose()?
            .unwrap_or_default();
        let name = match (prefix.is_empty(), partial.is_empty()) {
            (_, true) => prefix.to_owned(),
            (true, false) => partial,
            (false, false) => format!("{prefix}.{partial}"),
        };
        if name.len() > 16_384 {
            return Err(invalid(
                "A qualified form field name exceeds the 16 KiB limit.",
            ));
        }
        let actions = parent_actions || dict.has(b"A") || dict.has(b"AA");
        let children = optional(self.document, &dict, b"Kids")?
            .map(Object::as_array)
            .transpose()?
            .cloned()
            .unwrap_or_default();
        if children.len() > MAX_NODES {
            return Err(invalid("A form field has too many children."));
        }
        let children = children
            .iter()
            .map(|value| value.as_reference().map_err(|_| invalid("Direct form fields/widgets are not supported; use indirect PDF field objects.")))
            .collect::<Result<Vec<_>>>()?;
        let widget_children = children
            .iter()
            .map(|id| {
                let child = self.document.get_dictionary(*id)?;
                Ok(
                    child.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
                        && !child.has(b"T")
                        && !child.has(b"Kids"),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        if widget_children.iter().any(|value| *value) && widget_children.iter().any(|value| !*value)
        {
            return Err(invalid(
                "The form mixes child fields and widget annotations in one field.",
            ));
        }
        if !children.is_empty() && widget_children.iter().all(|value| *value) {
            for child in &children {
                if !self.visited.insert(*child) || self.visited.len() > MAX_NODES {
                    return Err(invalid(
                        "A form widget is cyclic, shared, or exceeds the node limit.",
                    ));
                }
                if self
                    .document
                    .get_dictionary(*child)?
                    .get(b"Parent")?
                    .as_reference()?
                    != id
                {
                    return Err(invalid("A widget's parent does not match its form field."));
                }
            }
            self.add_field(id, &dict, attributes, name, children, actions)?;
        } else if !children.is_empty() {
            for child in children {
                self.walk(child, Some(id), &attributes, &name, depth + 1, actions)?;
            }
        } else {
            let widgets = if dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
            {
                vec![id]
            } else {
                vec![]
            };
            self.add_field(id, &dict, attributes, name, widgets, actions)?;
        }
        Ok(())
    }

    fn add_field(
        &mut self,
        id: ObjectId,
        dict: &Dictionary,
        attributes: Dictionary,
        name: String,
        widgets: Vec<ObjectId>,
        actions: bool,
    ) -> Result<()> {
        if self.fields.len() >= MAX_FIELDS {
            return Err(invalid("This form exceeds the 1,024-field limit."));
        }
        let flags = u32::try_from(integer(self.document, &attributes, b"Ff", 0)?)
            .map_err(|_| invalid("A form field has invalid flags."))?;
        let kind = match optional(self.document, &attributes, b"FT")?
            .map(Object::as_name)
            .transpose()?
        {
            Some(b"Tx") => FieldKind::Text,
            Some(b"Ch") => FieldKind::Choice,
            Some(b"Sig") => FieldKind::Signature,
            Some(b"Btn") if flags & (1 << 16) != 0 => FieldKind::Unsupported,
            Some(b"Btn") if flags & (1 << 15) != 0 => FieldKind::Radio,
            Some(b"Btn") => FieldKind::Checkbox,
            _ => FieldKind::Unsupported,
        };
        let label = dict
            .get(b"TU")
            .ok()
            .map(|value| text(self.document, value, &mut self.text_bytes))
            .transpose()?
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| {
                if name.is_empty() {
                    format!("Unnamed field {}", key(id))
                } else {
                    name.clone()
                }
            });
        let max_length = integer(
            self.document,
            &attributes,
            b"MaxLen",
            MAX_FIELD_CHARACTERS as i64,
        )?;
        if max_length < 1 {
            return Err(invalid("A form field has an invalid maximum length."));
        }
        let mut info = FormField {
            id: key(id),
            name,
            label,
            kind: kind.clone(),
            read_only: flags & 1 != 0,
            reason: (flags & 1 != 0).then(|| "This field is read-only in the PDF.".into()),
            required: flags & 2 != 0,
            multiline: kind == FieldKind::Text && flags & (1 << 12) != 0,
            password: kind == FieldKind::Text && flags & (1 << 13) != 0,
            comb: kind == FieldKind::Text && flags & (1 << 24) != 0,
            max_length: usize::try_from(max_length)
                .unwrap_or(MAX_FIELD_CHARACTERS)
                .min(MAX_FIELD_CHARACTERS),
            multiple: kind == FieldKind::Choice && flags & (1 << 21) != 0,
            combo: kind == FieldKind::Choice && flags & (1 << 17) != 0,
            editable_choice: kind == FieldKind::Choice
                && flags & (1 << 17) != 0
                && flags & (1 << 18) != 0,
            no_toggle_to_off: kind == FieldKind::Radio && flags & (1 << 14) != 0,
            options: vec![],
            value: None,
            widgets: vec![],
        };
        if widgets.is_empty() {
            disable(&mut info, "This field has no visible page widget.");
        }
        for widget in &widgets {
            let annotation = self.document.get_dictionary(*widget)?;
            if annotation.has(b"A") || annotation.has(b"AA") || actions {
                disable(
                    &mut info,
                    "Fields with PDF actions or scripts are not supported. No actions are executed.",
                );
            }
            let widget_flags = integer(self.document, annotation, b"F", 0)?;
            if widget_flags & (1 | 2 | 32 | 64 | 128 | 512) != 0 {
                disable(
                    &mut info,
                    "A widget is hidden, read-only, or locked in the PDF.",
                );
            }
            match self.pages.get(widget) {
                Some(pages) if pages.len() == 1 => {
                    let page = pages[0];
                    let page_id = self.page_ids[page];
                    if let Ok(p) = annotation.get(b"P")
                        && p.as_reference()? != page_id
                    {
                        disable(&mut info, "A widget's page reference is inconsistent.");
                    }
                    match widget_rect(self.document, annotation, page_id) {
                        Ok(rect) => info.widgets.push(FormWidget {
                            id: key(*widget),
                            page,
                            rect,
                        }),
                        Err(_) => disable(
                            &mut info,
                            "A widget has unsupported or invalid page geometry.",
                        ),
                    }
                }
                _ => disable(
                    &mut info,
                    "A widget is missing from its page or is shared between pages.",
                ),
            }
        }
        if actions {
            disable(
                &mut info,
                "Fields with PDF actions or scripts are not supported. No actions are executed.",
            );
        }
        let mut exports = vec![];
        match kind {
            FieldKind::Text => {
                let value = optional(self.document, &attributes, b"V")?
                    .map(|value| text(self.document, value, &mut self.text_bytes))
                    .transpose()?
                    .unwrap_or_default();
                info.value = Some(FieldValue::Text {
                    text: normalized(&value),
                });
                if flags & ((1 << 20) | (1 << 25)) != 0 || dict.has(b"RV") {
                    disable(
                        &mut info,
                        "Rich-text and file-selection text fields are not supported.",
                    );
                }
                if info.password && info.multiline {
                    disable(&mut info, "Multiline password fields are not supported.");
                }
                if info.comb
                    && (info.multiline
                        || info.password
                        || !attributes.has(b"MaxLen")
                        || max_length > 256)
                {
                    disable(
                        &mut info,
                        "Comb fields need a maximum length up to 256 and cannot be multiline/password fields.",
                    );
                }
            }
            FieldKind::Choice => {
                if info.combo && info.multiple || !info.combo && flags & (1 << 18) != 0 {
                    disable(
                        &mut info,
                        "This choice field has an unsupported combination of flags.",
                    );
                }
                if let Some(options) = optional(self.document, &attributes, b"Opt")? {
                    for (index, option) in options.as_array()?.iter().enumerate() {
                        self.option_count += 1;
                        if self.option_count > MAX_OPTIONS {
                            return Err(invalid("This form exceeds the 4,096-option limit."));
                        }
                        let option = resolve(self.document, option)?;
                        let (export, display) = match option {
                            Object::Array(pair) if pair.len() == 2 => (&pair[0], &pair[1]),
                            Object::String(..) => (option, option),
                            _ => {
                                return Err(invalid(
                                    "A form choice has invalid export/display values.",
                                ));
                            }
                        };
                        let label = text(self.document, display, &mut self.text_bytes)?;
                        if label.chars().any(char::is_control) {
                            disable(
                                &mut info,
                                "Choice labels containing line breaks or control characters are not supported.",
                            );
                        }
                        info.options.push(FormOption {
                            id: index.to_string(),
                            export_value: text(self.document, export, &mut self.text_bytes)?,
                            label,
                        });
                        exports.push(resolve(self.document, export)?.clone());
                    }
                }
                if info.options.is_empty() && !info.editable_choice {
                    disable(&mut info, "This field has no selectable options.");
                }
                info.value = Some(self.choice_value(&attributes, &info)?);
            }
            FieldKind::Checkbox | FieldKind::Radio => {
                let mut states = BTreeMap::<Vec<u8>, (String, String)>::new();
                let button_exports = optional(self.document, &attributes, b"Opt")?
                    .map(Object::as_array)
                    .transpose()?;
                for (index, widget) in widgets.iter().enumerate() {
                    match button_state(self.document, *widget) {
                        Ok(on) => {
                            let export = button_exports
                                .and_then(|values| values.get(index))
                                .map(|value| text(self.document, value, &mut self.text_bytes))
                                .transpose()?
                                .unwrap_or(state_label(&on, &mut self.text_bytes)?);
                            let label = self
                                .document
                                .get_dictionary(*widget)?
                                .get(b"Contents")
                                .ok()
                                .map(|value| text(self.document, value, &mut self.text_bytes))
                                .transpose()?
                                .filter(|label| !label.trim().is_empty())
                                .unwrap_or_else(|| export.clone());
                            if let Some(previous) =
                                states.insert(on, (label.clone(), export.clone()))
                                && kind == FieldKind::Radio
                                && (flags & (1 << 25) == 0 || previous != (label, export))
                            {
                                disable(
                                    &mut info,
                                    "Duplicate radio on-states need RadiosInUnison and identical export values; ambiguous groups are read-only.",
                                );
                            }
                        }
                        Err(_) => disable(
                            &mut info,
                            "Each button needs portable Off/on appearance streams with one unambiguous on-state.",
                        ),
                    }
                }
                if kind == FieldKind::Checkbox && states.len() != 1 {
                    disable(
                        &mut info,
                        "Repeated checkbox widgets must share the same export/on-state.",
                    );
                }
                for (state, (label, export)) in &states {
                    info.options.push(FormOption {
                        id: state_key(state),
                        label: label.clone(),
                        export_value: export.clone(),
                    });
                }
                let value = optional(self.document, &attributes, b"V")?
                    .map(Object::as_name)
                    .transpose()?
                    .unwrap_or(b"Off");
                if value != b"Off" && !states.contains_key(value) {
                    disable(
                        &mut info,
                        "The button value does not match its appearance states.",
                    );
                }
                info.value = Some(FieldValue::Button {
                    state: (value != b"Off").then(|| state_key(value)),
                });
            }
            FieldKind::Signature => disable(
                &mut info,
                "Signature field. Use the digital-signature workflow, not form filling.",
            ),
            FieldKind::Unsupported => disable(
                &mut info,
                "Push buttons and this field type are not fillable. PDF actions are not executed.",
            ),
        }
        if matches!(info.kind, FieldKind::Text | FieldKind::Choice) {
            for widget in &widgets {
                if let Err(error) = style(self.document, *widget, &attributes) {
                    disable(&mut info, &error.to_string());
                }
            }
        }
        self.fields.push(Field {
            id,
            info,
            attributes,
            widgets,
            exports,
        });
        Ok(())
    }

    fn choice_value(&mut self, attributes: &Dictionary, info: &FormField) -> Result<FieldValue> {
        let mut values = vec![];
        if let Some(value) = optional(self.document, attributes, b"V")? {
            match value {
                Object::Array(items) => {
                    if items.len() > MAX_OPTIONS {
                        return Err(invalid("A choice field has too many selected values."));
                    }
                    for item in items {
                        values.push(text(self.document, item, &mut self.text_bytes)?);
                    }
                }
                _ => values.push(text(self.document, value, &mut self.text_bytes)?),
            }
        }
        if let Some(indices) = optional(self.document, attributes, b"I")? {
            if indices.as_array()?.len() > MAX_OPTIONS {
                return Err(invalid("A choice field has too many selected indices."));
            }
            let indices = indices
                .as_array()?
                .iter()
                .map(|value| {
                    usize::try_from(resolve(self.document, value)?.as_i64()?)
                        .map_err(|_| invalid("A choice field has an invalid selected index."))
                })
                .collect::<Result<Vec<_>>>()?;
            if indices.len() > MAX_OPTIONS
                || indices.windows(2).any(|pair| pair[0] >= pair[1])
                || indices.iter().any(|index| *index >= info.options.len())
                || !info.multiple && indices.len() > 1
            {
                return Err(invalid(
                    "A choice field's selected indices are out of range or duplicated.",
                ));
            }
            let selected = indices
                .iter()
                .map(|index| info.options[*index].export_value.clone())
                .collect::<Vec<_>>();
            if selected != values && !(indices.is_empty() && values.iter().all(String::is_empty)) {
                return Err(invalid(
                    "A choice field's export values disagree with its selected indices.",
                ));
            }
            return Ok(FieldValue::Choice {
                indices,
                custom: None,
            });
        }
        let mut indices = vec![];
        for value in values {
            let matches = info
                .options
                .iter()
                .enumerate()
                .filter(|(_, option)| option.export_value == value)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                return Err(invalid(
                    "Duplicate choice export values require explicit selected indices.",
                ));
            }
            if let Some(index) = matches.first() {
                indices.push(*index);
            } else if info.editable_choice {
                return Ok(FieldValue::Choice {
                    indices: vec![],
                    custom: Some(value),
                });
            } else if !value.is_empty() {
                return Err(invalid("A choice value is not present in its option list."));
            }
        }
        indices.sort_unstable();
        indices.dedup();
        if !info.multiple && indices.len() > 1 {
            return Err(invalid(
                "A single-choice field contains multiple selections.",
            ));
        }
        Ok(FieldValue::Choice {
            indices,
            custom: None,
        })
    }
}

fn disable(info: &mut FormField, reason: &str) {
    info.read_only = true;
    if info.reason.is_none() {
        info.reason = Some(reason.to_owned());
    }
}

fn read(document: &Document) -> Result<Form> {
    let catalog = document.catalog()?;
    let Some(acroform) = optional(document, catalog, b"AcroForm")? else {
        return Ok(Form {
            fields: vec![],
            reason: None,
            need_appearances: false,
        });
    };
    let acroform = acroform.as_dict()?;
    let mut reason = if acroform.has(b"XFA") {
        Some(
            "XFA forms are not supported. Use a static AcroForm copy; Folio never executes XFA."
                .into(),
        )
    } else if optional(document, acroform, b"CO")?
        .is_some_and(|value| value.as_array().map_or(true, |items| !items.is_empty()))
    {
        Some("This form has calculation dependencies. Calculations are not executed, so filling is disabled.".into())
    } else if catalog.has(b"Perms") || integer(document, acroform, b"SigFlags", 0)? & 2 != 0 {
        Some("Certified, usage-rights, or append-only signed forms are read-only. Filling requires an incremental signature-aware writer.".into())
    } else if catalog.has(b"AA")
        || optional(document, catalog, b"OpenAction")?.is_some_and(|value| value.as_dict().is_ok())
        || optional(document, catalog, b"Names")?
            .map(Object::as_dict)
            .transpose()?
            .is_some_and(|names| names.has(b"JavaScript"))
    {
        Some("This PDF has document-level actions or JavaScript. Filling is disabled; scripts are never executed.".into())
    } else {
        None
    };
    let pages = document.get_pages();
    if pages.len() > 10_000 {
        return Err(invalid("Form inspection is limited to 10,000 pages."));
    }
    let mut reader = Reader {
        document,
        page_ids: pages.values().copied().collect(),
        pages: HashMap::new(),
        visited: HashSet::new(),
        fields: vec![],
        text_bytes: 0,
        option_count: 0,
    };
    let mut annotation_count = 0;
    for (index, page) in pages.values().enumerate() {
        if document.get_dictionary(*page)?.has(b"AA") && reason.is_none() {
            reason = Some(
                "This PDF has page-level actions. Filling is disabled; actions are never executed."
                    .into(),
            );
        }
        if let Some(annotations) = optional(document, document.get_dictionary(*page)?, b"Annots")? {
            let annotations = annotations.as_array()?;
            annotation_count += annotations.len();
            if annotation_count > 65_536 {
                return Err(invalid(
                    "Form inspection exceeds the 65,536-annotation limit.",
                ));
            }
            for annotation in annotations {
                if let Object::Reference(id) = annotation {
                    reader.pages.entry(*id).or_default().push(index);
                }
            }
        }
    }
    let mut defaults = Dictionary::new();
    if let Ok(da) = acroform.get(b"DA") {
        defaults.set("DA", da.clone());
    }
    if let Ok(q) = acroform.get(b"Q") {
        defaults.set("Q", q.clone());
    }
    if let Some(fields) = optional(document, acroform, b"Fields")? {
        let fields = fields.as_array()?;
        if fields.len() > MAX_NODES {
            return Err(invalid("This form has too many root fields."));
        }
        for field in fields {
            reader.walk(field.as_reference()?, None, &defaults, "", 0, false)?;
        }
    }
    let mut names = HashMap::new();
    for field in &reader.fields {
        *names.entry(field.info.name.clone()).or_insert(0) += 1;
    }
    for field in &mut reader.fields {
        if names[&field.info.name] > 1 {
            disable(
                &mut field.info,
                "Distinct fields share this qualified name. Ambiguous duplicate field names are read-only.",
            );
        }
    }
    let signed = reader.fields.iter().any(|field| {
        field.info.kind == FieldKind::Signature
            && optional(document, &field.attributes, b"V").map_or(true, |value| value.is_some())
    });
    Ok(Form {
        fields: reader.fields,
        reason: if signed {
            Some(
                "This PDF contains an existing signature. Ordinary form filling is read-only."
                    .into(),
            )
        } else {
            reason
        },
        need_appearances: optional(document, acroform, b"NeedAppearances")?
            .map(Object::as_bool)
            .transpose()?
            .unwrap_or(false),
    })
}

pub(crate) fn inspect(
    document: &Document,
    allowed: bool,
    permission_reason: Option<&str>,
) -> Result<FormInfo> {
    let form = read(document)?;
    let mut warnings = vec![
        "Apply writes values and portable widget appearances together. Text appearances use embedded Noto Sans (Latin, Greek, and Cyrillic), not the original field font.".into(),
        "PDF scripts, actions, calculations, and XFA are never executed. Password fields are masked, not file encryption.".into(),
    ];
    if form.need_appearances {
        warnings.push("This PDF requests viewer-generated appearances. Changed fields get embedded appearances; untouched fields retain their original behavior.".into());
    }
    let reason = form.reason.or_else(|| {
        (!allowed).then(|| {
            permission_reason
                .unwrap_or("This PDF does not permit form filling.")
                .to_owned()
        })
    });
    Ok(FormInfo {
        fillable: reason.is_none(),
        reason,
        fields: form.fields.into_iter().map(|field| field.info).collect(),
        warnings,
    })
}

fn widget_rect(document: &Document, widget: &Dictionary, page: ObjectId) -> Result<[f32; 4]> {
    let values = resolve(document, widget.get(b"Rect")?)?.as_array()?;
    if values.len() != 4 {
        return Err(invalid("A form widget has an invalid rectangle."));
    }
    let values = values
        .iter()
        .map(|value| Ok(resolve(document, value)?.as_float()?))
        .collect::<Result<Vec<_>>>()?;
    let rect = [
        values[0].min(values[2]),
        values[1].min(values[3]),
        values[0].max(values[2]),
        values[1].max(values[3]),
    ];
    let bounds = page_bounds(document, page)?;
    if values.iter().any(|value| !value.is_finite())
        || rect[2] - rect[0] < 4.0
        || rect[3] - rect[1] < 4.0
        || rect[2] - rect[0] > 14_400.0
        || rect[3] - rect[1] > 14_400.0
        || rect[0] < bounds[0] - 1.0
        || rect[1] < bounds[1] - 1.0
        || rect[2] > bounds[2] + 1.0
        || rect[3] > bounds[3] + 1.0
    {
        return Err(invalid(
            "The form widget is outside the page or has unsupported dimensions.",
        ));
    }
    Ok(rect)
}

fn button_state(document: &Document, widget: ObjectId) -> Result<Vec<u8>> {
    let dict = document.get_dictionary(widget)?;
    let ap = resolve(document, dict.get(b"AP")?)?.as_dict()?;
    let normal = resolve(document, ap.get(b"N")?)?.as_dict()?;
    if normal.len() != 2 || !normal.has(b"Off") {
        return Err(invalid("The button needs Off and one on-state appearance."));
    }
    let mut on = None;
    for (name, appearance) in normal {
        if name.len() > 256 {
            return Err(invalid(
                "A button appearance state exceeds the 256-byte limit.",
            ));
        }
        resolve(document, appearance)?.as_stream()?;
        if name != b"Off" {
            on = Some(name.clone());
        }
    }
    on.ok_or_else(|| invalid("The button has no on-state appearance."))
}

struct Style {
    width: f32,
    height: f32,
    matrix: [f32; 6],
    size: f32,
    color: [f32; 3],
    background: Option<[f32; 3]>,
    border: Option<[f32; 3]>,
    border_width: f32,
    underline: bool,
    dash: Vec<f32>,
    align: i64,
}

fn color(document: &Document, value: &Object) -> Result<Option<[f32; 3]>> {
    let values = resolve(document, value)?
        .as_array()?
        .iter()
        .map(|value| Ok(resolve(document, value)?.as_float()?))
        .collect::<Result<Vec<_>>>()?;
    if values
        .iter()
        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
    {
        return Err(invalid("A form widget uses an invalid color."));
    }
    Ok(match values.as_slice() {
        [] => None,
        [gray] => Some([*gray; 3]),
        [r, g, b] => Some([*r, *g, *b]),
        [c, m, y, k] => Some([
            1.0 - (c + k).min(1.0),
            1.0 - (m + k).min(1.0),
            1.0 - (y + k).min(1.0),
        ]),
        _ => return Err(invalid("A form widget uses an unsupported color space.")),
    })
}

fn style(document: &Document, widget: ObjectId, attributes: &Dictionary) -> Result<Style> {
    let dict = document.get_dictionary(widget)?;
    let rect = resolve(document, dict.get(b"Rect")?)?.as_array()?;
    if rect.len() != 4 {
        return Err(invalid("A form widget has an invalid rectangle."));
    }
    let width = (rect[2].as_float()? - rect[0].as_float()?).abs();
    let height = (rect[3].as_float()? - rect[1].as_float()?).abs();
    let mk = optional(document, dict, b"MK")?
        .map(Object::as_dict)
        .transpose()?;
    let rotation = mk
        .map(|mk| integer(document, mk, b"R", 0))
        .transpose()?
        .unwrap_or(0);
    let (width, height, matrix) = match rotation.rem_euclid(360) {
        0 => (width, height, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
        90 => (height, width, [0.0, 1.0, -1.0, 0.0, width, 0.0]),
        180 => (width, height, [-1.0, 0.0, 0.0, -1.0, width, height]),
        270 => (height, width, [0.0, -1.0, 1.0, 0.0, 0.0, height]),
        _ => {
            return Err(invalid(
                "The widget rotation must be a multiple of 90 degrees.",
            ));
        }
    };
    let mut style = Style {
        width,
        height,
        matrix,
        size: 0.0,
        color: [0.0; 3],
        background: None,
        border: None,
        border_width: 0.0,
        underline: false,
        dash: vec![],
        align: integer(document, attributes, b"Q", 0)?,
    };
    if !(0..=2).contains(&style.align) {
        return Err(invalid("This field uses unsupported text alignment."));
    }
    if let Some(da) = optional(document, attributes, b"DA")? {
        let bytes = da.as_str()?;
        if bytes.len() > 4096 {
            return Err(invalid("The field's default appearance exceeds 4 KiB."));
        }
        let content = Content::decode(bytes)
            .map_err(|_| invalid("The field's default appearance is invalid."))?;
        for operation in content.operations {
            let operands = &operation.operands;
            match (operation.operator.as_str(), operands.as_slice()) {
                ("Tf", [_, size]) => style.size = size.as_float()?,
                ("g", [_]) | ("rg", [_, _, _]) | ("k", [_, _, _, _]) => {
                    style.color = color(document, &Object::Array(operands.clone()))?
                        .ok_or_else(|| invalid("The field text color is invalid."))?;
                }
                _ => {}
            }
        }
    }
    if !style.size.is_finite() || !(0.0..=256.0).contains(&style.size) {
        return Err(invalid("This field's default font size is unsupported."));
    }
    if let Some(mk) = mk {
        style.background = optional(document, mk, b"BG")?
            .map(|value| color(document, value))
            .transpose()?
            .flatten();
        style.border = optional(document, mk, b"BC")?
            .map(|value| color(document, value))
            .transpose()?
            .flatten();
    }
    if let Some(bs) = optional(document, dict, b"BS")? {
        let bs = bs.as_dict()?;
        style.border_width = optional(document, bs, b"W")?
            .map(Object::as_float)
            .transpose()?
            .unwrap_or(1.0);
        let border_style = optional(document, bs, b"S")?
            .map(Object::as_name)
            .transpose()?
            .unwrap_or(b"S");
        if !matches!(border_style, b"S" | b"D" | b"U") {
            return Err(invalid(
                "Beveled/inset or custom field borders are not supported.",
            ));
        }
        style.underline = border_style == b"U";
        if border_style == b"D" {
            style.dash = match optional(document, bs, b"D")? {
                Some(dash) => dash
                    .as_array()?
                    .iter()
                    .map(|value| Ok(resolve(document, value)?.as_float()?))
                    .collect::<Result<_>>()?,
                None => vec![3.0],
            };
        }
        if style.border.is_none() {
            style.border = Some([0.0; 3]);
        }
    } else if let Some(border) = optional(document, dict, b"Border")? {
        let border = border.as_array()?;
        if border.len() < 3
            || border.len() > 4
            || border[0].as_float()? != 0.0
            || border[1].as_float()? != 0.0
        {
            return Err(invalid(
                "Rounded or malformed field borders are not supported.",
            ));
        }
        style.border_width = border[2].as_float()?;
        if border.len() == 4 {
            style.dash = border[3]
                .as_array()?
                .iter()
                .map(|value| Ok(value.as_float()?))
                .collect::<Result<_>>()?;
        }
        if style.border.is_none() {
            style.border = Some([0.0; 3]);
        }
    } else if style.border.is_some() {
        style.border_width = 1.0;
    }
    if !style.border_width.is_finite()
        || !(0.0..=12.0).contains(&style.border_width)
        || style.dash.len() > 16
        || style
            .dash
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=128.0).contains(value))
        || !style.dash.is_empty() && style.dash.iter().all(|value| *value == 0.0)
    {
        return Err(invalid(
            "This field's border width or dash pattern is unsupported.",
        ));
    }
    Ok(style)
}

fn validate_text_value(value: &str, field: &FormField, multiline: bool) -> Result<String> {
    if value.len() > MAX_FIELD_CHARACTERS * 4 {
        return Err(invalid("A form value exceeds the 4,000-character limit."));
    }
    let value = normalized(value);
    if value.chars().count() > field.max_length {
        return Err(invalid(
            "The form value exceeds this field's maximum length.",
        ));
    }
    if value
        .chars()
        .any(|character| character.is_control() && !(multiline && character == '\n'))
    {
        return Err(invalid(
            "This field does not support line breaks or control characters.",
        ));
    }
    if field.required && value.trim().is_empty() {
        return Err(invalid("A required form field cannot be cleared."));
    }
    Ok(value)
}

fn validate_value(field: &Field, value: &FieldValue) -> Result<FieldValue> {
    if field.info.read_only {
        return Err(invalid(
            field
                .info
                .reason
                .as_deref()
                .unwrap_or("This form field is read-only."),
        ));
    }
    match (&field.info.kind, value) {
        (FieldKind::Text, FieldValue::Text { text }) => Ok(FieldValue::Text {
            text: validate_text_value(text, &field.info, field.info.multiline)?,
        }),
        (FieldKind::Checkbox | FieldKind::Radio, FieldValue::Button { state }) => {
            if state
                .as_ref()
                .is_some_and(|state| !field.info.options.iter().any(|option| option.id == *state))
            {
                return Err(invalid("Choose an existing button export state."));
            }
            if state.is_none() && (field.info.required || field.info.no_toggle_to_off) {
                return Err(invalid(
                    "This button group requires a selection and cannot be cleared.",
                ));
            }
            Ok(value.clone())
        }
        (FieldKind::Choice, FieldValue::Choice { indices, custom }) => {
            if indices.len() > field.info.options.len()
                || indices
                    .iter()
                    .any(|index| *index >= field.info.options.len())
                || indices.windows(2).any(|pair| pair[0] >= pair[1])
                || !field.info.multiple && indices.len() > 1
            {
                return Err(invalid(
                    "Choose valid, unique choice indices in ascending order.",
                ));
            }
            if let Some(custom) = custom {
                if !field.info.editable_choice || !indices.is_empty() {
                    return Err(invalid(
                        "Only an editable combo box accepts custom text instead of a selection.",
                    ));
                }
                let custom = validate_text_value(custom, &field.info, false)?;
                let mut matching = field
                    .info
                    .options
                    .iter()
                    .enumerate()
                    .filter(|(_, option)| option.export_value == custom);
                let first = matching.next();
                if matching.next().is_some() {
                    return Err(invalid(
                        "Custom text matches duplicate export values. Choose an explicit option instead.",
                    ));
                }
                return Ok(if let Some((index, _)) = first {
                    FieldValue::Choice {
                        indices: vec![index],
                        custom: None,
                    }
                } else {
                    FieldValue::Choice {
                        indices: vec![],
                        custom: Some(custom),
                    }
                });
            }
            if field.info.required
                && (indices.is_empty()
                    || indices
                        .iter()
                        .all(|index| field.info.options[*index].export_value.is_empty()))
            {
                return Err(invalid(
                    "A required choice field needs a nonempty selection.",
                ));
            }
            Ok(value.clone())
        }
        _ => Err(invalid("The value type does not match this form field.")),
    }
}

fn field_ancestors(document: &Document, id: ObjectId) -> Result<Vec<ObjectId>> {
    let mut ancestors = vec![];
    let mut current = id;
    while let Ok(parent) = document.get_dictionary(current)?.get(b"Parent") {
        let parent = parent.as_reference()?;
        if ancestors.len() >= 32 || ancestors.contains(&parent) {
            return Err(invalid(
                "The form has a cyclic or excessively deep parent chain.",
            ));
        }
        ancestors.push(parent);
        current = parent;
    }
    Ok(ancestors)
}

fn isolate_cleared_values(
    document: &mut Document,
    form: &Form,
    selected: &[(&Field, FieldValue)],
) -> Result<()> {
    let mut ancestors = HashSet::new();
    for (field, value) in selected {
        if let FieldValue::Choice {
            indices,
            custom: None,
        } = value
            && !field.info.multiple
            && indices.is_empty()
        {
            for ancestor in field_ancestors(document, field.id)? {
                if optional(document, document.get_dictionary(ancestor)?, b"V")?.is_some() {
                    ancestors.insert(ancestor);
                }
            }
        }
    }
    if ancestors.is_empty() {
        return Ok(());
    }
    // PDF null is an absent entry in other readers. Materialize sibling values
    // before removing inherited values, so a cleared leaf cannot inherit again.
    for field in &form.fields {
        if field_ancestors(document, field.id)?
            .iter()
            .any(|id| ancestors.contains(id))
            && let Ok(value) = field.attributes.get(b"V")
        {
            document
                .get_dictionary_mut(field.id)?
                .set("V", value.clone());
        }
    }
    for ancestor in ancestors {
        document.get_dictionary_mut(ancestor)?.remove(b"V");
    }
    Ok(())
}

fn font_resource_name(document: &Document, font: ObjectId) -> Result<String> {
    let form = resolve(document, document.catalog()?.get(b"AcroForm")?)?.as_dict()?;
    let resources = optional(document, form, b"DR")?
        .map(Object::as_dict)
        .transpose()?;
    let fonts = resources
        .map(|resources| optional(document, resources, b"Font"))
        .transpose()?
        .flatten()
        .map(Object::as_dict)
        .transpose()?;
    for suffix in 0..=MAX_NODES {
        let name = if suffix == 0 {
            format!("FolioForm{}", font.0)
        } else {
            format!("FolioForm{}_{suffix}", font.0)
        };
        if fonts.is_none_or(|fonts| !fonts.has(name.as_bytes())) {
            return Ok(name);
        }
    }
    Err(invalid(
        "The form's font resource names exceed the collision limit.",
    ))
}

fn field_error(field: &Field, error: impl std::fmt::Display) -> Error {
    let metadata = |value: &str| {
        let mut value = value.chars().filter(|character| !character.is_control());
        let mut label = value.by_ref().take(200).collect::<String>();
        if value.next().is_some() {
            label.push_str("...");
        }
        label
    };
    let label = metadata(&field.info.label);
    let name = if field.info.name.is_empty() {
        field.info.id.clone()
    } else {
        metadata(&field.info.name)
    };
    invalid(if label == name {
        format!("Field \"{name}\": {error}")
    } else {
        format!("Field \"{label}\" ({name}): {error}")
    })
}

pub(crate) fn fill(document: &mut Document, changes: Vec<FieldChange>) -> Result<()> {
    write_values(document, changes, true)
}

// Authoring and static-copy generation may preserve empty required/read-only
// values. Their callers validate structure and require general editing access.
fn write_values(document: &mut Document, changes: Vec<FieldChange>, filling: bool) -> Result<()> {
    if changes.is_empty() || changes.len() > MAX_FIELDS {
        return Err(invalid(
            "Apply between 1 and 1,024 form field changes at a time.",
        ));
    }
    let form = read(document)?;
    if let Some(reason) = &form.reason {
        return Err(invalid(reason));
    }
    let mut selected = vec![];
    let mut seen = HashSet::new();
    let mut font_text = String::new();
    let mut widget_count = 0;
    for change in changes {
        if !seen.insert(change.id.clone()) {
            return Err(invalid("The form request contains a duplicate field."));
        }
        let field = form.fields.iter().find(|field| field.info.id == change.id)
            .ok_or_else(|| invalid("This form field is no longer in the document. Reload the form before applying."))?;
        let value = if filling {
            validate_value(field, &change.value)
        } else {
            let mut stored = field.clone();
            stored.info.read_only = false;
            stored.info.required = false;
            stored.info.no_toggle_to_off = false;
            validate_value(&stored, &change.value)
        }
        .map_err(|error| field_error(field, error))?;
        widget_count += field.widgets.len();
        if widget_count > 1024 {
            return Err(invalid("Apply at most 1,024 widget appearances at a time."));
        }
        let mut field_text = String::new();
        match &value {
            FieldValue::Text { text } => {
                if field.info.password {
                    field_text.push_str(&"*".repeat(text.chars().count()));
                } else {
                    field_text.push_str(text);
                }
            }
            FieldValue::Choice { custom, .. } => {
                for option in &field.info.options {
                    field_text.push_str(&option.label);
                }
                if let Some(custom) = custom {
                    field_text.push_str(custom);
                }
            }
            _ => {}
        }
        if matches!(field.info.kind, FieldKind::Text | FieldKind::Choice) {
            font_metrics(&field_text).map_err(|error| field_error(field, error))?;
        }
        font_text.push_str(&field_text);
        if font_text.len() > MAX_TEXT_BYTES {
            return Err(invalid(
                "The changed fields exceed the 1 MiB appearance text limit.",
            ));
        }
        selected.push((field, value));
    }
    isolate_cleared_values(document, &form, &selected)?;
    let font = if selected
        .iter()
        .any(|(field, _)| matches!(field.info.kind, FieldKind::Text | FieldKind::Choice))
    {
        Some(crate::writing::embed_text_font(document, &font_text).map_err(|_| invalid(
            "The form display text cannot be embedded. Use precomposed Latin, Greek, or Cyrillic characters supported by Noto Sans.",
        ))?)
    } else {
        None
    };
    let resource_name = font
        .as_ref()
        .map(|(id, _)| font_resource_name(document, *id))
        .transpose()?;
    let appearance_font = if let (Some((id, encoded)), Some(name)) = (&font, &resource_name) {
        let (face, glyphs) = font_metrics(&font_text)?;
        Some(AppearanceFont {
            id: *id,
            name,
            encoded,
            face,
            glyphs,
        })
    } else {
        None
    };
    let mut appearance_bytes = 0;
    for (field, value) in selected {
        match &value {
            FieldValue::Text { text } => {
                document
                    .get_dictionary_mut(field.id)?
                    .set("V", text_string(&text.replace('\n', "\r")));
            }
            FieldValue::Button { state } => {
                let selected = state
                    .as_ref()
                    .and_then(|state| field.info.options.iter().find(|option| option.id == *state));
                let on = if let Some(selected) = selected {
                    let widget = field
                        .widgets
                        .iter()
                        .find(|widget| {
                            button_state(document, **widget)
                                .is_ok_and(|state| state_key(&state) == selected.id)
                        })
                        .ok_or_else(|| invalid("The selected button appearance is unavailable."))?;
                    button_state(document, *widget)?
                } else {
                    b"Off".to_vec()
                };
                document
                    .get_dictionary_mut(field.id)?
                    .set("V", Object::Name(on.clone()));
                for widget in &field.widgets {
                    let widget_on = button_state(document, *widget)?;
                    document.get_dictionary_mut(*widget)?.set(
                        "AS",
                        Object::Name(if widget_on == on {
                            on.clone()
                        } else {
                            b"Off".to_vec()
                        }),
                    );
                }
            }
            FieldValue::Choice { indices, custom } => {
                let values = indices
                    .iter()
                    .map(|index| field.exports[*index].clone())
                    .collect::<Vec<_>>();
                let dict = document.get_dictionary_mut(field.id)?;
                dict.set(
                    "V",
                    if let Some(custom) = custom {
                        text_string(custom)
                    } else if field.info.multiple {
                        Object::Array(values)
                    } else {
                        values.into_iter().next().unwrap_or(Object::Null)
                    },
                );
                if custom.is_some() {
                    dict.remove(b"I");
                } else {
                    dict.set(
                        "I",
                        Object::Array(
                            indices
                                .iter()
                                .map(|index| Object::Integer(*index as i64))
                                .collect(),
                        ),
                    );
                }
                if !field.info.combo {
                    dict.set("TI", indices.first().copied().unwrap_or(0) as i64);
                }
            }
        }
        if let Some(font) = &appearance_font
            && matches!(field.info.kind, FieldKind::Text | FieldKind::Choice)
        {
            for widget in &field.widgets {
                let style = style(document, *widget, &field.attributes)
                    .map_err(|error| field_error(field, error))?;
                let appearance =
                    appearance(document, field, &value, &style, font, &mut appearance_bytes)
                        .map_err(|error| field_error(field, error))?;
                document
                    .get_dictionary_mut(*widget)?
                    .set("AP", dictionary! { "N" => appearance });
            }
            let style = style(document, field.widgets[0], &field.attributes)?;
            document.get_dictionary_mut(field.id)?.set(
                "DA",
                Object::string_literal(format!(
                    "/{} {} Tf {} {} {} rg",
                    font.name, style.size, style.color[0], style.color[1], style.color[2],
                )),
            );
        }
    }
    let mut acroform = resolve(document, document.catalog()?.get(b"AcroForm")?)?
        .as_dict()?
        .clone();
    if let (Some((font, _)), Some(name)) = (&font, &resource_name) {
        let mut resources = optional(document, &acroform, b"DR")?
            .map(Object::as_dict)
            .transpose()?
            .cloned()
            .unwrap_or_default();
        let mut fonts = optional(document, &resources, b"Font")?
            .map(Object::as_dict)
            .transpose()?
            .cloned()
            .unwrap_or_default();
        fonts.set(name.as_bytes(), *font);
        resources.set("Font", fonts);
        acroform.set("DR", resources);
    }
    // Existing appearances may be stale when this global flag is true.
    // Keep untouched fields' viewer behavior rather than suppressing it.
    if !form.need_appearances {
        acroform.set("NeedAppearances", false);
    }
    let mut location = document.catalog()?.get(b"AcroForm")?.clone();
    for _ in 0..32 {
        match location {
            Object::Reference(id) => {
                if let Object::Reference(next) = document.get_object(id)? {
                    location = Object::Reference(*next);
                } else {
                    document.set_object(id, acroform);
                    return Ok(());
                }
            }
            _ => {
                document.catalog_mut()?.set("AcroForm", acroform);
                return Ok(());
            }
        }
    }
    Err(invalid("The AcroForm reference is cyclic."))
}

#[derive(Clone, Copy)]
struct GlyphMetrics {
    advance: f32,
    bounds: Option<[f32; 4]>,
}

struct AppearanceFont<'a> {
    id: ObjectId,
    name: &'a str,
    encoded: &'a BTreeMap<char, u16>,
    face: ttf_parser::Face<'static>,
    glyphs: BTreeMap<char, GlyphMetrics>,
}

#[derive(Clone, Copy, Default)]
struct LineMetrics {
    advance: f32,
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    has_ink: bool,
}

impl LineMetrics {
    fn push(&mut self, glyph: GlyphMetrics, size: f32) {
        if let Some(bounds) = glyph.bounds {
            self.left = self.left.min(self.advance + bounds[0] * size);
            self.right = self.right.max(self.advance + bounds[2] * size);
            if self.has_ink {
                self.bottom = self.bottom.min(bounds[1] * size);
                self.top = self.top.max(bounds[3] * size);
            } else {
                self.bottom = bounds[1] * size;
                self.top = bounds[3] * size;
                self.has_ink = true;
            }
        }
        self.advance += glyph.advance * size;
        self.right = self.right.max(self.advance);
    }

    fn width(&self) -> f32 {
        self.right - self.left
    }
    fn height(&self) -> f32 {
        self.top - self.bottom
    }
}

fn line_metrics(text: &str, glyphs: &BTreeMap<char, GlyphMetrics>, size: f32) -> LineMetrics {
    let mut metrics = LineMetrics::default();
    for character in text.chars() {
        metrics.push(glyphs[&character], size);
    }
    metrics
}

fn font_metrics(text: &str) -> Result<(ttf_parser::Face<'static>, BTreeMap<char, GlyphMetrics>)> {
    let face = ttf_parser::Face::parse(FONT, 0)
        .map_err(|_| invalid("The bundled form font is unavailable."))?;
    let mut glyphs = BTreeMap::new();
    let unit = 1.0 / f32::from(face.units_per_em());
    for character in text.chars().filter(|character| *character != '\n') {
        let glyph = crate::writing::embedded_text_glyph(&face, character).map_err(|_| invalid(
            "The form display text cannot be embedded. Use precomposed Latin, Greek, or Cyrillic characters supported by Noto Sans.",
        ))?;
        glyphs.insert(
            character,
            GlyphMetrics {
                advance: f32::from(face.glyph_hor_advance(glyph).unwrap_or(face.units_per_em()))
                    * unit,
                bounds: face.glyph_bounding_box(glyph).map(|bounds| {
                    [
                        f32::from(bounds.x_min) * unit,
                        f32::from(bounds.y_min) * unit,
                        f32::from(bounds.x_max) * unit,
                        f32::from(bounds.y_max) * unit,
                    ]
                }),
            },
        );
    }
    Ok((face, glyphs))
}

fn appearance(
    document: &mut Document,
    field: &Field,
    value: &FieldValue,
    style: &Style,
    font: &AppearanceFont<'_>,
    appearance_bytes: &mut usize,
) -> Result<ObjectId> {
    let display = match value {
        FieldValue::Text { text } if field.info.password => "*".repeat(text.chars().count()),
        FieldValue::Text { text } => text.clone(),
        FieldValue::Choice {
            custom: Some(text), ..
        } => text.clone(),
        FieldValue::Choice { indices, .. } if field.info.combo => indices
            .first()
            .map(|index| field.info.options[*index].label.clone())
            .unwrap_or_default(),
        FieldValue::Choice { .. } => field
            .info
            .options
            .iter()
            .map(|option| option.label.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => {
            return Err(invalid(
                "Button appearances must use their existing state streams.",
            ));
        }
    };
    let glyphs = &font.glyphs;
    let padding = (style.border_width + 2.0).max(2.0);
    let available_width = style.width - padding * 2.0 - if field.info.combo { 12.0 } else { 0.0 };
    let available_height = style.height - padding * 2.0;
    if available_width < 1.0 || available_height < 1.0 {
        return Err(invalid(
            "This form widget has no usable area inside its border.",
        ));
    }
    let ascent = f32::from(font.face.ascender()) / f32::from(font.face.units_per_em());
    let descent = -f32::from(font.face.descender()) / f32::from(font.face.units_per_em());
    let list = field.info.kind == FieldKind::Choice && !field.info.combo;
    // Global font metrics include scripts absent from this field. Fit a single
    // line's painted glyph bounds without shrinking its fixed DA font size.
    let single_line = !field.info.multiline && !list;
    let wrap = |size: f32| {
        let mut lines = vec![];
        for paragraph in display.split('\n') {
            let mut line = String::new();
            let mut metrics = LineMetrics::default();
            for character in paragraph.chars() {
                let glyph = glyphs[&character];
                let mut candidate = metrics;
                candidate.push(glyph, size);
                if field.info.multiline && !line.is_empty() && candidate.width() > available_width {
                    lines.push(std::mem::take(&mut line));
                    metrics = LineMetrics::default();
                }
                line.push(character);
                metrics.push(glyph, size);
            }
            lines.push(line);
        }
        lines
    };
    let mut size = if style.size > 0.0 {
        style.size
    } else {
        (available_height / (ascent + descent)).min(12.0)
    };
    let mut lines = wrap(size);
    loop {
        let line_height = size * (ascent + descent).max(1.2);
        let fits = if field.info.comb {
            let cell = available_width / field.info.max_length as f32;
            display.chars().all(|character| {
                let mut metrics = LineMetrics::default();
                metrics.push(glyphs[&character], size);
                metrics.width() <= cell + 0.01
            }) && line_metrics(&display, glyphs, size).height() <= available_height + 0.01
        } else {
            lines
                .iter()
                .all(|line| line_metrics(line, glyphs, size).width() <= available_width + 0.01)
                && if single_line {
                    line_metrics(&display, glyphs, size).height() <= available_height + 0.01
                } else {
                    list && line_height <= available_height
                        || !list && lines.len() as f32 * line_height <= available_height + 0.01
                }
        };
        if fits {
            break;
        }
        if style.size > 0.0 || size <= 4.0 {
            return Err(invalid(
                "The value does not fit the field's appearance. Use shorter text; fixed-size fields are not silently clipped.",
            ));
        }
        size = (size - 0.5).max(4.0);
        lines = wrap(size);
    }
    let mut operations = vec![Operation::new("q", vec![])];
    if let Some(background) = style.background {
        operations.push(Operation::new(
            "rg",
            background.into_iter().map(Object::Real).collect(),
        ));
        operations.push(Operation::new(
            "re",
            vec![0.into(), 0.into(), style.width.into(), style.height.into()],
        ));
        operations.push(Operation::new("f", vec![]));
    }
    if let Some(border) = style.border
        && style.border_width > 0.0
    {
        operations.push(Operation::new(
            "RG",
            border.into_iter().map(Object::Real).collect(),
        ));
        operations.push(Operation::new("w", vec![style.border_width.into()]));
        if !style.dash.is_empty() {
            operations.push(Operation::new("d", vec![numbers(&style.dash), 0.into()]));
        }
        let half = style.border_width / 2.0;
        if style.underline {
            operations.push(Operation::new("m", vec![half.into(), half.into()]));
            operations.push(Operation::new(
                "l",
                vec![(style.width - half).into(), half.into()],
            ));
        } else {
            operations.push(Operation::new(
                "re",
                vec![
                    half.into(),
                    half.into(),
                    (style.width - style.border_width).into(),
                    (style.height - style.border_width).into(),
                ],
            ));
        }
        operations.push(Operation::new("S", vec![]));
    }
    operations.push(Operation::new(
        "re",
        vec![
            padding.into(),
            padding.into(),
            available_width.into(),
            available_height.into(),
        ],
    ));
    operations.push(Operation::new("W", vec![]));
    operations.push(Operation::new("n", vec![]));
    operations.push(Operation::new("BMC", vec![Object::Name(b"Tx".to_vec())]));
    let line_height = size * (ascent + descent).max(1.2);
    let selected = match value {
        FieldValue::Choice { indices, .. } => indices.as_slice(),
        _ => &[],
    };
    let top = if list {
        selected.first().copied().unwrap_or(0)
    } else {
        0
    };
    let visible = if list {
        ((available_height / line_height).floor() as usize).max(1)
    } else {
        lines.len()
    };
    for (row, line) in lines.iter().enumerate().skip(top).take(visible) {
        let metrics = line_metrics(line, glyphs, size);
        let y = if single_line {
            padding + (available_height - metrics.height()) / 2.0 - metrics.bottom
        } else {
            style.height - padding - ascent * size - (row - top) as f32 * line_height
        };
        if list && selected.contains(&row) {
            operations.push(Operation::new(
                "rg",
                vec![0.76.into(), 0.85.into(), 1.0.into()],
            ));
            operations.push(Operation::new(
                "re",
                vec![
                    padding.into(),
                    (y - descent * size).into(),
                    available_width.into(),
                    line_height.into(),
                ],
            ));
            operations.push(Operation::new("f", vec![]));
        }
        operations.push(Operation::new("BT", vec![]));
        operations.push(Operation::new(
            "Tf",
            vec![Object::Name(font.name.as_bytes().to_vec()), size.into()],
        ));
        operations.push(Operation::new(
            "rg",
            style.color.into_iter().map(Object::Real).collect(),
        ));
        if field.info.comb {
            let cell = available_width / field.info.max_length as f32;
            for (index, character) in line.chars().enumerate() {
                let mut glyph = LineMetrics::default();
                glyph.push(glyphs[&character], size);
                let x = padding + index as f32 * cell + (cell - glyph.width()) / 2.0 - glyph.left;
                operations.push(Operation::new(
                    "Tm",
                    vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
                ));
                operations.push(Operation::new(
                    "Tj",
                    vec![Object::String(
                        font.encoded[&character].to_be_bytes().to_vec(),
                        StringFormat::Hexadecimal,
                    )],
                ));
            }
        } else {
            let remaining = available_width - metrics.width();
            let x = padding - metrics.left
                + match style.align {
                    1 => remaining / 2.0,
                    2 => remaining,
                    _ => 0.0,
                };
            operations.push(Operation::new(
                "Tm",
                vec![1.into(), 0.into(), 0.into(), 1.into(), x.into(), y.into()],
            ));
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(
                    line.chars()
                        .flat_map(|character| font.encoded[&character].to_be_bytes())
                        .collect(),
                    StringFormat::Hexadecimal,
                )],
            ));
        }
        operations.push(Operation::new("ET", vec![]));
    }
    operations.extend([Operation::new("EMC", vec![]), Operation::new("Q", vec![])]);
    let content = Content { operations }.encode()?;
    if content.len() > MAX_TEXT_BYTES {
        return Err(invalid(
            "A widget appearance exceeds the 1 MiB resource limit.",
        ));
    }
    *appearance_bytes += content.len();
    if *appearance_bytes > MAX_APPEARANCE_BYTES {
        return Err(invalid(
            "The form changes exceed the 32 MiB appearance resource limit.",
        ));
    }
    Ok(document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Form",
            "BBox" => numbers(&[0.0, 0.0, style.width, style.height]), "Matrix" => numbers(&style.matrix),
            "Resources" => dictionary! { "Font" => dictionary! { font.name.as_bytes() => font.id } },
        },
        content,
    )))
}
