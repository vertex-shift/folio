use std::collections::{HashMap, HashSet};

use lopdf::{Dictionary, Document, Object, ObjectId, decode_text_string};
use serde::Serialize;

use crate::{
    document::{Result, invalid},
    page_geometry::PageGeometry,
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureWidgetInfo {
    pub id: String,
    pub page: usize,
    pub rect: [f32; 4],
    pub visible: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureFieldInfo {
    pub id: String,
    pub name: String,
    pub widgets: Vec<SignatureWidgetInfo>,
    pub blocked_reason: Option<String>,
}

pub(super) struct Widget {
    pub id: ObjectId,
    pub rect: [f32; 4],
    pub geometry: PageGeometry,
}

pub(super) struct Field {
    pub id: ObjectId,
    pub signature: Option<ObjectId>,
    pub widgets: Vec<Widget>,
    pub info: SignatureFieldInfo,
}

pub(super) struct Fields {
    pub fields: Vec<Field>,
    pub visited: HashSet<ObjectId>,
}

pub(super) fn key(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}

fn optional<'a>(
    document: &'a Document,
    dictionary: &'a Dictionary,
    name: &[u8],
) -> Result<Option<&'a Object>> {
    dictionary
        .get(name)
        .ok()
        .map(|value| document.dereference(value).map(|(_, value)| value))
        .transpose()
        .map(|value| value.filter(|value| !matches!(value, Object::Null)))
        .map_err(Into::into)
}

fn text(document: &Document, value: &Object) -> Result<String> {
    let value = document.dereference(value)?.1;
    if value.as_str()?.len() > 16_384 {
        return Err(invalid("A signature field name exceeds the 16 KiB limit."));
    }
    decode_text_string(value)
        .map_err(|_| invalid("A signature field name has invalid text encoding."))
}

struct Reader<'a> {
    document: &'a Document,
    pages: Vec<ObjectId>,
    locations: HashMap<ObjectId, Vec<usize>>,
    visited: HashSet<ObjectId>,
    fields: Vec<Field>,
}

impl Reader<'_> {
    fn mark(&mut self, id: ObjectId) -> Result<()> {
        if !self.visited.insert(id) || self.visited.len() > 4096 {
            return Err(invalid(
                "Signing requires an unambiguous form tree without shared/cyclic nodes, within 4,096 nodes.",
            ));
        }
        Ok(())
    }

    fn walk(
        &mut self,
        id: ObjectId,
        parent: Option<ObjectId>,
        inherited: &Dictionary,
        prefix: &str,
        actions: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return Err(invalid("The signing field tree exceeds 32 levels."));
        }
        self.mark(id)?;
        let dictionary = self.document.get_dictionary(id)?.clone();
        let actual_parent = dictionary
            .get(b"Parent")
            .ok()
            .map(Object::as_reference)
            .transpose()?;
        if actual_parent != parent {
            return Err(invalid(
                "A signing field's Parent does not match its field tree.",
            ));
        }
        if dictionary.has(b"Lock") || dictionary.has(b"SV") {
            return Err(invalid(
                "Signature field locks and seed-value policies are not implemented. This PDF will not be signed.",
            ));
        }
        let mut attributes = inherited.clone();
        for name in [b"FT".as_slice(), b"Ff", b"V"] {
            if let Ok(value) = dictionary.get(name) {
                attributes.set(name, value.clone());
            }
        }
        let partial = dictionary
            .get(b"T")
            .ok()
            .map(|value| text(self.document, value))
            .transpose()?
            .unwrap_or_default();
        let name = match (prefix.is_empty(), partial.is_empty()) {
            (_, true) => prefix.to_owned(),
            (true, false) => partial,
            (false, false) => format!("{prefix}.{partial}"),
        };
        if name.len() > 16_384 {
            return Err(invalid("A qualified signature field name exceeds 16 KiB."));
        }
        let actions = actions || dictionary.has(b"A") || dictionary.has(b"AA");
        let children = optional(self.document, &dictionary, b"Kids")?
            .map(Object::as_array)
            .transpose()?
            .cloned()
            .unwrap_or_default();
        if children.len() > 4096 {
            return Err(invalid("A signing field has too many children."));
        }
        let children = children
            .iter()
            .map(|value| {
                value.as_reference().map_err(|_| {
                    invalid("Signing requires indirect form fields and widgets, not direct dictionaries.")
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let widget_children = children
            .iter()
            .map(|child| {
                let dictionary = self.document.get_dictionary(*child)?;
                Ok(
                    dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget")
                        && !dictionary.has(b"T")
                        && !dictionary.has(b"Kids"),
                )
            })
            .collect::<Result<Vec<_>>>()?;
        if widget_children.iter().any(|widget| *widget)
            && widget_children.iter().any(|widget| !*widget)
        {
            return Err(invalid(
                "A signing field mixes widget children with logical fields.",
            ));
        }
        let is_signature = optional(self.document, &attributes, b"FT")?
            .map(Object::as_name)
            .transpose()?
            == Some(b"Sig");
        if !children.is_empty() && !widget_children.iter().all(|widget| *widget) {
            if is_signature && optional(self.document, &attributes, b"V")?.is_some() {
                return Err(invalid(
                    "A nonterminal signature field has an ambiguous inherited value.",
                ));
            }
            for child in children {
                self.walk(child, Some(id), &attributes, &name, actions, depth + 1)?;
            }
            return Ok(());
        }
        let widgets = if children.is_empty() {
            if dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget") {
                vec![id]
            } else {
                vec![]
            }
        } else {
            if dictionary.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Widget") {
                return Err(invalid(
                    "A signature field cannot be both a widget and a widget parent.",
                ));
            }
            for child in &children {
                self.mark(*child)?;
                let widget = self.document.get_dictionary(*child)?;
                if widget.get(b"Parent")?.as_reference()? != id
                    || widget.has(b"V")
                    || widget.has(b"FT")
                {
                    return Err(invalid(
                        "A signature widget has an inconsistent parent, type, or independent value.",
                    ));
                }
                if widget.has(b"Lock") || widget.has(b"SV") {
                    return Err(invalid("Signature widget policies are not implemented."));
                }
            }
            children
        };
        if !is_signature {
            return Ok(());
        }
        if self.fields.len() >= 64 || widgets.len() > 16 {
            return Err(invalid(
                "Signing is limited to 64 signature fields and 16 widgets per field.",
            ));
        }
        let signature = match attributes.get(b"V") {
            Err(_) | Ok(Object::Null) => None,
            Ok(Object::Reference(id)) => Some(*id),
            _ => {
                return Err(invalid(
                    "A signature field has a direct or ambiguous nonempty value.",
                ));
            }
        };
        let flags = optional(self.document, &attributes, b"Ff")?
            .map(Object::as_i64)
            .transpose()?
            .unwrap_or(0);
        let mut info = SignatureFieldInfo {
            id: key(id),
            name: if name.is_empty() {
                format!("Unnamed signature {}", key(id))
            } else {
                name
            },
            widgets: vec![],
            blocked_reason: if flags < 0 || flags > u32::MAX as i64 || flags & 1 != 0 {
                Some("This signature field is read-only or has invalid field flags.".into())
            } else if actions {
                Some("Fields with PDF actions or scripts cannot be selected for signing.".into())
            } else if widgets.is_empty() {
                Some("This signature field has no page widget.".into())
            } else {
                None
            },
        };
        let mut resolved = Vec::new();
        for widget_id in widgets {
            let widget = self.document.get_dictionary(widget_id)?;
            let page = match self.locations.get(&widget_id).map(Vec::as_slice) {
                Some([page]) => *page,
                _ => {
                    return Err(invalid(
                        "A signature widget must occur exactly once in page Annots.",
                    ));
                }
            };
            let page_id = self.pages[page];
            if let Ok(value) = widget.get(b"P")
                && value.as_reference()? != page_id
            {
                return Err(invalid(
                    "A signature widget's P entry does not match its page.",
                ));
            }
            let values = self
                .document
                .dereference(widget.get(b"Rect")?)?
                .1
                .as_array()?;
            if values.len() != 4 {
                return Err(invalid("A signature widget has an invalid rectangle."));
            }
            let mut rect = [0.0; 4];
            for (target, value) in rect.iter_mut().zip(values) {
                *target = self.document.dereference(value)?.1.as_float()?;
            }
            if rect.iter().any(|value| !value.is_finite()) {
                return Err(invalid("A signature widget has nonfinite coordinates."));
            }
            if rect[0] > rect[2] {
                rect.swap(0, 2);
            }
            if rect[1] > rect[3] {
                rect.swap(1, 3);
            }
            let geometry = PageGeometry::for_page(self.document, page_id)?;
            let width = (rect[2] - rect[0]) * geometry.user_unit;
            let height = (rect[3] - rect[1]) * geometry.user_unit;
            let (display_width, display_height) = if geometry.rotation % 180 == 0 {
                (width, height)
            } else {
                (height, width)
            };
            let visible = width > 0.0 && height > 0.0;
            let widget_flags = optional(self.document, widget, b"F")?
                .map(Object::as_i64)
                .transpose()?
                .unwrap_or(0);
            let rotation = optional(self.document, widget, b"MK")?
                .map(Object::as_dict)
                .transpose()?
                .map(|mk| optional(self.document, mk, b"R"))
                .transpose()?
                .flatten()
                .map(Object::as_i64)
                .transpose()?
                .unwrap_or(0);
            let reason = if widget.has(b"A") || widget.has(b"AA") {
                Some("Signature widgets with actions or scripts are not supported.")
            } else if widget_flags < 0 || widget_flags & (1 | 2 | 32 | 64 | 128 | 512) != 0 {
                Some("A signature widget is hidden, read-only, or locked.")
            } else if rotation != 0 {
                Some("Signature widgets with an independent MK rotation are not supported.")
            } else if visible
                && (display_width < 72.0
                    || display_height < 32.0
                    || width > 14_400.0
                    || height > 14_400.0
                    || rect[0] < geometry.bounds[0]
                    || rect[1] < geometry.bounds[1]
                    || rect[2] > geometry.bounds[2]
                    || rect[3] > geometry.bounds[3])
            {
                Some(
                    "Visible signature widgets must fit the visible page and measure 72 x 32 to 14,400 x 14,400 physical points when displayed.",
                )
            } else if !visible && (width != 0.0 || height != 0.0) {
                Some(
                    "A signature widget must have either a visible rectangle or zero width and height.",
                )
            } else {
                None
            };
            if let Some(reason) = reason {
                info.blocked_reason.get_or_insert_with(|| reason.into());
            }
            info.widgets.push(SignatureWidgetInfo {
                id: key(widget_id),
                page,
                rect,
                visible,
            });
            resolved.push(Widget {
                id: widget_id,
                rect,
                geometry,
            });
        }
        self.fields.push(Field {
            id,
            signature,
            widgets: resolved,
            info,
        });
        Ok(())
    }
}

pub(super) fn read(document: &Document) -> Result<Fields> {
    let Some(form) = optional(document, document.catalog()?, b"AcroForm")? else {
        return Ok(Fields {
            fields: vec![],
            visited: HashSet::new(),
        });
    };
    let form = form.as_dict()?;
    if form.has(b"XFA") {
        return Err(invalid("Signing XFA forms is not supported."));
    }
    let roots = optional(document, form, b"Fields")?
        .map(Object::as_array)
        .transpose()?
        .cloned()
        .unwrap_or_default();
    if roots.len() > 4096 {
        return Err(invalid("The signing form exceeds 4,096 root fields."));
    }
    let pages: Vec<_> = document.get_pages().into_values().collect();
    let mut locations: HashMap<ObjectId, Vec<usize>> = HashMap::new();
    for (page, id) in pages.iter().enumerate() {
        if let Some(annotations) = optional(document, document.get_dictionary(*id)?, b"Annots")? {
            for annotation in annotations.as_array()? {
                if let Ok(id) = annotation.as_reference() {
                    locations.entry(id).or_default().push(page);
                }
            }
        }
    }
    let mut reader = Reader {
        document,
        pages,
        locations,
        visited: HashSet::new(),
        fields: vec![],
    };
    for root in roots {
        reader.walk(
            root.as_reference()
                .map_err(|_| invalid("Direct form fields are not supported for signing."))?,
            None,
            &Dictionary::new(),
            "",
            false,
            0,
        )?;
    }
    Ok(Fields {
        fields: reader.fields,
        visited: reader.visited,
    })
}

impl Fields {
    pub fn empty_fields(&self) -> Vec<SignatureFieldInfo> {
        self.fields
            .iter()
            .filter(|field| field.signature.is_none())
            .map(|field| field.info.clone())
            .collect()
    }

    pub fn selected(&self, id: &str) -> Result<&Field> {
        let field = self.fields.iter().find(|field| field.info.id == id)
            .ok_or_else(|| invalid("That signature field no longer exists. Refresh and select a current empty field."))?;
        if field.signature.is_some() {
            return Err(invalid("The selected signature field is already signed."));
        }
        if let Some(reason) = &field.info.blocked_reason {
            return Err(invalid(reason));
        }
        Ok(field)
    }
}
