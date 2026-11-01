use std::{borrow::Cow, collections::BTreeMap};

use lopdf::{
    Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};

use super::{
    Direction,
    catalog::{self, FontId},
    font,
};
use crate::{
    document::{Result, invalid, page_id},
    pdf_objects::{MAX_EDIT_CONTENT_BYTES, page_resources, resolve, resource, stream_bytes},
};

const MAX_FONTS: usize = 32;
const MAX_FONT_BYTES: usize = 32 * 1024 * 1024;
const MAX_LOGICAL_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct CanonicalText {
    pub stream: ObjectId,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Default)]
pub(crate) struct Reader {
    fonts: BTreeMap<ObjectId, BTreeMap<u16, String>>,
    font_bytes: usize,
}

fn number(value: &Object) -> Result<f64> {
    let value = match value {
        Object::Integer(value) => *value as f64,
        Object::Real(value) => f64::from(*value),
        _ => return Err(invalid("Canonical OCR has a nonnumeric drawing operand.")),
    };
    if !value.is_finite() || value.abs() > 1e9 {
        return Err(invalid("Canonical OCR has unstable drawing coordinates."));
    }
    Ok(value)
}

fn same_object(
    actual_doc: &Document,
    actual: &Object,
    expected_doc: &Document,
    expected: &Object,
    depth: usize,
) -> Result<bool> {
    if depth > 16 {
        return Err(invalid(
            "Canonical OCR font references exceed the supported depth.",
        ));
    }
    let actual = resolve(actual_doc, actual)?;
    let expected = resolve(expected_doc, expected)?;
    match (actual, expected) {
        (Object::Integer(_) | Object::Real(_), Object::Integer(_) | Object::Real(_)) => {
            Ok((number(actual)? - number(expected)?).abs() <= 0.0001)
        }
        (Object::Array(a), Object::Array(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (a, b) in a.iter().zip(b) {
                if !same_object(actual_doc, a, expected_doc, b, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Object::Dictionary(a), Object::Dictionary(b)) => {
            if a.len() != b.len() {
                return Ok(false);
            }
            for (name, expected) in b {
                let Ok(actual) = a.get(name) else {
                    return Ok(false);
                };
                if !same_object(actual_doc, actual, expected_doc, expected, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Object::Stream(a), Object::Stream(b)) => {
            let mut a_dictionary = a.dict.clone();
            let mut b_dictionary = b.dict.clone();
            for name in [b"Length".as_slice(), b"Filter", b"DecodeParms"] {
                a_dictionary.remove(name);
                b_dictionary.remove(name);
            }
            if !same_object(
                actual_doc,
                &Object::Dictionary(a_dictionary),
                expected_doc,
                &Object::Dictionary(b_dictionary),
                depth + 1,
            )? {
                return Ok(false);
            }
            Ok(stream_bytes(a, MAX_FONT_BYTES)? == stream_bytes(b, MAX_FONT_BYTES)?)
        }
        (Object::String(a, _), Object::String(b, _)) => Ok(a == b),
        _ => Ok(actual == expected),
    }
}

fn is_separator(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|value| matches!(value, b' ' | b'\t' | b'\r' | b'\n'))
}

fn unicode_map(document: &Document, font: &lopdf::Dictionary) -> Result<BTreeMap<u16, String>> {
    let stream = resolve(document, font.get(b"ToUnicode")?)?.as_stream()?;
    let bytes = stream_bytes(stream, MAX_LOGICAL_BYTES)?;
    let content = std::str::from_utf8(&bytes)
        .map_err(|_| invalid("Canonical OCR ToUnicode is not its generated ASCII CMap."))?;
    let mut result = BTreeMap::new();
    for line in content.lines() {
        let values = line.split_ascii_whitespace().collect::<Vec<_>>();
        if values.len() != 2 || !values[0].starts_with('<') || !values[1].starts_with('<') {
            continue;
        }
        let code = values[0]
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .filter(|value| value.len() == 4)
            .ok_or_else(|| invalid("Canonical OCR has a malformed two-byte CMap source code."))?;
        let code = u16::from_str_radix(code, 16)
            .map_err(|_| invalid("Canonical OCR has an invalid CMap source code."))?;
        if code == 0 {
            continue;
        }
        let value = values[1]
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .filter(|value| {
                !value.is_empty() && value.len() <= 2048 && value.len().is_multiple_of(4)
            })
            .ok_or_else(|| invalid("Canonical OCR has malformed complete-word UTF-16 data."))?;
        let units = value
            .as_bytes()
            .chunks_exact(4)
            .map(|bytes| {
                let value = std::str::from_utf8(bytes)
                    .map_err(|_| invalid("Invalid OCR CMap hex data."))?;
                u16::from_str_radix(value, 16).map_err(|_| invalid("Invalid OCR CMap UTF-16 unit."))
            })
            .collect::<Result<Vec<_>>>()?;
        let text = String::from_utf16(&units)
            .map_err(|_| invalid("Canonical OCR CMap has invalid Unicode surrogates."))?;
        if result.insert(code, text).is_some() || result.len() > 60_000 {
            return Err(invalid(
                "Canonical OCR CMap has duplicate codes or exceeds its source encoding limit.",
            ));
        }
    }
    // The complete raw CMap is compared with the generator below; this parser never
    // executes CMap code or accepts an additional mapping/layout dialect.
    Ok(result)
}

impl Reader {
    fn font(&mut self, document: &Document, object: &Object) -> Result<&BTreeMap<u16, String>> {
        let object_id = object
            .as_reference()
            .map_err(|_| invalid("Canonical OCR requires an indirect, verified font."))?;
        if !self.fonts.contains_key(&object_id) {
            if self.fonts.len() >= MAX_FONTS {
                return Err(invalid(
                    "Canonical OCR inspection exceeds 32 distinct fonts.",
                ));
            }
            let dictionary = resolve(document, object)?.as_dict()?;
            let name = dictionary.get(b"BaseFont")?.as_name()?;
            let mut identity = None;
            for id in [
                FontId::Latin,
                FontId::Arabic,
                FontId::Devanagari,
                FontId::CjkSc,
                FontId::CjkJp,
                FontId::CjkKr,
            ] {
                for direction in [Direction::Ltr, Direction::Rtl] {
                    if name
                        == format!("FolioOCRLogicalV1_{}_{}", id.key(), direction.key()).as_bytes()
                    {
                        identity = Some((id, direction));
                    }
                }
            }
            let (id, direction) = identity.ok_or_else(|| {
                invalid("The OCR font is not the supported canonical logical-word schema.")
            })?;
            let pinned = catalog::catalog()?
                .fonts
                .iter()
                .find(|entry| entry.id == id)
                .ok_or_else(|| invalid("The OCR font is absent from the local pinned catalog."))?;
            let digest: String = openssl::sha::sha256(id.bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if id.bytes().len() != pinned.bytes || digest != pinned.sha256 {
                return Err(invalid(
                    "The local OCR font failed its byte-size/SHA-256 pin. No semantic source is accepted.",
                ));
            }
            self.font_bytes += id.bytes().len();
            if self.font_bytes > MAX_FONT_BYTES {
                return Err(invalid(
                    "Canonical OCR inspection exceeds 32 MiB of font programs.",
                ));
            }
            let face = ttf_parser::Face::parse(id.bytes(), 0)
                .map_err(|_| invalid("The pinned canonical OCR font cannot be parsed."))?;
            if face.is_variable()
                || face.tables().glyf.is_none()
                || face.tables().cff.is_some()
                || face.tables().cff2.is_some()
            {
                return Err(invalid(
                    "Canonical OCR requires the pinned static TrueType program.",
                ));
            }
            let descendants = resolve(document, dictionary.get(b"DescendantFonts")?)?.as_array()?;
            if descendants.len() != 1 {
                return Err(invalid("Canonical OCR has an ambiguous descendant font."));
            }
            let descendant = resolve(document, &descendants[0])?.as_dict()?;
            let map = stream_bytes(
                resolve(document, descendant.get(b"CIDToGIDMap")?)?.as_stream()?,
                120_002,
            )?;
            if map.len() < 4 || map.len() % 2 != 0 || map[..2] != [0, 0] {
                return Err(invalid("Canonical OCR has a malformed CID-to-glyph map."));
            }
            let mapping = unicode_map(document, dictionary)?;
            if mapping.len() != map.len() / 2 - 1 {
                return Err(invalid(
                    "Canonical OCR glyph and Unicode map ranges disagree.",
                ));
            }
            let mut codes = BTreeMap::new();
            let mut entries = BTreeMap::new();
            for cid in 1..map.len() / 2 {
                let cid = u16::try_from(cid)
                    .map_err(|_| invalid("Canonical OCR exceeds its CID range."))?;
                let value = mapping
                    .get(&cid)
                    .cloned()
                    .ok_or_else(|| invalid("Canonical OCR has incomplete Unicode mapping."))?;
                let separator = is_separator(&value);
                if !separator
                    && (value.is_empty()
                        || value.chars().count() > 256
                        || value.chars().any(|character| {
                            character.is_whitespace()
                                || character.is_control()
                                || matches!(character as u32, 0x202a..=0x202e | 0x2066..=0x2069)
                                || !matches!(character, '\u{200c}' | '\u{200d}')
                                    && face.glyph_index(character).is_none_or(|glyph| glyph.0 == 0)
                        }))
                {
                    return Err(invalid(
                        "Canonical OCR contains an invalid logical word or unsupported source glyph.",
                    ));
                }
                if entries.insert(value.clone(), separator).is_some() {
                    return Err(invalid("Canonical OCR repeats a logical CID mapping."));
                }
                codes.insert(cid, value);
            }
            if entries.values().all(|separator| *separator) {
                return Err(invalid("Canonical OCR has no actual word glyphs."));
            }
            let mut expected = Document::with_version("1.7");
            let embedded = font::embed_entries(&mut expected, id, direction, entries)?;
            if !same_object(
                document,
                object,
                &expected,
                &Object::Reference(embedded.object),
                0,
            )? {
                return Err(invalid(
                    "The complete OCR font, Unicode map, glyph map, widths or descriptor does not match its verified canonical logical text. A marker or font name is not authority.",
                ));
            }
            self.fonts.insert(object_id, codes);
        }
        Ok(self
            .fonts
            .get(&object_id)
            .expect("validated canonical font"))
    }

    pub(crate) fn page(&mut self, document: &Document, page: usize) -> Result<Vec<CanonicalText>> {
        let id = page_id(document, page)?;
        let dictionary = document.get_dictionary(id)?;
        let values: Vec<_> = match dictionary.get(b"Contents") {
            Ok(value) => match resolve(document, value)? {
                Object::Array(values) => values.iter().collect(),
                Object::Stream(_) => vec![value],
                Object::Null => vec![],
                _ => {
                    return Err(invalid(
                        "The semantic page has unsupported content streams.",
                    ));
                }
            },
            Err(lopdf::Error::DictKey(_)) => vec![],
            Err(error) => return Err(error.into()),
        };
        let resources = page_resources(document, id)?;
        let mut result = Vec::new();
        let mut offset = 0;
        let mut bytes = 0;
        for value in values {
            let stream = resolve(document, value)?.as_stream()?;
            let body = stream_bytes(stream, MAX_EDIT_CONTENT_BYTES)?;
            bytes += body.len();
            if bytes > MAX_EDIT_CONTENT_BYTES {
                return Err(invalid(
                    "The semantic page exceeds 16 MiB of decoded content.",
                ));
            }
            let operations = Content::decode(&body)?.operations;
            if stream.dict.has(b"FolioOCR") {
                if result.len() >= 32
                    || !stream.dict.get(b"FolioOCR")?.as_bool()?
                    || stream.dict.iter().any(|(name, _)| {
                        ![b"Length".as_slice(), b"Filter", b"DecodeParms", b"FolioOCR"]
                            .contains(&name.as_slice())
                    })
                {
                    return Err(invalid(
                        "The canonical OCR stream marker or dictionary is unsupported.",
                    ));
                }
                let stream_id = value.as_reference().map_err(|_| {
                    invalid("The canonical OCR stream has no stable indirect identity.")
                })?;
                let logical = self.stream(document, &resources, &operations)?;
                result.push(CanonicalText {
                    stream: stream_id,
                    start: offset,
                    end: offset + operations.len() - 1,
                    text: logical,
                });
            }
            offset += operations.len();
            if offset > 200_000 {
                return Err(invalid(
                    "The semantic page exceeds 200,000 content operations.",
                ));
            }
        }
        Ok(result)
    }

    fn stream(
        &mut self,
        document: &Document,
        resources: &lopdf::Dictionary,
        operations: &[Operation],
    ) -> Result<String> {
        fn empty(operation: &Operation, name: &str) -> bool {
            operation.operator == name && operation.operands.is_empty()
        }
        if operations.len() < 14
            || !empty(&operations[0], "q")
            || !empty(&operations[1], "BT")
            || !empty(&operations[operations.len() - 2], "ET")
            || !empty(&operations[operations.len() - 1], "Q")
        {
            return Err(invalid(
                "Canonical OCR must contain only its complete balanced q/BT/word/ET/Q source scope.",
            ));
        }
        let mut logical = String::new();
        let mut index = 2;
        let mut words = 0;
        while index < operations.len() - 2 {
            if index + 10 > operations.len() - 2 {
                return Err(invalid("The canonical OCR word scope is incomplete."));
            }
            let text = actual_text(&operations[index])?;
            if is_separator(&text) {
                return Err(invalid(
                    "A canonical OCR separator cannot replace a word scope.",
                ));
            }
            let font_operation = &operations[index + 1];
            if font_operation.operator != "Tf"
                || font_operation.operands.len() != 2
                || number(&font_operation.operands[1])? != 1.0
            {
                return Err(invalid(
                    "Canonical OCR must use its verified word font at size 1.",
                ));
            }
            let font = resource(
                document,
                resources,
                b"Font",
                font_operation.operands[0].as_name()?,
            )?;
            for (at, name, expected) in [
                (2, "Tr", 3.0),
                (3, "Tc", 0.0),
                (4, "Tw", 0.0),
                (5, "Tz", 100.0),
                (6, "Ts", 0.0),
            ] {
                let operation = &operations[index + at];
                if operation.operator != name
                    || operation.operands.len() != 1
                    || number(&operation.operands[0])? != expected
                {
                    return Err(invalid(
                        "Canonical OCR must remain invisible Tr3 text with its exact source text-state operations.",
                    ));
                }
            }
            let matrix = &operations[index + 7];
            if matrix.operator != "Tm" || matrix.operands.len() != 6 {
                return Err(invalid("Canonical OCR has an invalid word matrix."));
            }
            let matrix = matrix
                .operands
                .iter()
                .map(number)
                .collect::<Result<Vec<_>>>()?;
            let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
            let horizontal = matrix[0].hypot(matrix[1]);
            let vertical = matrix[2].hypot(matrix[3]);
            if determinant <= 0.0
                || horizontal < 1e-8
                || vertical < 1e-8
                || (matrix[0] * matrix[2] + matrix[1] * matrix[3]).abs()
                    > horizontal * vertical * 0.0001
            {
                return Err(invalid(
                    "Canonical OCR has unsupported, reflected or degenerate word geometry.",
                ));
            }
            let codes = self.font(document, font)?;
            check_code(&operations[index + 8], codes, &text)?;
            if !empty(&operations[index + 9], "EMC") {
                return Err(invalid(
                    "The canonical OCR word has an invalid marked-content end.",
                ));
            }
            logical.push_str(&text);
            words += 1;
            index += 10;
            if index + 3 <= operations.len() - 2
                && operations[index].operator == "BDC"
                && operations[index + 1].operator == "Tj"
            {
                let separator = actual_text(&operations[index])?;
                if !is_separator(&separator) || !empty(&operations[index + 2], "EMC") {
                    return Err(invalid("Canonical OCR has a malformed explicit separator."));
                }
                check_code(&operations[index + 1], codes, &separator)?;
                logical.push_str(&separator);
                index += 3;
            }
            if words > super::MAX_WORDS_PER_PAGE || logical.len() > MAX_LOGICAL_BYTES {
                return Err(invalid(
                    "Canonical OCR semantic text exceeds its bounded word or 2 MiB text limit.",
                ));
            }
        }
        Ok(logical)
    }
}

fn actual_text(operation: &Operation) -> Result<String> {
    if operation.operator != "BDC"
        || operation.operands.len() != 2
        || operation.operands[0].as_name()? != b"Span"
    {
        return Err(invalid(
            "Only exact canonical OCR Span/ActualText word scopes are supported.",
        ));
    }
    let dictionary = operation.operands[1].as_dict()?;
    if dictionary.len() != 1 {
        return Err(invalid(
            "Canonical OCR Span metadata cannot contain tags, MCIDs or additional properties.",
        ));
    }
    let Object::String(bytes, StringFormat::Hexadecimal) = dictionary.get(b"ActualText")? else {
        return Err(invalid(
            "Canonical OCR ActualText must be its exact UTF-16BE source string.",
        ));
    };
    if bytes.len() < 4
        || bytes.len() > 2050
        || !bytes.len().is_multiple_of(2)
        || bytes[..2] != [0xfe, 0xff]
    {
        return Err(invalid(
            "Canonical OCR ActualText has a missing BOM or malformed UTF-16 data.",
        ));
    }
    String::from_utf16(
        &bytes[2..]
            .chunks_exact(2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|_| invalid("Canonical OCR ActualText has invalid Unicode surrogates."))
}

fn check_code(operation: &Operation, codes: &BTreeMap<u16, String>, actual: &str) -> Result<()> {
    if operation.operator != "Tj" || operation.operands.len() != 1 {
        return Err(invalid(
            "Canonical OCR must draw one exact logical-word CID.",
        ));
    }
    let Object::String(bytes, StringFormat::Hexadecimal) = &operation.operands[0] else {
        return Err(invalid(
            "Canonical OCR word encoding must be a hexadecimal CID.",
        ));
    };
    if bytes.len() != 2
        || codes
            .get(&u16::from_be_bytes([bytes[0], bytes[1]]))
            .is_none_or(|value| value != actual)
    {
        return Err(invalid(
            "Canonical OCR ActualText disagrees with the real word CID/ToUnicode mapping.",
        ));
    }
    Ok(())
}

pub(crate) fn analysis_document<'a>(
    document: &'a Document,
    page: usize,
    verified: &[CanonicalText],
) -> Result<Cow<'a, Document>> {
    if verified.is_empty() {
        return Ok(Cow::Borrowed(document));
    }
    let id = page_id(document, page)?;
    let mut content =
        Content::decode(&document.get_page_content_with_limit(id, MAX_EDIT_CONTENT_BYTES)?)?;
    for span in verified {
        for operation in content
            .operations
            .get_mut(span.start..=span.end)
            .ok_or_else(|| invalid("Canonical OCR operation identities changed."))?
        {
            *operation = Operation::new("n", vec![]);
        }
    }
    // Pristine balanced OCR text has no surviving graphics effect. Omit only its verified
    // operations from normal text inspection, retaining every original operation index.
    let mut analysis = document.clone();
    let contents = analysis.add_object(Stream::new(dictionary! {}, content.encode()?));
    analysis.get_dictionary_mut(id)?.set("Contents", contents);
    Ok(Cow::Owned(analysis))
}
