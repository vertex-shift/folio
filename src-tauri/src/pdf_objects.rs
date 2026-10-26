use std::collections::HashSet;

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream,
    content::{Content, Operation},
    decode_text_string, dictionary,
};

use crate::document::{Result, inherited, invalid, page_id};

pub(crate) const MAX_EDIT_CONTENT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn resolve<'a>(document: &'a Document, value: &'a Object) -> Result<&'a Object> {
    let mut value = value;
    let mut visited = HashSet::new();
    while let Object::Reference(id) = value {
        if !visited.insert(*id) || visited.len() > 32 {
            return Err(invalid(
                "The PDF has a cyclic or excessively deep object reference.",
            ));
        }
        value = document.get_object(*id)?;
    }
    Ok(value)
}

pub(crate) fn text(document: &Document, value: &Object, limit: usize) -> Result<String> {
    let value = resolve(document, value)?;
    if value.as_str()?.len() > limit * 2 + 2 {
        return Err(invalid("A PDF text property exceeds its supported size."));
    }
    let value = decode_text_string(value)?;
    if value.len() > limit {
        return Err(invalid("A PDF text property exceeds its supported size."));
    }
    Ok(value)
}

pub(crate) fn key(id: ObjectId) -> String {
    format!("{}:{}", id.0, id.1)
}

pub(crate) fn parse_key(value: &str) -> Result<ObjectId> {
    let (number, generation) = value
        .split_once(':')
        .ok_or_else(|| invalid("The PDF object identifier is invalid."))?;
    Ok((
        number
            .parse()
            .map_err(|_| invalid("The PDF object identifier is invalid."))?,
        generation
            .parse()
            .map_err(|_| invalid("The PDF object identifier is invalid."))?,
    ))
}

pub(crate) fn validate_text(value: &str, limit: usize, allow_empty: bool) -> Result<()> {
    if value.len() > limit
        || (!allow_empty && value.trim().is_empty())
        || value.chars().any(|character| {
            character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                || matches!(character, '\u{fffe}' | '\u{ffff}')
        })
    {
        return Err(invalid(format!(
            "Enter {}text without control characters, within {limit} UTF-8 bytes.",
            if allow_empty { "" } else { "nonempty " }
        )));
    }
    Ok(())
}

pub(crate) fn stream_bytes(stream: &Stream, limit: usize) -> Result<Vec<u8>> {
    if stream.content.len() > limit {
        return Err(invalid(
            "An encoded metadata or attachment stream exceeds its size limit.",
        ));
    }
    if stream.dict.has(b"Filter") {
        let filters = stream.filters()?;
        if filters.len() > 8 {
            return Err(invalid(
                "A metadata or attachment stream exceeds eight decoding filters.",
            ));
        }
        if let Ok(parameters) = stream.dict.get(b"DecodeParms") {
            let parameters: Vec<_> = match parameters {
                Object::Array(values) if values.len() == filters.len() => values.iter().collect(),
                Object::Dictionary(_) | Object::Null if filters.len() == 1 => vec![parameters],
                _ => {
                    return Err(invalid(
                        "Indirect or malformed stream decoding parameters are not supported.",
                    ));
                }
            };
            for parameters in parameters {
                if matches!(parameters, Object::Null) {
                    continue;
                }
                let dictionary = parameters.as_dict()?;
                for key in [
                    b"Predictor".as_slice(),
                    b"Colors",
                    b"BitsPerComponent",
                    b"Columns",
                    b"EarlyChange",
                ] {
                    if dictionary
                        .get(key)
                        .is_ok_and(|value| matches!(value, Object::Reference(_)))
                    {
                        return Err(invalid(
                            "Indirect stream predictor parameters are not supported.",
                        ));
                    }
                }
            }
        }
    }
    Ok(stream.get_plain_content_with_limit(limit)?)
}

pub(crate) fn page_resources(document: &Document, id: ObjectId) -> Result<Dictionary> {
    match inherited(document, id, b"Resources")? {
        Some(value) => Ok(resolve(document, &value)?.as_dict()?.clone()),
        None => Ok(Dictionary::new()),
    }
}

pub(crate) fn form_resources(
    document: &Document,
    stream: &Stream,
    parent: &Dictionary,
) -> Result<Dictionary> {
    match stream.dict.get(b"Resources") {
        Ok(value) => Ok(resolve(document, value)?.as_dict()?.clone()),
        Err(lopdf::Error::DictKey(_)) => Ok(parent.clone()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn resource<'a>(
    document: &'a Document,
    resources: &'a Dictionary,
    kind: &[u8],
    name: &[u8],
) -> Result<&'a Object> {
    Ok(resolve(document, resources.get(kind)?)?
        .as_dict()?
        .get(name)?)
}

pub(crate) fn install_resource(
    document: &Document,
    resources: &mut Dictionary,
    kind: &[u8],
    prefix: &str,
    value: Object,
) -> Result<Vec<u8>> {
    let mut entries = match resources.get(kind) {
        Ok(value) => resolve(document, value)?.as_dict()?.clone(),
        Err(lopdf::Error::DictKey(_)) => Dictionary::new(),
        Err(error) => return Err(error.into()),
    };
    for index in 1..=200_000 {
        let name = format!("{prefix}{index}").into_bytes();
        if !entries.has(&name) {
            entries.set(name.clone(), value);
            resources.set(kind, entries);
            return Ok(name);
        }
    }
    Err(invalid("Too many resource names in this PDF."))
}

pub(crate) fn rewrite_operation(
    document: &mut Document,
    page: usize,
    path: &[usize],
    mut change: impl FnMut(&mut Document, &mut Dictionary, &Operation) -> Result<Vec<Operation>>,
) -> Result<()> {
    if path.is_empty() || path.len() > 17 {
        return Err(invalid(
            "The source operation path is invalid or too deeply nested.",
        ));
    }
    let id = page_id(document, page)?;
    if document
        .get_pages()
        .values()
        .filter(|value| **value == id)
        .count()
        != 1
    {
        return Err(invalid(
            "This PDF reuses a page object. It cannot be edited independently.",
        ));
    }
    let bytes = document.get_page_content_with_limit(id, MAX_EDIT_CONTENT_BYTES)?;
    let resources = page_resources(document, id)?;
    let (bytes, resources) = rewrite_container(document, bytes, resources, path, &mut change)?;
    let mut stream = Stream::new(dictionary! {}, bytes);
    stream.compress()?;
    let contents = document.add_object(stream);
    let page = document.get_dictionary_mut(id)?;
    // Copy only the invocation chain; shared streams and resources remain untouched.
    page.set("Contents", contents);
    page.set("Resources", resources);
    Ok(())
}

fn rewrite_container<F>(
    document: &mut Document,
    bytes: Vec<u8>,
    mut resources: Dictionary,
    path: &[usize],
    change: &mut F,
) -> Result<(Vec<u8>, Dictionary)>
where
    F: FnMut(&mut Document, &mut Dictionary, &Operation) -> Result<Vec<Operation>>,
{
    let mut content = Content::decode(&bytes)?;
    if content
        .operations
        .iter()
        .any(|op| matches!(op.operator.as_str(), "BI" | "ID" | "EI"))
    {
        return Err(invalid(
            "Content streams containing inline images cannot be rewritten safely.",
        ));
    }
    let index = path[0];
    let operation = content
        .operations
        .get(index)
        .ok_or_else(|| invalid("The selected content operation no longer exists."))?
        .clone();
    if path.len() == 1 {
        let replacement = change(document, &mut resources, &operation)?;
        content.operations.splice(index..=index, replacement);
    } else {
        if operation.operator != "Do" || operation.operands.len() != 1 {
            return Err(invalid("The selected Form invocation path changed."));
        }
        let name = operation.operands[0].as_name()?;
        let mut stream = resolve(document, resource(document, &resources, b"XObject", name)?)?
            .as_stream()?
            .clone();
        if stream.dict.get(b"Subtype")?.as_name()? != b"Form" {
            return Err(invalid("The selected Form invocation is no longer a Form."));
        }
        let child_resources = form_resources(document, &stream, &resources)?;
        let (bytes, child_resources) = rewrite_container(
            document,
            stream.get_plain_content_with_limit(MAX_EDIT_CONTENT_BYTES)?,
            child_resources,
            &path[1..],
            change,
        )?;
        stream.set_plain_content(bytes);
        stream.dict.set("Resources", child_resources);
        stream.compress()?;
        let id = document.add_object(stream);
        let name = install_resource(
            document,
            &mut resources,
            b"XObject",
            "FolioEditedForm",
            id.into(),
        )?;
        content.operations[index].operands[0] = Object::Name(name);
    }
    Ok((content.encode()?, resources))
}
