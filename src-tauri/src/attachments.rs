use std::{collections::HashSet, io::Read, path::Path};

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary, text_string};
use serde::Serialize;

use crate::{
    document::{Result, invalid},
    pdf_objects::{resolve, stream_bytes, text, validate_text},
};

pub const MAX_ATTACHMENT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_ATTACHMENT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 128;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub byte_length: usize,
    pub suggested_name: String,
}

struct Entry {
    key: Object,
    specification: Object,
}

struct Tree {
    names: Dictionary,
    root: Dictionary,
    entries: Vec<Entry>,
}

fn walk(
    document: &Document,
    value: &Object,
    depth: usize,
    visited: &mut HashSet<ObjectId>,
    nodes: &mut usize,
    entries: &mut Vec<Entry>,
) -> Result<()> {
    *nodes += 1;
    if depth > 32 || *nodes > 1024 {
        return Err(invalid(
            "The attachment name tree exceeds 32 levels or 1,024 nodes.",
        ));
    }
    if let Object::Reference(id) = value
        && !visited.insert(*id)
    {
        return Err(invalid(
            "The attachment name tree is cyclic or shares a node.",
        ));
    }
    let dictionary = resolve(document, value)?.as_dict()?;
    if dictionary.has(b"Names") && dictionary.has(b"Kids") {
        return Err(invalid(
            "An attachment name-tree node contains both Names and Kids.",
        ));
    }
    if let Ok(value) = dictionary.get(b"Names") {
        let values = resolve(document, value)?.as_array()?;
        if values.len() % 2 != 0 || entries.len() + values.len() / 2 > MAX_ATTACHMENTS {
            return Err(invalid(
                "The attachment name tree is malformed or exceeds 128 files.",
            ));
        }
        for pair in values.chunks_exact(2) {
            text(document, &pair[0], 1024)?;
            let key = resolve(document, &pair[0])?.clone();
            if entries.iter().any(|entry| entry.key == key) {
                return Err(invalid(
                    "The attachment name tree contains duplicate names.",
                ));
            }
            entries.push(Entry {
                key,
                specification: pair[1].clone(),
            });
        }
    }
    if let Ok(value) = dictionary.get(b"Kids") {
        let children = resolve(document, value)?.as_array()?;
        if children.len() > 1024 {
            return Err(invalid("The attachment name tree has too many children."));
        }
        for child in children {
            walk(document, child, depth + 1, visited, nodes, entries)?;
        }
    }
    Ok(())
}

impl Tree {
    fn read(document: &Document) -> Result<Self> {
        let names = document
            .catalog()?
            .get(b"Names")
            .ok()
            .map(|value| {
                Ok::<_, crate::document::Error>(resolve(document, value)?.as_dict()?.clone())
            })
            .transpose()?
            .unwrap_or_default();
        let mut entries = Vec::new();
        let root = if let Ok(value) = names.get(b"EmbeddedFiles") {
            walk(
                document,
                value,
                0,
                &mut HashSet::new(),
                &mut 0,
                &mut entries,
            )?;
            resolve(document, value)?.as_dict()?.clone()
        } else {
            Dictionary::new()
        };
        Ok(Self {
            names,
            root,
            entries,
        })
    }

    fn write(mut self, document: &mut Document) -> Result<()> {
        self.entries.sort_by(|a, b| {
            a.key
                .as_str()
                .unwrap_or_default()
                .cmp(b.key.as_str().unwrap_or_default())
        });
        let mut pairs = Vec::new();
        for entry in self.entries {
            pairs.extend([entry.key, entry.specification]);
        }
        for key in [b"Kids".as_slice(), b"Limits"] {
            self.root.remove(key);
        }
        self.root.set("Names", pairs);
        let root = document.add_object(self.root);
        self.names.set("EmbeddedFiles", root);
        let names = document.add_object(self.names);
        document.catalog_mut()?.set("Names", names);
        Ok(())
    }
}

fn data<'a>(document: &'a Document, entry: &'a Entry) -> Result<(&'a Dictionary, &'a Stream)> {
    let specification = resolve(document, &entry.specification)?.as_dict()?;
    if specification.has(b"RF") {
        return Err(invalid("Related-file attachment chains are not supported."));
    }
    let embedded = resolve(document, specification.get(b"EF")?)?.as_dict()?;
    let value = embedded.get(b"UF").or_else(|_| embedded.get(b"F"))?;
    let stream = resolve(document, value)?.as_stream()?;
    if stream.content.len() > MAX_ATTACHMENT_BYTES {
        return Err(invalid("An encoded attachment exceeds the 16 MiB limit."));
    }
    if let (Ok(unicode), Ok(legacy)) = (embedded.get(b"UF"), embedded.get(b"F"))
        && unicode != legacy
    {
        return Err(invalid(
            "This attachment has different Unicode and legacy file payloads; export it with another reader.",
        ));
    }
    Ok((specification, stream))
}

pub(crate) fn inspect(document: &Document) -> Result<Vec<AttachmentInfo>> {
    let tree = Tree::read(document)?;
    let mut total = 0;
    let mut encoded_total = 0;
    tree.entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let (specification, stream) = data(document, entry)?;
            encoded_total += stream.content.len();
            let bytes = stream_bytes(stream, MAX_ATTACHMENT_BYTES)?;
            total += bytes.len();
            if total > MAX_TOTAL_ATTACHMENT_BYTES || encoded_total > MAX_TOTAL_ATTACHMENT_BYTES {
                return Err(invalid(
                    "The attachment payloads exceed the 64 MiB aggregate limit.",
                ));
            }
            let name = specification
                .get(b"UF")
                .or_else(|_| specification.get(b"F"))
                .ok()
                .map(|value| text(document, value, 1024))
                .unwrap_or_else(|| text(document, &entry.key, 1024))?;
            let description = specification
                .get(b"Desc")
                .ok()
                .map(|value| text(document, value, 4096))
                .transpose()?
                .unwrap_or_default();
            Ok(AttachmentInfo {
                id: index.to_string(),
                suggested_name: suggested_basename(&name),
                name,
                description,
                byte_length: bytes.len(),
            })
        })
        .collect()
}

pub(crate) fn export(document: &Document, id: &str) -> Result<(String, Vec<u8>)> {
    let infos = inspect(document)?;
    let index = id
        .parse::<usize>()
        .map_err(|_| invalid("The attachment identifier is invalid."))?;
    let info = infos
        .get(index)
        .ok_or_else(|| invalid("That attachment no longer exists."))?;
    let tree = Tree::read(document)?;
    let (_, stream) = data(document, &tree.entries[index])?;
    Ok((
        info.suggested_name.clone(),
        stream_bytes(stream, MAX_ATTACHMENT_BYTES)?,
    ))
}

pub fn read_picked_file(path: &Path) -> Result<(String, Vec<u8>)> {
    let mut file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_ATTACHMENT_BYTES as u64 {
        return Err(invalid("Choose an attachment file no larger than 16 MiB."));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_ATTACHMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(invalid("The attachment exceeds 16 MiB."));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("The attachment filename is not Unicode."))?
        .to_owned();
    Ok((name, bytes))
}

pub(crate) fn add(document: &mut Document, name: &str, bytes: Vec<u8>) -> Result<()> {
    validate_text(name, 1024, false)?;
    if bytes.len() > MAX_ATTACHMENT_BYTES {
        return Err(invalid("The attachment exceeds 16 MiB."));
    }
    if document.catalog()?.has(b"Collection") {
        return Err(invalid(
            "PDF portfolios require a dedicated portfolio editor; attachment changes are not supported.",
        ));
    }
    let infos = inspect(document)?;
    if infos.len() >= MAX_ATTACHMENTS {
        return Err(invalid("This PDF already has 128 attachments."));
    }
    if infos.iter().map(|info| info.byte_length).sum::<usize>() + bytes.len()
        > MAX_TOTAL_ATTACHMENT_BYTES
    {
        return Err(invalid("Attachments must total no more than 64 MiB."));
    }
    let mut tree = Tree::read(document)?;
    if infos.iter().any(|info| info.name == name)
        || tree
            .entries
            .iter()
            .any(|entry| text(document, &entry.key, 1024).is_ok_and(|key| key == name))
    {
        return Err(invalid(
            "An attachment already uses this name. Rename the local file before adding it.",
        ));
    }
    let length = bytes.len();
    let stream = document.add_object(Stream::new(
        dictionary! { "Type" => "EmbeddedFile", "Params" => dictionary! { "Size" => length as i64 } }, bytes,
    ));
    let specification = document.add_object(dictionary! {
        "Type" => "Filespec", "F" => text_string(name), "UF" => text_string(name),
        "EF" => dictionary! { "F" => stream, "UF" => stream },
    });
    tree.entries.push(Entry {
        key: text_string(name),
        specification: specification.into(),
    });
    tree.write(document)?;
    inspect(document)?;
    Ok(())
}

pub(crate) fn remove(document: &mut Document, id: &str) -> Result<()> {
    if document.catalog()?.has(b"Collection") {
        return Err(invalid(
            "Removing attachments from a PDF portfolio is not supported.",
        ));
    }
    inspect(document)?;
    let index = id
        .parse::<usize>()
        .map_err(|_| invalid("The attachment identifier is invalid."))?;
    let mut tree = Tree::read(document)?;
    if index >= tree.entries.len() {
        return Err(invalid("That attachment no longer exists."));
    }
    tree.entries.remove(index);
    tree.write(document)
}

pub fn suggested_basename(value: &str) -> String {
    let leaf = value.rsplit(['/', '\\']).next().unwrap_or_default();
    let mut result = String::new();
    for character in leaf.chars().take(120) {
        let character = if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            ) {
            '_'
        } else {
            character
        };
        if result.len() + character.len_utf8() > 240 {
            break;
        }
        result.push(character);
    }
    result = result.trim_matches([' ', '.']).to_owned();
    let stem = result
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    ) {
        result.insert(0, '_');
    }
    if result.is_empty() {
        "attachment.bin".into()
    } else {
        result
    }
}
