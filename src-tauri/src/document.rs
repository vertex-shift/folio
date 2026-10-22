use std::{
    collections::{HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, content::Content, content::Operation,
    decode_text_string, dictionary, text_string,
};
use serde::{Deserialize, Serialize};

pub const MAX_DOCUMENT_BYTES: usize = 128 * 1024 * 1024;
const HISTORY_BYTES: usize = 64 * 1024 * 1024;
const HISTORY_STEPS: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("The PDF could not be processed: {0}")]
    Pdf(#[from] lopdf::Error),
    #[error("The file operation failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    pub key: String,
    pub rotation: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationInfo {
    pub id: String,
    pub page: usize,
    pub kind: String,
    pub text: String,
    pub author: String,
    pub rect: [f32; 4],
    pub color: [f32; 3],
    pub deletable: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub id: u64,
    pub name: String,
    pub revision: u64,
    pub page_count: usize,
    pub byte_length: usize,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub read_only_reason: Option<String>,
    pub pages: Vec<PageInfo>,
    pub annotations: Vec<AnnotationInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Rotate {
        page: usize,
        degrees: i32,
    },
    Move {
        from: usize,
        to: usize,
    },
    Delete {
        page: usize,
    },
    Highlight {
        page: usize,
        quads: Vec<[f32; 8]>,
        color: [f32; 3],
        text: String,
    },
    Note {
        page: usize,
        point: [f32; 2],
        text: String,
    },
    RemoveAnnotation {
        id: String,
    },
}

struct Snapshot {
    bytes: Vec<u8>,
    revision: u64,
}

pub struct Session {
    pub id: u64,
    pub name: String,
    pub source_path: Option<PathBuf>,
    pub saved_path: Option<PathBuf>,
    document: Document,
    bytes: Vec<u8>,
    revision: u64,
    saved_revision: u64,
    next_revision: u64,
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    read_only_reason: Option<String>,
}

impl Session {
    pub fn from_bytes(
        bytes: Vec<u8>,
        name: String,
        source_path: Option<PathBuf>,
        id: u64,
    ) -> Result<Self> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid("This PDF is larger than the 128 MB MVP limit."));
        }
        let document = Document::load_mem(&bytes)?;
        if document.is_encrypted() || document.was_encrypted() {
            return Err(invalid(
                "Password-protected PDFs are not supported yet. Open an unencrypted copy.",
            ));
        }
        if document.get_pages().is_empty() {
            return Err(invalid("This PDF has no readable pages."));
        }
        let signed = document.objects.values().any(|object| {
            object.as_dict().is_ok_and(|dict| {
                dict.get(b"Type").and_then(Object::as_name).ok() == Some(b"Sig")
                    || (dict.has(b"ByteRange") && dict.has(b"Contents"))
            })
        });
        let session = Self {
            id,
            name,
            source_path,
            saved_path: None,
            document,
            bytes,
            revision: 0,
            saved_revision: 0,
            next_revision: 1,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            read_only_reason: signed.then(|| {
                "Digitally signed PDF: read-only. Rewriting this file would invalidate its signatures."
                    .to_owned()
            }),
        };
        session.info()?;
        Ok(session)
    }

    pub fn from_path(path: PathBuf, id: u64) -> Result<Self> {
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(invalid("Choose a PDF file, not a folder."));
        }
        if metadata.len() > MAX_DOCUMENT_BYTES as u64 {
            return Err(invalid("This PDF is larger than the 128 MB MVP limit."));
        }
        let name = path
            .file_name()
            .ok_or_else(|| invalid("The selected file has no name."))?
            .to_string_lossy()
            .into_owned();
        Self::from_bytes(fs::read(&path)?, name, Some(path), id)
    }

    pub fn demo(id: u64) -> Result<Self> {
        Self::from_bytes(
            include_bytes!("../resources/folio-guide.pdf").to_vec(),
            "Folio quick start.pdf".to_owned(),
            None,
            id,
        )
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn info(&self) -> Result<DocumentInfo> {
        let pages = self
            .document
            .get_pages()
            .values()
            .map(|id| {
                Ok(PageInfo {
                    key: object_key(*id),
                    rotation: page_rotation(&self.document, *id)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(DocumentInfo {
            id: self.id,
            name: self.name.clone(),
            revision: self.revision,
            page_count: pages.len(),
            byte_length: self.bytes.len(),
            dirty: self.is_dirty(),
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            read_only_reason: self.read_only_reason.clone(),
            pages,
            annotations: annotations(&self.document)?,
        })
    }

    pub fn apply(&mut self, edit: Edit) -> Result<DocumentInfo> {
        if let Some(reason) = &self.read_only_reason {
            return Err(invalid(reason));
        }
        let mut next = self.document.clone();
        apply_edit(&mut next, edit, self.next_revision)?;
        annotations(&next)?;
        let mut bytes = Vec::new();
        next.save_to(&mut bytes)?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid("The edited PDF exceeds the 128 MB MVP limit."));
        }
        self.undo.push_back(Snapshot {
            bytes: std::mem::replace(&mut self.bytes, bytes),
            revision: self.revision,
        });
        self.document = next;
        self.revision = self.next_revision;
        self.next_revision += 1;
        self.redo.clear();
        self.trim_history();
        self.info()
    }

    pub fn undo(&mut self) -> Result<DocumentInfo> {
        let snapshot = self
            .undo
            .back()
            .ok_or_else(|| invalid("There is no change to undo."))?;
        let document = Document::load_mem(&snapshot.bytes)?;
        let snapshot = self
            .undo
            .pop_back()
            .ok_or_else(|| invalid("The undo history is unavailable."))?;
        self.redo.push_back(Snapshot {
            bytes: std::mem::replace(&mut self.bytes, snapshot.bytes),
            revision: self.revision,
        });
        self.document = document;
        self.revision = snapshot.revision;
        self.info()
    }

    pub fn redo(&mut self) -> Result<DocumentInfo> {
        let snapshot = self
            .redo
            .back()
            .ok_or_else(|| invalid("There is no change to redo."))?;
        let document = Document::load_mem(&snapshot.bytes)?;
        let snapshot = self
            .redo
            .pop_back()
            .ok_or_else(|| invalid("The redo history is unavailable."))?;
        self.undo.push_back(Snapshot {
            bytes: std::mem::replace(&mut self.bytes, snapshot.bytes),
            revision: self.revision,
        });
        self.document = document;
        self.revision = snapshot.revision;
        self.info()
    }

    pub fn suggested_name(&self) -> String {
        if self.saved_path.is_some() {
            return self.name.clone();
        }
        let stem = Path::new(&self.name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        format!("{stem}-edited.pdf")
    }

    pub fn save_as(&mut self, path: &Path) -> Result<DocumentInfo> {
        atomic_write(path, &self.bytes)?;
        self.record_saved(path.to_path_buf(), self.revision);
        self.info()
    }

    pub fn record_saved(&mut self, path: PathBuf, revision: u64) {
        self.saved_revision = revision;
        if let Some(name) = path.file_name() {
            self.name = name.to_string_lossy().into_owned();
        }
        self.saved_path = Some(path);
    }

    fn trim_history(&mut self) {
        while self.undo.len() > HISTORY_STEPS
            || self.undo.iter().map(|s| s.bytes.len()).sum::<usize>() > HISTORY_BYTES
        {
            self.undo.pop_front();
        }
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

fn object_key(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}

fn page_id(document: &Document, index: usize) -> Result<ObjectId> {
    document
        .get_pages()
        .values()
        .nth(index)
        .copied()
        .ok_or_else(|| invalid("That page is no longer in the document."))
}

fn inherited(document: &Document, id: ObjectId, key: &[u8]) -> Result<Option<Object>> {
    let mut current = id;
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(current) || visited.len() > 64 {
            return Err(invalid(
                "The PDF has a cyclic or excessively deep page tree.",
            ));
        }
        let dictionary = document.get_dictionary(current)?;
        if let Ok(value) = dictionary.get(key) {
            return Ok(Some(value.clone()));
        }
        match dictionary.get(b"Parent") {
            Ok(parent) => current = parent.as_reference()?,
            Err(_) => return Ok(None),
        }
    }
}

fn page_rotation(document: &Document, id: ObjectId) -> Result<i32> {
    let Some(value) = inherited(document, id, b"Rotate")? else {
        return Ok(0);
    };
    let rotation = document.dereference(&value)?.1.as_i64()?;
    if rotation % 90 != 0 {
        return Err(invalid(
            "A page has an invalid rotation; expected a multiple of 90 degrees.",
        ));
    }
    Ok(rotation.rem_euclid(360) as i32)
}

fn page_bounds(document: &Document, id: ObjectId) -> Result<[f32; 4]> {
    let value = inherited(document, id, b"CropBox")?
        .or(inherited(document, id, b"MediaBox")?)
        .ok_or_else(|| invalid("This page does not define its dimensions."))?;
    let array = document.dereference(&value)?.1.as_array()?;
    if array.len() != 4 {
        return Err(invalid("This page has an invalid page rectangle."));
    }
    let mut bounds = [0.0; 4];
    for (index, item) in array.iter().enumerate() {
        bounds[index] = document.dereference(item)?.1.as_float()?;
    }
    if bounds.iter().any(|value| !value.is_finite())
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
    {
        return Err(invalid("This page has invalid dimensions."));
    }
    Ok(bounds)
}

fn apply_edit(document: &mut Document, edit: Edit, revision: u64) -> Result<()> {
    match edit {
        Edit::Rotate { page, degrees } => {
            if ![-90, 90, 180].contains(&degrees) {
                return Err(invalid("Rotate by 90, -90, or 180 degrees."));
            }
            let id = page_id(document, page)?;
            let rotation = (page_rotation(document, id)? + degrees).rem_euclid(360);
            document.get_dictionary_mut(id)?.set("Rotate", rotation);
        }
        Edit::Move { from, to } => {
            let mut order: Vec<_> = document.get_pages().values().copied().collect();
            if from >= order.len() || to >= order.len() || from == to {
                return Err(invalid("Choose two different pages within the document."));
            }
            let id = order.remove(from);
            order.insert(to, id);
            rebuild_page_tree(document, &order)?;
        }
        Edit::Delete { page } => {
            let mut order: Vec<_> = document.get_pages().values().copied().collect();
            if order.len() <= 1 {
                return Err(invalid("A PDF must keep at least one page."));
            }
            if page >= order.len() {
                return Err(invalid("That page is no longer in the document."));
            }
            if has_form_fields(document)? {
                return Err(invalid(
                    "Removing pages from interactive forms is not supported yet. Use a flattened copy.",
                ));
            }
            let removed = order.remove(page);
            let replacement = order[page.min(order.len() - 1)];
            for object in document.objects.values_mut() {
                retarget_destinations(object, removed, replacement);
            }
            rebuild_page_tree(document, &order)?;
            document.delete_object(removed);
            document.prune_objects();
        }
        Edit::Highlight {
            page,
            quads,
            color,
            text,
        } => {
            let id = page_id(document, page)?;
            let bounds = page_bounds(document, id)?;
            validate_text(&text, true)?;
            if quads.is_empty() || quads.len() > 500 {
                return Err(invalid("Select between 1 and 500 text fragments."));
            }
            if color
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
            {
                return Err(invalid("The highlight color is invalid."));
            }
            let mut rect = [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ];
            for quad in &quads {
                for point in quad.chunks_exact(2) {
                    validate_point([point[0], point[1]], bounds)?;
                    rect[0] = rect[0].min(point[0]);
                    rect[1] = rect[1].min(point[1]);
                    rect[2] = rect[2].max(point[0]);
                    rect[3] = rect[3].max(point[1]);
                }
            }
            if rect[2] - rect[0] < 0.1 || rect[3] - rect[1] < 0.1 {
                return Err(invalid("The selected highlight has no area."));
            }
            let appearance = highlight_appearance(document, &quads, rect, color)?;
            let annotation = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Highlight",
                "Rect" => numbers(&rect),
                "QuadPoints" => Object::Array(quads.iter().flatten().map(|v| Object::Real(*v)).collect()),
                "C" => numbers(&color),
                "CA" => 0.32_f32,
                "F" => 4,
                "T" => text_string("Folio"),
                "Contents" => text_string(&text),
                "NM" => text_string(&format!("folio-{revision}-{}", document.max_id + 1)),
                "P" => id,
                "AP" => dictionary! { "N" => appearance },
            };
            append_annotation(document, id, annotation)?;
        }
        Edit::Note { page, point, text } => {
            validate_text(&text, false)?;
            let id = page_id(document, page)?;
            let bounds = page_bounds(document, id)?;
            validate_point(point, bounds)?;
            let side = 20.0_f32
                .min(bounds[2] - bounds[0])
                .min(bounds[3] - bounds[1]);
            let x = point[0].clamp(bounds[0], bounds[2] - side);
            let y = point[1].clamp(bounds[1] + side, bounds[3]);
            let annotation = dictionary! {
                "Type" => "Annot",
                "Subtype" => "Text",
                "Rect" => numbers(&[x, y - side, x + side, y]),
                "C" => numbers(&[1.0, 0.77, 0.20]),
                "F" => 4,
                "T" => text_string("Folio"),
                "Contents" => text_string(text.trim()),
                "Name" => "Comment",
                "Open" => false,
                "NM" => text_string(&format!("folio-{revision}-{}", document.max_id + 1)),
                "P" => id,
            };
            append_annotation(document, id, annotation)?;
        }
        Edit::RemoveAnnotation { id } => {
            let annotation = annotations(document)?
                .into_iter()
                .find(|annotation| annotation.id == id && annotation.deletable)
                .ok_or_else(|| invalid("This annotation cannot be removed."))?;
            let (number, generation) = annotation
                .id
                .split_once(':')
                .ok_or_else(|| invalid("The annotation identifier is invalid."))?;
            let object_id = (
                number
                    .parse()
                    .map_err(|_| invalid("Invalid annotation identifier."))?,
                generation
                    .parse()
                    .map_err(|_| invalid("Invalid annotation identifier."))?,
            );
            let id = page_id(document, annotation.page)?;
            let mut entries = annotation_entries(document, id)?;
            entries.retain(|entry| entry.as_reference().ok() != Some(object_id));
            document.get_dictionary_mut(id)?.set("Annots", entries);
            document.delete_object(object_id);
            document.prune_objects();
        }
    }
    Ok(())
}

fn rebuild_page_tree(document: &mut Document, order: &[ObjectId]) -> Result<()> {
    // Reparenting without copying these inherited values loses fonts, page boxes,
    // or rotations on PDFs whose original page tree supplies them.
    let mut inherited_values = Vec::new();
    for id in order {
        let mut values = Vec::new();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if let Some(value) = inherited(document, *id, key)? {
                values.push((key.to_vec(), value));
            }
        }
        inherited_values.push((*id, values));
    }
    let root = document.add_object(dictionary! {
        "Type" => "Pages",
        "Count" => order.len() as i64,
        "Kids" => Object::Array(order.iter().copied().map(Object::Reference).collect()),
    });
    for (id, values) in inherited_values {
        let page = document.get_dictionary_mut(id)?;
        for (key, value) in values {
            page.set(key, value);
        }
        page.set("Parent", root);
    }
    document.catalog_mut()?.set("Pages", root);
    document.prune_objects();
    Ok(())
}

fn retarget_destinations(object: &mut Object, removed: ObjectId, replacement: ObjectId) {
    match object {
        Object::Array(values) => {
            let is_destination = values.len() >= 2
                && values[0].as_reference().ok() == Some(removed)
                && values[1].as_name().is_ok_and(|name| {
                    [
                        b"XYZ".as_slice(),
                        b"Fit",
                        b"FitH",
                        b"FitV",
                        b"FitR",
                        b"FitB",
                        b"FitBH",
                        b"FitBV",
                    ]
                    .contains(&name)
                });
            if is_destination {
                values[0] = Object::Reference(replacement);
            }
            for value in values {
                retarget_destinations(value, removed, replacement);
            }
        }
        Object::Dictionary(dictionary) => {
            for (_, value) in dictionary.iter_mut() {
                retarget_destinations(value, removed, replacement);
            }
        }
        Object::Stream(stream) => {
            for (_, value) in stream.dict.iter_mut() {
                retarget_destinations(value, removed, replacement);
            }
        }
        _ => {}
    }
}

fn has_form_fields(document: &Document) -> Result<bool> {
    let Ok(form) = document.catalog()?.get(b"AcroForm") else {
        return Ok(false);
    };
    let form = document.dereference(form)?.1.as_dict()?;
    if form.has(b"XFA") {
        return Ok(true);
    }
    match form.get(b"Fields") {
        Ok(fields) => Ok(!document.dereference(fields)?.1.as_array()?.is_empty()),
        Err(_) => Ok(false),
    }
}

fn validate_text(text: &str, allow_empty: bool) -> Result<()> {
    if (!allow_empty && text.trim().is_empty()) || text.chars().count() > 4000 {
        return Err(invalid(
            "Use between 1 and 4,000 characters for a note or selected passage.",
        ));
    }
    Ok(())
}

fn validate_point(point: [f32; 2], bounds: [f32; 4]) -> Result<()> {
    if point.iter().any(|value| !value.is_finite())
        || point[0] < bounds[0] - 1.0
        || point[0] > bounds[2] + 1.0
        || point[1] < bounds[1] - 1.0
        || point[1] > bounds[3] + 1.0
    {
        return Err(invalid(
            "The annotation must be placed inside the PDF page.",
        ));
    }
    Ok(())
}

fn numbers(values: &[f32]) -> Object {
    Object::Array(values.iter().map(|value| Object::Real(*value)).collect())
}

fn highlight_appearance(
    document: &mut Document,
    quads: &[[f32; 8]],
    rect: [f32; 4],
    color: [f32; 3],
) -> Result<ObjectId> {
    let mut operations = vec![
        Operation::new("q", vec![]),
        Operation::new("gs", vec![Object::Name(b"GS0".to_vec())]),
        Operation::new(
            "rg",
            color.iter().map(|value| Object::Real(*value)).collect(),
        ),
    ];
    for quad in quads {
        for (step, index) in [0, 2, 6, 4].into_iter().enumerate() {
            operations.push(Operation::new(
                if step == 0 { "m" } else { "l" },
                vec![
                    (quad[index] - rect[0]).into(),
                    (quad[index + 1] - rect[1]).into(),
                ],
            ));
        }
        operations.push(Operation::new("h", vec![]));
        operations.push(Operation::new("f", vec![]));
    }
    operations.push(Operation::new("Q", vec![]));
    let data = Content { operations }.encode()?;
    Ok(document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "FormType" => 1,
            "BBox" => numbers(&[0.0, 0.0, rect[2] - rect[0], rect[3] - rect[1]]),
            "Resources" => dictionary! {
                "ExtGState" => dictionary! {
                    "GS0" => dictionary! {
                        "Type" => "ExtGState",
                        "ca" => 0.32_f32,
                        "CA" => 0.32_f32,
                        "BM" => "Multiply",
                    },
                },
            },
        },
        data,
    )))
}

fn annotation_entries(document: &Document, id: ObjectId) -> Result<Vec<Object>> {
    let page = document.get_dictionary(id)?;
    let Ok(value) = page.get(b"Annots") else {
        return Ok(Vec::new());
    };
    Ok(document.dereference(value)?.1.as_array()?.clone())
}

fn append_annotation(
    document: &mut Document,
    page: ObjectId,
    annotation: Dictionary,
) -> Result<()> {
    let mut entries = annotation_entries(document, page)?;
    let id = document.add_object(annotation);
    entries.push(Object::Reference(id));
    document.get_dictionary_mut(page)?.set("Annots", entries);
    Ok(())
}

fn annotations(document: &Document) -> Result<Vec<AnnotationInfo>> {
    let mut result = Vec::new();
    for (page, id) in document.get_pages().values().enumerate() {
        for (index, entry) in annotation_entries(document, *id)?.iter().enumerate() {
            let dictionary = document.dereference(entry)?.1.as_dict()?;
            let subtype = dictionary.get(b"Subtype")?.as_name()?;
            let kind = match subtype {
                b"Text" => "note",
                b"Highlight" => "highlight",
                _ => continue,
            };
            let rectangle = document
                .dereference(dictionary.get(b"Rect")?)?
                .1
                .as_array()?;
            if rectangle.len() != 4 {
                return Err(invalid("A PDF annotation has an invalid rectangle."));
            }
            let mut rect = [0.0; 4];
            for (index, value) in rectangle.iter().enumerate() {
                rect[index] = document.dereference(value)?.1.as_float()?;
            }
            let text = |key: &[u8]| -> Result<Option<String>> {
                dictionary
                    .get(key)
                    .ok()
                    .map(|value| Ok(decode_text_string(document.dereference(value)?.1)?))
                    .transpose()
            };
            let mut color = [1.0, 0.77, 0.20];
            if let Ok(value) = dictionary.get(b"C") {
                let values = document.dereference(value)?.1.as_array()?;
                let values = values
                    .iter()
                    .map(|value| Ok(document.dereference(value)?.1.as_float()?))
                    .collect::<Result<Vec<_>>>()?;
                match values.as_slice() {
                    [] => {}
                    [gray] => color = [*gray; 3],
                    [red, green, blue] => color = [*red, *green, *blue],
                    [cyan, magenta, yellow, black] => {
                        color = [
                            1.0 - (cyan + black).min(1.0),
                            1.0 - (magenta + black).min(1.0),
                            1.0 - (yellow + black).min(1.0),
                        ];
                    }
                    _ => return Err(invalid("A PDF annotation has an invalid color.")),
                }
            }
            let reference = entry.as_reference().ok();
            result.push(AnnotationInfo {
                id: reference
                    .map(object_key)
                    .unwrap_or_else(|| format!("direct-{page}-{index}")),
                page,
                kind: kind.to_owned(),
                text: text(b"Contents")?.unwrap_or_default(),
                author: text(b"T")?.unwrap_or_else(|| "PDF annotation".to_owned()),
                rect,
                color,
                deletable: reference.is_some(),
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_document() -> (Vec<u8>, Vec<ObjectId>) {
        let mut document = Document::with_version("1.7");
        let root = document.new_object_id();
        let branch = document.new_object_id();
        let font = document.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources = document.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
        let mut pages = Vec::new();
        for index in 0..3 {
            let content = document.add_object(Stream::new(
                dictionary! {},
                format!("BT /F1 12 Tf 50 100 Td (Unique page {index}) Tj ET").into_bytes(),
            ));
            pages.push(document.add_object(dictionary! {
                "Type" => "Page", "Parent" => branch, "Contents" => content,
            }));
        }
        document.objects.insert(
            branch,
            dictionary! {
                "Type" => "Pages", "Parent" => root, "Count" => 3,
                "Kids" => Object::Array(pages.iter().copied().map(Object::Reference).collect()),
                "Resources" => resources, "MediaBox" => numbers(&[10.0, 20.0, 410.0, 620.0]),
                "CropBox" => numbers(&[20.0, 30.0, 400.0, 600.0]), "Rotate" => 90,
            }
            .into(),
        );
        document.objects.insert(
            root,
            dictionary! {
                "Type" => "Pages", "Count" => 3, "Kids" => vec![Object::Reference(branch)],
            }
            .into(),
        );
        let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => root });
        document.trailer.set("Root", catalog);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        (bytes, pages)
    }

    fn session() -> Session {
        let (bytes, _) = nested_document();
        Session::from_bytes(bytes, "test.pdf".into(), None, 1).unwrap()
    }

    #[test]
    fn reordering_preserves_inherited_page_properties_and_ids() {
        let mut session = session();
        let original = session.info().unwrap();
        session.apply(Edit::Move { from: 0, to: 2 }).unwrap();
        let info = session.info().unwrap();
        assert_eq!(info.pages[2].key, original.pages[0].key);
        for id in session.document.get_pages().values() {
            assert_eq!(page_rotation(&session.document, *id).unwrap(), 90);
            assert_eq!(
                page_bounds(&session.document, *id).unwrap(),
                [20.0, 30.0, 400.0, 600.0]
            );
            assert!(
                session
                    .document
                    .get_dictionary(*id)
                    .unwrap()
                    .has(b"Resources")
            );
        }
        let reopened =
            Session::from_bytes(session.bytes.clone(), "copy.pdf".into(), None, 2).unwrap();
        assert_eq!(reopened.info().unwrap().pages[2].key, original.pages[0].key);
    }

    #[test]
    fn rotate_and_undo_respect_inherited_rotation_and_dirty_state() {
        let mut session = session();
        assert!(!session.is_dirty());
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        assert_eq!(session.info().unwrap().pages[0].rotation, 180);
        assert!(session.is_dirty());
        session.undo().unwrap();
        assert!(!session.is_dirty());
        assert_eq!(session.info().unwrap().pages[0].rotation, 90);
        session.redo().unwrap();
        assert!(session.is_dirty());
        assert_eq!(session.info().unwrap().pages[0].rotation, 180);
    }

    #[test]
    fn unicode_annotations_survive_save_reopen_and_removal() {
        let mut session = session();
        session
            .apply(Edit::Note {
                page: 0,
                point: [60.0, 160.0],
                text: "Review: \u{6771}\u{4eac} \u{2713}".into(),
            })
            .unwrap();
        session
            .apply(Edit::Highlight {
                page: 1,
                quads: vec![[40.0, 150.0, 190.0, 150.0, 40.0, 130.0, 190.0, 130.0]],
                color: [1.0, 0.88, 0.2],
                text: "A selected passage".into(),
            })
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reviewed.pdf");
        session.save_as(&path).unwrap();
        assert!(!session.is_dirty());
        let mut reopened = Session::from_path(path, 2).unwrap();
        let info = reopened.info().unwrap();
        assert_eq!(info.annotations.len(), 2);
        assert_eq!(
            info.annotations[0].text,
            "Review: \u{6771}\u{4eac} \u{2713}"
        );
        let annotation = reopened
            .document
            .get_page_annotations(page_id(&reopened.document, 1).unwrap())
            .unwrap()[0];
        assert!(annotation.has(b"AP"));
        reopened
            .apply(Edit::RemoveAnnotation {
                id: info.annotations[0].id.clone(),
            })
            .unwrap();
        assert_eq!(reopened.info().unwrap().annotations.len(), 1);
        reopened.undo().unwrap();
        assert_eq!(reopened.info().unwrap().annotations.len(), 2);
    }

    #[test]
    fn deletion_removes_content_and_retargets_bookmarks() {
        let mut session = session();
        let ids: Vec<_> = session.document.get_pages().values().copied().collect();
        let bookmark = session.document.add_object(dictionary! {
            "Title" => text_string("Removed page"),
            "Dest" => vec![Object::Reference(ids[0]), Object::Name(b"Fit".to_vec())],
        });
        session
            .document
            .catalog_mut()
            .unwrap()
            .set("TestDestination", bookmark);
        session.apply(Edit::Delete { page: 0 }).unwrap();
        assert_eq!(session.info().unwrap().page_count, 2);
        assert!(!session.document.objects.contains_key(&ids[0]));
        let destination = session
            .document
            .get_dictionary(bookmark)
            .unwrap()
            .get(b"Dest")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(destination[0].as_reference().unwrap(), ids[1]);
        assert!(!String::from_utf8_lossy(session.bytes()).contains("Unique page 0"));
    }

    #[test]
    fn invalid_edits_are_transactional_and_last_page_is_protected() {
        let mut session = session();
        let before = session.bytes.clone();
        assert!(
            session
                .apply(Edit::Highlight {
                    page: 0,
                    quads: vec![[f32::NAN; 8]],
                    color: [1.0, 1.0, 0.0],
                    text: "".into(),
                })
                .is_err()
        );
        assert_eq!(before, session.bytes);
        assert!(!session.info().unwrap().can_undo);
        assert!(
            session
                .apply(Edit::Note {
                    page: 0,
                    point: [-200.0, 100.0],
                    text: "Outside".into(),
                })
                .is_err()
        );
        session.apply(Edit::Delete { page: 0 }).unwrap();
        session.apply(Edit::Delete { page: 0 }).unwrap();
        assert!(session.apply(Edit::Delete { page: 0 }).is_err());
        assert_eq!(session.info().unwrap().page_count, 1);
    }

    #[test]
    fn a_new_edit_after_undo_clears_redo() {
        let mut session = session();
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        session.undo().unwrap();
        session
            .apply(Edit::Rotate {
                page: 1,
                degrees: -90,
            })
            .unwrap();
        assert!(!session.info().unwrap().can_redo);
        assert!(session.redo().is_err());
    }

    #[test]
    fn signed_documents_are_viewable_but_never_rewritten() {
        let mut session = session();
        session.document.add_object(dictionary! {
            "Type" => "Sig", "ByteRange" => vec![0.into(), 1.into(), 2.into(), 3.into()],
            "Contents" => Object::string_literal("Example signature marker"),
        });
        let mut bytes = Vec::new();
        session.document.save_to(&mut bytes).unwrap();
        let mut signed = Session::from_bytes(bytes.clone(), "signed.pdf".into(), None, 2).unwrap();
        assert!(signed.info().unwrap().read_only_reason.is_some());
        assert!(
            signed
                .apply(Edit::Rotate {
                    page: 0,
                    degrees: 90
                })
                .is_err()
        );
        assert_eq!(signed.bytes(), bytes);
    }

    #[test]
    fn source_is_unchanged_when_saving_a_copy_and_undo_tracks_saved_revision() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.pdf");
        let copy = directory.path().join("copy.pdf");
        let (bytes, _) = nested_document();
        fs::write(&original, &bytes).unwrap();
        let mut session = Session::from_path(original.clone(), 1).unwrap();
        session
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        session.save_as(&copy).unwrap();
        assert_eq!(fs::read(original).unwrap(), bytes);
        session.undo().unwrap();
        assert!(session.is_dirty());
        session.redo().unwrap();
        assert!(!session.is_dirty());
        assert!(Session::from_path(copy, 2).is_ok());
    }

    #[test]
    fn demo_has_three_pages_and_invalid_files_fail_explicitly() {
        assert_eq!(Session::demo(1).unwrap().info().unwrap().page_count, 3);
        assert!(Session::from_bytes(b"not a PDF".to_vec(), "bad.pdf".into(), None, 2).is_err());
    }
}
