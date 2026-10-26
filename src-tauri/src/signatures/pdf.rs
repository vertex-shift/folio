use std::{collections::BTreeMap, ops::Range};

use lopdf::{Document, ObjectId, xref::XrefEntry};

use super::{CheckError, CheckResult, MAX_CMS_BYTES};

const MAX_SIGNATURE_OBJECT_BYTES: usize = 2 * MAX_CMS_BYTES + 64 * 1024;

#[derive(Debug)]
pub(super) struct Value {
    pub span: Range<usize>,
    pub kind: Kind,
}

#[derive(Debug)]
pub(super) enum Kind {
    Integer(i64),
    Name(Vec<u8>),
    Hex(Vec<u8>),
    Array(Vec<Value>),
    Dictionary(BTreeMap<Vec<u8>, Value>),
    Other,
}

pub(super) struct SignatureObject {
    pub offset: usize,
    pub end: usize,
    pub entries: BTreeMap<Vec<u8>, Value>,
}

impl SignatureObject {
    pub fn get(&self, name: &[u8]) -> CheckResult<&Value> {
        self.entries.get(name).ok_or_else(|| {
            CheckError::invalid("The signature dictionary is missing a required entry.")
        })
    }

    pub fn name(&self, name: &[u8]) -> CheckResult<&[u8]> {
        match &self.get(name)?.kind {
            Kind::Name(value) => Ok(value),
            _ => Err(CheckError::invalid(
                "A signature name must be a direct PDF name.",
            )),
        }
    }

    pub fn byte_range(&self, length: usize) -> CheckResult<[usize; 4]> {
        let Kind::Array(values) = &self.get(b"ByteRange")?.kind else {
            return Err(CheckError::invalid("ByteRange must be a direct array."));
        };
        if values.len() != 4 {
            return Err(CheckError::invalid(
                "ByteRange must contain exactly two byte spans; multiple gaps are not supported.",
            ));
        }
        let mut ranges = [0; 4];
        for (target, value) in ranges.iter_mut().zip(values) {
            let Kind::Integer(value) = value.kind else {
                return Err(CheckError::invalid(
                    "ByteRange values must be direct integers.",
                ));
            };
            *target = usize::try_from(value).map_err(|_| {
                CheckError::invalid("ByteRange contains a negative or oversized offset.")
            })?;
        }
        let [start, first_length, second_start, second_length] = ranges;
        let end = second_start
            .checked_add(second_length)
            .ok_or_else(|| CheckError::invalid("ByteRange overflows the document bounds."))?;
        if start != 0
            || first_length == 0
            || second_start <= first_length
            || second_length == 0
            || end > length
            || self.end > end
        {
            return Err(CheckError::invalid(
                "ByteRange is overlapping, incomplete, or outside the document.",
            ));
        }
        let contents = self.get(b"Contents")?;
        if !matches!(contents.kind, Kind::Hex(_)) || contents.span != (first_length..second_start) {
            return Err(CheckError::invalid(
                "The only unsigned gap must be exactly this signature's hexadecimal Contents string, including its delimiters.",
            ));
        }
        Ok(ranges)
    }
}

pub(super) fn signature_object(
    bytes: &[u8],
    document: &Document,
    id: ObjectId,
) -> CheckResult<SignatureObject> {
    if document.xref_recovery.is_some() {
        return Err(CheckError::unsupported(
            "The PDF required cross-reference repair. Signature verification is not safe on a repaired structure.",
        ));
    }
    let offset = match document.reference_table.entries.get(&id.0) {
        Some(XrefEntry::Normal { offset, generation }) if *generation == id.1 => *offset as usize,
        Some(XrefEntry::Compressed { .. }) => {
            return Err(CheckError::unsupported(
                "Signatures inside compressed object streams are not supported.",
            ));
        }
        _ => {
            return Err(CheckError::invalid(
                "The signature has no matching cross-reference entry.",
            ));
        }
    };
    let limit = bytes
        .len()
        .min(offset.saturating_add(MAX_SIGNATURE_OBJECT_BYTES));
    if offset >= limit {
        return Err(CheckError::invalid(
            "The signature object offset is outside the document.",
        ));
    }
    let mut reader = Reader {
        bytes: &bytes[..limit],
        position: offset,
        remaining: 16_384,
    };
    if reader.word()? != id.0.to_string().as_bytes()
        || reader.word()? != id.1.to_string().as_bytes()
        || reader.word()? != b"obj"
    {
        return Err(CheckError::invalid(
            "The signature object does not match its cross-reference entry.",
        ));
    }
    let value = reader.value(0)?;
    let Kind::Dictionary(entries) = value.kind else {
        return Err(CheckError::invalid(
            "The signature object must be a dictionary.",
        ));
    };
    if reader.word()? != b"endobj" {
        return Err(CheckError::invalid(
            "The signature dictionary has an invalid object boundary.",
        ));
    }
    Ok(SignatureObject {
        offset,
        end: reader.position,
        entries,
    })
}

pub(super) fn validate_footer(bytes: &[u8], document: &Document) -> CheckResult<()> {
    let trimmed = bytes.trim_ascii_end();
    let Some(before_eof) = trimmed.strip_suffix(b"%%EOF") else {
        return Err(CheckError::invalid(
            "The signed byte spans do not end at a complete PDF revision.",
        ));
    };
    let start = before_eof.len().saturating_sub(1024);
    let tail = &before_eof[start..];
    let Some(index) = tail.windows(9).rposition(|value| value == b"startxref") else {
        return Err(CheckError::invalid(
            "The signed revision has no final startxref marker.",
        ));
    };
    let offset = std::str::from_utf8(tail[index + 9..].trim_ascii())
        .ok()
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<usize>().ok());
    if offset != Some(document.xref_start)
        || document.xref_start == 0
        || document.xref_recovery.is_some()
    {
        return Err(CheckError::invalid(
            "The signed revision's final cross-reference offset is invalid.",
        ));
    }
    Ok(())
}

pub(super) fn revision_prev(bytes: &[u8], document: &Document) -> CheckResult<Option<usize>> {
    let mut reader = Reader {
        bytes,
        position: document.xref_start,
        remaining: 16_384,
    };
    if reader.word()? == b"xref" {
        let mut entries = 0usize;
        loop {
            let word = reader.word()?;
            if word == b"trailer" {
                break;
            }
            let number = |value: &[u8]| {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .ok_or_else(|| {
                        CheckError::invalid("The appended cross-reference section is malformed.")
                    })
            };
            number(word)?;
            let count = number(reader.word()?)?;
            entries = entries
                .checked_add(count)
                .filter(|count| *count <= 4096)
                .ok_or_else(|| {
                    CheckError::invalid("The appended cross-reference section exceeds its limit.")
                })?;
            for _ in 0..count {
                number(reader.word()?)?;
                number(reader.word()?)?;
                if !matches!(reader.word()?, b"n" | b"f") {
                    return Err(CheckError::invalid(
                        "An appended cross-reference entry is malformed.",
                    ));
                }
            }
        }
    } else {
        reader.word()?;
        if reader.word()? != b"obj" {
            return Err(CheckError::invalid(
                "The appended cross-reference stream is malformed.",
            ));
        }
    }
    let Kind::Dictionary(entries) = reader.value(0)?.kind else {
        return Err(CheckError::invalid(
            "The appended revision has no trailer dictionary.",
        ));
    };
    match entries.get(b"Prev".as_slice()) {
        None => Ok(None),
        Some(Value {
            kind: Kind::Integer(value),
            ..
        }) => usize::try_from(*value)
            .map(Some)
            .map_err(|_| CheckError::invalid("The appended Prev offset is negative or too large.")),
        _ => Err(CheckError::invalid(
            "The appended Prev offset is not a direct integer.",
        )),
    }
}

fn whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

fn delimiter(byte: u8) -> bool {
    whitespace(byte) || b"()<>[]{}/%".contains(&byte)
}

fn nibble(byte: u8) -> CheckResult<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(CheckError::invalid(
            "A PDF hexadecimal string or escaped name is malformed.",
        )),
    }
}

fn real_number(word: &[u8]) -> bool {
    let unsigned = word
        .strip_prefix(b"+")
        .or_else(|| word.strip_prefix(b"-"))
        .unwrap_or(word);
    let mut digits = 0;
    let mut points = 0;
    for byte in unsigned {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' => points += 1,
            _ => return false,
        }
    }
    digits > 0 && points <= 1
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    remaining: usize,
}

impl<'a> Reader<'a> {
    fn skip_space(&mut self) {
        while let Some(byte) = self.bytes.get(self.position) {
            if whitespace(*byte) {
                self.position += 1;
            } else if *byte == b'%' {
                while self
                    .bytes
                    .get(self.position)
                    .is_some_and(|byte| !b"\r\n".contains(byte))
                {
                    self.position += 1;
                }
            } else {
                break;
            }
        }
    }

    fn word(&mut self) -> CheckResult<&'a [u8]> {
        self.skip_space();
        let start = self.position;
        while self
            .bytes
            .get(self.position)
            .is_some_and(|byte| !delimiter(*byte))
        {
            self.position += 1;
        }
        if self.position == start {
            return Err(CheckError::invalid(
                "The signature's PDF syntax is malformed or truncated.",
            ));
        }
        Ok(&self.bytes[start..self.position])
    }

    fn name(&mut self) -> CheckResult<Vec<u8>> {
        self.skip_space();
        if self.bytes.get(self.position) != Some(&b'/') {
            return Err(CheckError::invalid(
                "A signature dictionary key is not a PDF name.",
            ));
        }
        self.position += 1;
        let mut output = Vec::new();
        while let Some(&byte) = self.bytes.get(self.position) {
            if delimiter(byte) {
                break;
            }
            if byte == b'#' {
                let pair = self
                    .bytes
                    .get(self.position + 1..self.position + 3)
                    .ok_or_else(|| CheckError::invalid("A PDF name escape is truncated."))?;
                output.push(nibble(pair[0])? * 16 + nibble(pair[1])?);
                self.position += 3;
            } else {
                output.push(byte);
                self.position += 1;
            }
        }
        Ok(output)
    }

    fn value(&mut self, depth: usize) -> CheckResult<Value> {
        if depth > 32 || self.remaining == 0 {
            return Err(CheckError::unsupported(
                "The signature dictionary is excessively complex.",
            ));
        }
        self.remaining -= 1;
        self.skip_space();
        let start = self.position;
        let kind = match self.bytes.get(self.position) {
            Some(b'/') => Kind::Name(self.name()?),
            Some(b'<') if self.bytes.get(self.position + 1) == Some(&b'<') => {
                self.position += 2;
                let mut entries = BTreeMap::new();
                loop {
                    self.skip_space();
                    if self.bytes.get(self.position..self.position + 2) == Some(b">>") {
                        self.position += 2;
                        break;
                    }
                    let name = self.name()?;
                    let value = self.value(depth + 1)?;
                    if entries.insert(name, value).is_some() {
                        return Err(CheckError::invalid(
                            "Duplicate PDF dictionary keys make this signature ambiguous.",
                        ));
                    }
                }
                Kind::Dictionary(entries)
            }
            Some(b'<') => {
                self.position += 1;
                let mut output = Vec::new();
                let mut high = None;
                loop {
                    let byte = *self
                        .bytes
                        .get(self.position)
                        .ok_or_else(|| CheckError::invalid("Signature Contents is truncated."))?;
                    self.position += 1;
                    if byte == b'>' {
                        if high.is_some() {
                            return Err(CheckError::invalid(
                                "Signature Contents must have complete hexadecimal bytes.",
                            ));
                        }
                        break;
                    }
                    if whitespace(byte) {
                        continue;
                    }
                    let value = nibble(byte)?;
                    if let Some(first) = high.take() {
                        output.push(first * 16 + value);
                        if output.len() > MAX_CMS_BYTES {
                            return Err(CheckError::unsupported(
                                "The signature exceeds the 256 KiB CMS limit.",
                            ));
                        }
                    } else {
                        high = Some(value);
                    }
                }
                Kind::Hex(output)
            }
            Some(b'[') => {
                self.position += 1;
                let mut values = Vec::new();
                loop {
                    self.skip_space();
                    if self.bytes.get(self.position) == Some(&b']') {
                        self.position += 1;
                        break;
                    }
                    values.push(self.value(depth + 1)?);
                }
                Kind::Array(values)
            }
            Some(b'(') => {
                self.position += 1;
                let mut nesting = 1;
                while nesting > 0 {
                    match self.bytes.get(self.position) {
                        Some(b'\\') => {
                            self.position += 1;
                            if self.bytes.get(self.position).is_none() {
                                return Err(CheckError::invalid(
                                    "A PDF string escape is truncated.",
                                ));
                            }
                        }
                        Some(b'(') => nesting += 1,
                        Some(b')') => nesting -= 1,
                        None => {
                            return Err(CheckError::invalid(
                                "A PDF signature string is truncated.",
                            ));
                        }
                        _ => {}
                    }
                    self.position += 1;
                }
                Kind::Other
            }
            _ => {
                let word = self.word()?;
                let integer = std::str::from_utf8(word)
                    .ok()
                    .and_then(|value| value.parse::<i64>().ok());
                if let Some(number) = integer {
                    let after_number = self.position;
                    let reference = self.word().ok().is_some_and(|value| {
                        !value.is_empty() && value.iter().all(u8::is_ascii_digit)
                    }) && self.word().ok() == Some(b"R");
                    if reference {
                        Kind::Other
                    } else {
                        self.position = after_number;
                        Kind::Integer(number)
                    }
                } else if matches!(word, b"true" | b"false" | b"null") || real_number(word) {
                    Kind::Other
                } else {
                    return Err(CheckError::invalid(
                        "A signature dictionary value has invalid PDF syntax.",
                    ));
                }
            }
        };
        Ok(Value {
            span: start..self.position,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> CheckResult<Value> {
        Reader {
            bytes,
            position: 0,
            remaining: 1024,
        }
        .value(0)
    }

    #[test]
    fn duplicate_and_escaped_duplicate_dictionary_keys_are_rejected() {
        for bytes in [
            b"<< /Contents <00> /Contents <01> >>".as_slice(),
            b"<< /ByteRange [0 1 2 3] /Byte#52ange [0 1 2 3] >>",
            b"<< /Type /Sig /Reference << /X 1 /X 2 >> >>",
        ] {
            assert!(parse(bytes).unwrap_err().message.contains("Duplicate"));
        }
    }

    #[test]
    fn misleading_markers_in_literal_strings_do_not_become_dictionary_entries() {
        let value = parse(
            b"<< /Reason (contains \\( /Contents <00> /ByteRange [0 1 2 3] \\)) /Type /Sig >>",
        )
        .unwrap();
        let Kind::Dictionary(entries) = value.kind else {
            panic!("dictionary expected");
        };
        assert_eq!(entries.len(), 2);
        assert!(entries.contains_key(b"Type".as_slice()));
        assert!(!entries.contains_key(b"ByteRange".as_slice()));
    }

    #[test]
    fn malformed_tokens_truncation_and_non_pdf_numeric_forms_are_rejected() {
        for bytes in [
            b"<< /Contents <ABC> >>".as_slice(),
            b"<< /Contents <GG> >>",
            b"<< /Contents (truncated >>",
            b"<< /ByteRange [0 1 2 3",
            b"<< /Type /Si#",
            b"<< /ByteRange [0 NaN 2 3] >>",
            b"<< /ByteRange [0 1e2 2 3] >>",
        ] {
            assert!(parse(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn references_are_not_accepted_as_byte_range_integers() {
        let value = parse(b"<< /ByteRange [0 9 0 R 20 30] /Contents <00> >>").unwrap();
        let Kind::Dictionary(entries) = value.kind else {
            panic!("dictionary expected");
        };
        let signature = SignatureObject {
            offset: 0,
            end: 10,
            entries,
        };
        assert!(
            signature
                .byte_range(100)
                .unwrap_err()
                .message
                .contains("direct integers")
        );
    }
}
