use std::collections::HashSet;

use lopdf::{Document, Object, ObjectId, Stream, decode_text_string};

use crate::document::{Result, invalid};

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
