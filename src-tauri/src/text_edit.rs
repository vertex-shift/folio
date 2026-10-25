use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use lopdf::{
    Dictionary, Document, Encoding, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::document::{Result, inherited, invalid, page_id};

const MAX_CONTENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CMAP_BYTES: usize = 2 * 1024 * 1024;
const MAX_OPERATIONS: usize = 200_000;
const MAX_RUNS: usize = 10_000;
const MAX_FONTS: usize = 128;
const MAX_FONT_MAPPINGS: usize = 200_000;
type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub path: Vec<usize>,
    pub text: Option<String>,
    pub font: String,
    pub font_size: Option<f64>,
    pub origin: Option<[f64; 2]>,
    pub editable: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReplacementFont {
    Original,
    Noto,
}

enum Codec {
    Simple(Box<[Option<char>; 256]>),
    Unicode {
        encoding: Encoding<'static>,
        width: usize,
    },
}

impl Codec {
    fn mapping_count(&self) -> usize {
        match self {
            Self::Simple(_) => 256,
            Self::Unicode {
                encoding: Encoding::UnicodeMapEncoding(map),
                ..
            } => map
                .bf_ranges
                .iter()
                .flat_map(|ranges| ranges.iter())
                .map(|(range, _)| (*range.end() - *range.start()) as usize + 1)
                .sum(),
            Self::Unicode { .. } => 0,
        }
    }

    fn units(&self, bytes: &[u8]) -> std::result::Result<Vec<(String, Vec<u8>)>, String> {
        match self {
            Self::Simple(map) => bytes
                .iter()
                .map(|byte| {
                    map[usize::from(*byte)]
                        .map(|character| (character.to_string(), vec![*byte]))
                        .ok_or_else(|| {
                            format!("The font does not map source byte {byte:02X} reliably.")
                        })
                })
                .collect(),
            Self::Unicode {
                encoding: Encoding::UnicodeMapEncoding(map),
                width,
            } => {
                if !bytes.len().is_multiple_of(*width) {
                    return Err("The text has incomplete encoded glyphs.".into());
                }
                bytes
                    .chunks_exact(*width)
                    .map(|bytes| {
                        let code = bytes
                            .iter()
                            .fold(0_u32, |value, byte| value * 256 + u32::from(*byte));
                        let value = map
                            .get(code, *width as u8)
                            .ok_or_else(|| "The font's Unicode map is incomplete.".to_owned())?;
                        let text = String::from_utf16(&value).map_err(|_| {
                            "The font's Unicode map contains invalid text.".to_owned()
                        })?;
                        Ok((text, bytes.to_vec()))
                    })
                    .collect()
            }
            Self::Unicode { .. } => Err("The Unicode codec is invalid.".into()),
        }
    }

    fn decode(&self, bytes: &[u8]) -> std::result::Result<String, String> {
        Ok(self
            .units(bytes)?
            .into_iter()
            .map(|(text, _)| text)
            .collect())
    }

    fn encode(&self, text: &str, original: &[u8]) -> Result<Vec<u8>> {
        let preferred: BTreeMap<_, _> =
            self.units(original).map_err(invalid)?.into_iter().collect();
        let mut result = Vec::new();
        for character in text.chars() {
            let value = character.to_string();
            if let Some(code) = preferred.get(&value) {
                result.extend(code);
                continue;
            }
            let code = match self {
                Self::Simple(map) => map.iter().position(|entry| *entry == Some(character))
                    .map(|index| vec![index as u8]),
                Self::Unicode { encoding: Encoding::UnicodeMapEncoding(map), width } => {
                    let unicode: Vec<_> = character.encode_utf16(&mut [0; 2]).to_vec();
                    map.get_source_codes_for_unicode(&unicode).and_then(|entries| {
                        entries.iter().filter(|entry| usize::from(entry.code_len) == *width)
                            .min_by_key(|entry| entry.source_code)
                            .map(|entry| entry.source_code.to_be_bytes()[4 - width..].to_vec())
                    })
                }
                Self::Unicode { .. } => None,
            }.ok_or_else(|| invalid(format!(
                "The original font cannot encode U+{:04X}. Select Noto Sans to embed a replacement font.", character as u32)))?;
            result.extend(code);
        }
        if self.decode(&result).map_err(invalid)? != text {
            return Err(invalid(
                "The original font could not encode the replacement exactly. Select Noto Sans.",
            ));
        }
        Ok(result)
    }
}

struct BoundFont {
    dictionary: Dictionary,
    id: Option<ObjectId>,
    label: String,
    codec: std::result::Result<Codec, String>,
}

#[derive(Clone)]
struct Graphics {
    matrix: Matrix,
    font: Option<Arc<BoundFont>>,
    font_problem: Option<String>,
    size: Option<f64>,
    rise: f64,
    render_mode: i64,
    fill_alpha: f64,
    stroke_alpha: f64,
    leading: f64,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            matrix: IDENTITY,
            font: None,
            font_problem: None,
            size: None,
            rise: 0.0,
            render_mode: 0,
            fill_alpha: 1.0,
            stroke_alpha: 1.0,
            leading: 0.0,
        }
    }
}

struct LocatedRun {
    view: TextRun,
    font: Option<Arc<BoundFont>>,
    original: Vec<u8>,
}

#[derive(Default)]
struct Scanner {
    bytes: usize,
    operations: usize,
    runs: Vec<LocatedRun>,
    fonts: HashMap<ObjectId, Arc<BoundFont>>,
    inline_fonts: Vec<Arc<BoundFont>>,
    visiting: HashSet<ObjectId>,
    font_mappings: usize,
}

pub fn page_runs(document: &Document, page: usize) -> Result<Vec<TextRun>> {
    Ok(scan_page(document, page)?
        .into_iter()
        .map(|run| run.view)
        .collect())
}

pub(crate) fn replace(
    document: &mut Document,
    page: usize,
    path: &[usize],
    expected: &str,
    replacement: &str,
    font: ReplacementFont,
) -> Result<()> {
    if path.is_empty()
        || path.len() > 17
        || expected.chars().count() > 4000
        || replacement.chars().count() > 4000
        || replacement
            .chars()
            .any(|character| character.is_control() || matches!(character, '\u{2028}' | '\u{2029}'))
    {
        return Err(invalid(
            "Edit one text run at a time, using at most 4,000 characters without line breaks or control characters.",
        ));
    }
    if expected == replacement && font == ReplacementFont::Original {
        return Err(invalid("Change the text before applying the edit."));
    }
    let replacement_for_shaping = if font == ReplacementFont::Noto {
        replacement.nfc().collect::<String>()
    } else {
        replacement.to_owned()
    };
    if requires_shaping(&replacement_for_shaping) {
        return Err(invalid(
            "This replacement needs complex-script or combining-mark shaping, which is not yet supported. Use precomposed supported text where possible.",
        ));
    }
    let run = scan_page(document, page)?
        .into_iter()
        .find(|run| run.view.path == path)
        .ok_or_else(|| invalid("That source text is no longer available. Reload the text list."))?;
    if !run.view.editable {
        return Err(invalid(
            run.view
                .reason
                .unwrap_or_else(|| "This text cannot be edited safely.".into()),
        ));
    }
    if run.view.text.as_deref() != Some(expected) {
        return Err(invalid(
            "The source text changed before this edit. Reload it and try again.",
        ));
    }
    let font_binding = run
        .font
        .ok_or_else(|| invalid("The original text font is missing."))?;
    let size = run
        .view
        .font_size
        .ok_or_else(|| invalid("The original text has an invalid font size."))?;
    let page_id = page_id(document, page)?;
    if document
        .get_pages()
        .values()
        .filter(|id| **id == page_id)
        .count()
        != 1
    {
        return Err(invalid(
            "This PDF reuses a page object. It cannot be edited independently.",
        ));
    }
    let bytes = document.get_page_content_with_limit(page_id, MAX_CONTENT_BYTES)?;
    let resources = page_resources(document, page_id)?;
    let change = Change {
        path,
        expected_font: font_binding,
        original: run.original,
        replacement,
        font,
        size,
    };
    let (content, resources) = change_container(document, bytes, resources, &change)?;
    let mut stream = Stream::new(dictionary! {}, content);
    stream.compress()?;
    let stream_id = document.add_object(stream);
    let page = document.get_dictionary_mut(page_id)?;
    // Never mutate a shared content stream or resource dictionary in place.
    page.set("Contents", stream_id);
    page.set("Resources", resources);
    document.prune_objects();
    Ok(())
}

fn scan_page(document: &Document, page: usize) -> Result<Vec<LocatedRun>> {
    let id = page_id(document, page)?;
    let bytes = document.get_page_content_with_limit(id, MAX_CONTENT_BYTES)?;
    let resources = page_resources(document, id)?;
    let mut scanner = Scanner::default();
    scanner.scan(document, bytes, &resources, Graphics::default(), &[], None)?;
    Ok(scanner.runs)
}

impl Scanner {
    fn scan(
        &mut self,
        document: &Document,
        bytes: Vec<u8>,
        resources: &Dictionary,
        mut graphics: Graphics,
        prefix: &[usize],
        inherited_block: Option<String>,
    ) -> Result<()> {
        self.bytes += bytes.len();
        if self.bytes > MAX_CONTENT_BYTES || prefix.len() > 16 {
            return Err(invalid(
                "This page's text streams are too large or deeply nested for editing.",
            ));
        }
        let content = Content::decode(&bytes)?;
        let inherited_block = if content
            .operations
            .iter()
            .any(|operation| matches!(operation.operator.as_str(), "BI" | "ID" | "EI"))
        {
            Some(
                "This text stream contains inline image data that cannot yet be rewritten safely."
                    .into(),
            )
        } else {
            inherited_block
        };
        self.operations += content.operations.len();
        if self.operations > MAX_OPERATIONS {
            return Err(invalid(
                "This page has too many drawing operations to edit safely.",
            ));
        }
        let mut stack = Vec::new();
        let mut text_matrix = IDENTITY;
        let mut line_matrix = IDENTITY;
        let mut position_known = true;
        let mut in_text = false;
        let mut marks: Vec<Option<String>> = Vec::new();
        for (index, operation) in content.operations.iter().enumerate() {
            match operation.operator.as_str() {
                "q" => stack.push(graphics.clone()),
                "Q" => {
                    graphics = stack
                        .pop()
                        .ok_or_else(|| invalid("The page has an unbalanced graphics state."))?
                }
                "cm" => {
                    graphics.matrix =
                        multiply(graphics.matrix, matrix_operands(&operation.operands)?)
                }
                "BT" => {
                    text_matrix = IDENTITY;
                    line_matrix = IDENTITY;
                    position_known = true;
                    in_text = true;
                }
                "ET" => in_text = false,
                "Tf" => {
                    let name = operand(operation, 0)?.as_name()?;
                    graphics.size = Some(number(operand(operation, 1)?)?);
                    match resource(document, resources, b"Font", name) {
                        Ok(value) => {
                            graphics.font = Some(self.bind_font(document, value));
                            graphics.font_problem = None;
                        }
                        Err(error) => {
                            graphics.font = None;
                            graphics.font_problem = Some(error.to_string());
                        }
                    }
                }
                "gs" => {
                    let name = operand(operation, 0)?.as_name()?;
                    let state = document
                        .dereference(resource(document, resources, b"ExtGState", name)?)?
                        .1
                        .as_dict()?;
                    if let Ok(font) = state.get(b"Font") {
                        let font = document.dereference(font)?.1.as_array()?;
                        if font.len() != 2 {
                            return Err(invalid("An extended graphics font is invalid."));
                        }
                        graphics.font = Some(self.bind_font(document, &font[0]));
                        graphics.font_problem = None;
                        graphics.size = Some(number(&font[1])?);
                    }
                    if let Ok(value) = state.get(b"ca") {
                        graphics.fill_alpha = number(value)?;
                    }
                    if let Ok(value) = state.get(b"CA") {
                        graphics.stroke_alpha = number(value)?;
                    }
                }
                "Tm" => {
                    text_matrix = matrix_operands(&operation.operands)?;
                    line_matrix = text_matrix;
                    position_known = true;
                }
                "TL" => graphics.leading = number(operand(operation, 0)?)?,
                "Ts" => graphics.rise = number(operand(operation, 0)?)?,
                "Tr" => graphics.render_mode = operand(operation, 0)?.as_i64()?,
                "Td" | "TD" => {
                    let x = number(operand(operation, 0)?)?;
                    let y = number(operand(operation, 1)?)?;
                    if operation.operator == "TD" {
                        graphics.leading = -y;
                    }
                    line_matrix = multiply(line_matrix, [1.0, 0.0, 0.0, 1.0, x, y]);
                    text_matrix = line_matrix;
                    position_known = true;
                }
                "T*" => {
                    line_matrix =
                        multiply(line_matrix, [1.0, 0.0, 0.0, 1.0, 0.0, -graphics.leading]);
                    text_matrix = line_matrix;
                    position_known = true;
                }
                "BMC" | "BDC" => marks.push(mark_block(document, resources, operation)?),
                "EMC" => {
                    marks.pop();
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    if operation.operator == "'" || operation.operator == "\"" {
                        line_matrix =
                            multiply(line_matrix, [1.0, 0.0, 0.0, 1.0, 0.0, -graphics.leading]);
                        text_matrix = line_matrix;
                        position_known = true;
                    }
                    let original = shown_bytes(operation)?;
                    let font = graphics.font.clone();
                    let decoded = match &font {
                        Some(font) => match &font.codec {
                            Ok(codec) => codec.decode(&original),
                            Err(reason) => Err(reason.clone()),
                        },
                        None => Err(graphics
                            .font_problem
                            .clone()
                            .unwrap_or_else(|| "This text has no resolved font.".into())),
                    };
                    let mut reason = decoded.as_ref().err().cloned();
                    let size = graphics.size.filter(|size| {
                        size.is_finite() && size.abs() > 0.001 && size.abs() <= 10_000.0
                    });
                    if size.is_none() {
                        reason = Some("This text has an invalid font size.".into());
                    }
                    if !in_text {
                        reason = Some("This text appears outside a valid PDF text object.".into());
                    }
                    if !matches!(graphics.render_mode, 0..=2)
                        || graphics.render_mode == 0 && graphics.fill_alpha <= 0.0
                        || graphics.render_mode == 1 && graphics.stroke_alpha <= 0.0
                        || graphics.render_mode == 2
                            && graphics.fill_alpha <= 0.0
                            && graphics.stroke_alpha <= 0.0
                    {
                        reason = Some("Invisible OCR text and text used for clipping cannot yet be edited safely.".into());
                    }
                    if let Some(block) = inherited_block
                        .as_ref()
                        .or_else(|| marks.iter().rev().flatten().next())
                    {
                        reason = Some(block.clone());
                    }
                    let text = decoded.ok();
                    if text.as_ref().is_some_and(|text| requires_shaping(text)) {
                        reason = Some("This run needs complex-script or combining-mark shaping and cannot yet be edited safely.".into());
                    }
                    if text
                        .as_ref()
                        .is_some_and(|text| text.chars().count() > 4000)
                    {
                        reason =
                            Some("This text run exceeds the 4,000-character editing limit.".into());
                    }
                    if !original.is_empty() {
                        let mut path = prefix.to_vec();
                        path.push(index);
                        let origin = position_known
                            .then(|| {
                                point(graphics.matrix, point(text_matrix, [0.0, graphics.rise]))
                            })
                            .filter(|point| point.iter().all(|value| value.is_finite()));
                        self.runs.push(LocatedRun {
                            view: TextRun {
                                path,
                                text,
                                font: font
                                    .as_ref()
                                    .map(|font| font.label.clone())
                                    .unwrap_or_else(|| "Unresolved font".into()),
                                font_size: size,
                                origin,
                                editable: reason.is_none(),
                                reason,
                            },
                            font,
                            original,
                        });
                        if self.runs.len() > MAX_RUNS {
                            return Err(invalid("This page has more than 10,000 text runs."));
                        }
                    }
                    position_known = false;
                }
                "Do" => {
                    let name = operand(operation, 0)?.as_name()?;
                    let object = resource(document, resources, b"XObject", name)?;
                    let (id, object) = document.dereference(object)?;
                    let stream = object.as_stream()?;
                    if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form") {
                        continue;
                    }
                    let id = id.ok_or_else(|| {
                        invalid("A Form XObject has no stable object identifier.")
                    })?;
                    if !self.visiting.insert(id) {
                        return Err(invalid("The PDF has a cyclic Form XObject."));
                    }
                    let resources = form_resources(document, stream, resources)?;
                    let mut state = graphics.clone();
                    if let Ok(matrix) = stream.dict.get(b"Matrix") {
                        state.matrix = multiply(
                            state.matrix,
                            matrix_operands(document.dereference(matrix)?.1.as_array()?)?,
                        );
                    }
                    let mut path = prefix.to_vec();
                    path.push(index);
                    let block = inherited_block
                        .clone()
                        .or_else(|| marks.iter().rev().flatten().next().cloned());
                    self.scan(
                        document,
                        stream.get_plain_content_with_limit(MAX_CONTENT_BYTES)?,
                        &resources,
                        state,
                        &path,
                        block,
                    )?;
                    self.visiting.remove(&id);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn bind_font(&mut self, document: &Document, value: &Object) -> Arc<BoundFont> {
        let resolved = document.dereference(value);
        if let Ok((Some(id), _)) = resolved.as_ref()
            && let Some(font) = self.fonts.get(id)
        {
            return font.clone();
        }
        if let Ok((None, Object::Dictionary(dictionary))) = resolved.as_ref()
            && let Some(font) = self
                .inline_fonts
                .iter()
                .find(|font| font.dictionary == *dictionary)
        {
            return font.clone();
        }
        if self.fonts.len() + self.inline_fonts.len() >= MAX_FONTS {
            return Arc::new(BoundFont {
                id: None,
                dictionary: Dictionary::new(),
                label: "Font limit reached".into(),
                codec: Err("This page exceeds the 128-font limit for safe text editing.".into()),
            });
        }
        let (id, dictionary, mut codec) = match resolved {
            Ok((id, value)) => match value.as_dict() {
                Ok(font) => (id, font.clone(), strict_codec(document, font)),
                Err(error) => (id, Dictionary::new(), Err(error.to_string())),
            },
            Err(error) => (None, Dictionary::new(), Err(error.to_string())),
        };
        if let Ok(value) = &codec {
            let count = value.mapping_count();
            if count > MAX_FONT_MAPPINGS - self.font_mappings {
                codec = Err("This page has too many font mappings for safe text editing.".into());
            } else {
                self.font_mappings += count;
            }
        }
        let label = dictionary
            .get(b"BaseFont")
            .and_then(Object::as_name)
            .map(|name| String::from_utf8_lossy(name).chars().take(160).collect())
            .unwrap_or_else(|_| "Embedded font".into());
        let font = Arc::new(BoundFont {
            dictionary,
            id,
            label,
            codec,
        });
        if let Some(id) = id {
            self.fonts.insert(id, font.clone());
        } else {
            self.inline_fonts.push(font.clone());
        }
        font
    }
}

fn strict_codec(document: &Document, font: &Dictionary) -> std::result::Result<Codec, String> {
    let subtype = font
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(|error| error.to_string())?;
    if ![b"Type1".as_slice(), b"TrueType", b"Type0"].contains(&subtype) {
        return Err("This font type is not supported for safe text replacement.".into());
    }
    if subtype == b"Type0" {
        let encoding = font
            .get(b"Encoding")
            .and_then(|value| document.dereference(value).map(|(_, value)| value))
            .and_then(Object::as_name)
            .map_err(|error| error.to_string())?;
        if encoding != b"Identity-H" {
            return Err(
                "Vertical or predefined composite-font encodings cannot yet be edited safely."
                    .into(),
            );
        }
    }
    if let Ok(unicode) = font.get(b"ToUnicode") {
        let stream = document
            .dereference(unicode)
            .and_then(|(_, value)| value.as_stream())
            .map_err(|error| error.to_string())?;
        let bytes = stream
            .get_plain_content_with_limit(MAX_CMAP_BYTES)
            .map_err(|error| error.to_string())?;
        validate_cmap(&bytes)?;
        // A shadow dictionary forces Unicode decoding; the lenient library fallback is rejected.
        let shadow =
            dictionary! { "Type" => "Font", "ToUnicode" => Stream::new(dictionary! {}, bytes) };
        return match shadow
            .get_font_encoding_with_limit(document, MAX_CMAP_BYTES)
            .map_err(|error| error.to_string())?
        {
            Encoding::UnicodeMapEncoding(map) => Ok(Codec::Unicode {
                encoding: Encoding::UnicodeMapEncoding(map),
                width: if subtype == b"Type0" { 2 } else { 1 },
            }),
            _ => Err("The font's Unicode map could not be parsed reliably.".into()),
        };
    }
    if subtype == b"Type0" {
        return Err("The composite font has no Unicode map.".into());
    }
    let base_font = font
        .get(b"BaseFont")
        .and_then(Object::as_name)
        .unwrap_or(b"");
    let standard = [
        b"Helvetica".as_slice(),
        b"Helvetica-Bold",
        b"Helvetica-Oblique",
        b"Helvetica-BoldOblique",
        b"Times-Roman",
        b"Times-Bold",
        b"Times-Italic",
        b"Times-BoldItalic",
        b"Courier",
        b"Courier-Bold",
        b"Courier-Oblique",
        b"Courier-BoldOblique",
    ]
    .contains(&base_font);
    let encoding = font
        .get(b"Encoding")
        .ok()
        .map(|value| document.dereference(value).map(|(_, value)| value))
        .transpose()
        .map_err(|error| error.to_string())?;
    match encoding {
        Some(Object::Name(name)) => Ok(Codec::Simple(Box::new(simple_map(name)?))),
        Some(Object::Dictionary(encoding)) => {
            let unknown_base = !encoding.has(b"BaseEncoding") && !standard;
            let base = match encoding.get(b"BaseEncoding") {
                Ok(value) => value.as_name().map_err(|error| error.to_string())?,
                Err(_) => b"StandardEncoding",
            };
            let base_map = simple_map(base)?;
            if !encoding.has(b"Differences") {
                if unknown_base {
                    return Err("The font does not declare a reliable base encoding.".into());
                }
                return Ok(Codec::Simple(Box::new(base_map)));
            }
            let differences = encoding
                .get(b"Differences")
                .and_then(Object::as_array)
                .map_err(|error| error.to_string())?;
            if differences.len() > 1024 {
                return Err("The font's Differences map is too large.".into());
            }
            let mut code: Option<u16> = None;
            for item in differences {
                match item {
                    Object::Integer(value) if (0..=255).contains(value) => {
                        code = Some(*value as u16)
                    }
                    Object::Name(_) => {
                        let value = code
                            .filter(|value| *value <= 255)
                            .ok_or("The font's Differences map has invalid source codes.")?;
                        code = Some(value + 1);
                    }
                    _ => return Err("The font's Differences map has invalid entries.".into()),
                }
            }
            let mut specification = encoding.clone();
            specification.set("Type", "Encoding");
            specification.set("BaseEncoding", Object::Name(base.to_vec()));
            let shadow = dictionary! { "Type" => "Font", "Encoding" => specification };
            let codec = shadow
                .get_font_encoding_with_limit(document, MAX_CMAP_BYTES)
                .map_err(|error| error.to_string())?;
            let Encoding::Differences(differences) = &codec else {
                return Err("The font's Differences encoding could not be parsed reliably.".into());
            };
            let mut map = [None; 256];
            for (index, entry) in map.iter_mut().enumerate() {
                if unknown_base && !differences.map.contains_key(&(index as u8)) {
                    continue;
                }
                let text = codec
                    .bytes_to_string(&[index as u8])
                    .map_err(|error| error.to_string())?;
                let mut chars = text.chars();
                *entry = chars.next().filter(|_| chars.next().is_none());
            }
            Ok(Codec::Simple(Box::new(map)))
        }
        None if standard => Ok(Codec::Simple(Box::new(simple_map(b"StandardEncoding")?))),
        _ => Err("The font has no reliable supported character encoding.".into()),
    }
}

fn simple_map(name: &[u8]) -> std::result::Result<[Option<char>; 256], String> {
    if ![
        b"WinAnsiEncoding".as_slice(),
        b"StandardEncoding",
        b"MacRomanEncoding",
        b"MacExpertEncoding",
    ]
    .contains(&name)
    {
        return Err("The font's byte encoding is not supported for editing.".into());
    }
    let shadow = dictionary! { "Type" => "Font", "Encoding" => Object::Name(name.to_vec()) };
    let document = Document::with_version("1.7");
    let codec = shadow
        .get_font_encoding(&document)
        .map_err(|error| error.to_string())?;
    let mut map = [None; 256];
    for (index, entry) in map.iter_mut().enumerate() {
        let text = codec
            .bytes_to_string(&[index as u8])
            .map_err(|error| error.to_string())?;
        let mut chars = text.chars();
        *entry = chars.next().filter(|_| chars.next().is_none());
    }
    Ok(map)
}

fn validate_cmap(bytes: &[u8]) -> std::result::Result<(), String> {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
                index += 1;
            }
        } else if bytes[index] == b'<' {
            if bytes.get(index + 1) == Some(&b'<') {
                index += 2;
                continue;
            }
            index += 1;
            let mut digits = 0;
            while index < bytes.len() && bytes[index] != b'>' {
                if bytes[index].is_ascii_hexdigit() {
                    digits += 1;
                } else if !bytes[index].is_ascii_whitespace() {
                    return Err("The Unicode map contains invalid hexadecimal text.".into());
                }
                if digits > 4 {
                    return Err(
                        "This font's complex Unicode map is not yet supported for safe editing."
                            .into(),
                    );
                }
                index += 1;
            }
            if index == bytes.len() {
                return Err("The Unicode map has an unterminated value.".into());
            }
        }
        index += 1;
    }
    Ok(())
}

struct Change<'a> {
    path: &'a [usize],
    expected_font: Arc<BoundFont>,
    original: Vec<u8>,
    replacement: &'a str,
    font: ReplacementFont,
    size: f64,
}

fn change_container(
    document: &mut Document,
    bytes: Vec<u8>,
    mut resources: Dictionary,
    change: &Change<'_>,
) -> Result<(Vec<u8>, Dictionary)> {
    let mut content = Content::decode(&bytes)?;
    let index = change.path[0];
    let operation = content
        .operations
        .get(index)
        .ok_or_else(|| invalid("The selected content operation no longer exists."))?
        .clone();
    if change.path.len() > 1 {
        if operation.operator != "Do" {
            return Err(invalid("The selected Form path changed."));
        }
        let name = operand(&operation, 0)?.as_name()?;
        let stream = document
            .dereference(resource(document, &resources, b"XObject", name)?)?
            .1
            .as_stream()?
            .clone();
        let child_resources = form_resources(document, &stream, &resources)?;
        let child = Change {
            path: &change.path[1..],
            expected_font: change.expected_font.clone(),
            original: change.original.clone(),
            replacement: change.replacement,
            font: change.font,
            size: change.size,
        };
        let (bytes, child_resources) = change_container(
            document,
            stream.get_plain_content_with_limit(MAX_CONTENT_BYTES)?,
            child_resources,
            &child,
        )?;
        let mut edited = stream;
        edited.set_plain_content(bytes);
        edited.dict.set("Resources", child_resources);
        edited.compress()?;
        let id = document.add_object(edited);
        let name = install_resource(
            document,
            &mut resources,
            b"XObject",
            "FolioEditedForm",
            id.into(),
        )?;
        content.operations[index].operands[0] = Object::Name(name);
    } else {
        let mut replacement = Vec::new();
        if operation.operator == "'" || operation.operator == "\"" {
            if operation.operator == "\"" {
                replacement.push(Operation::new("Tw", vec![operand(&operation, 0)?.clone()]));
                replacement.push(Operation::new("Tc", vec![operand(&operation, 1)?.clone()]));
            }
            replacement.push(Operation::new("T*", vec![]));
        }
        let mut restore = None;
        let bytes = if change.font == ReplacementFont::Noto && !change.replacement.is_empty() {
            let text: String = change.replacement.nfc().collect();
            let (font, encoding) = crate::writing::embed_text_font(document, &text)?;
            let name = install_resource(
                document,
                &mut resources,
                b"Font",
                "FolioReplacement",
                font.into(),
            )?;
            let original = change
                .expected_font
                .id
                .map(Object::Reference)
                .unwrap_or_else(|| {
                    Object::Reference(document.add_object(change.expected_font.dictionary.clone()))
                });
            let old =
                install_resource(document, &mut resources, b"Font", "FolioOriginal", original)?;
            replacement.push(Operation::new(
                "Tf",
                vec![Object::Name(name), Object::Real(change.size as f32)],
            ));
            restore = Some(Operation::new(
                "Tf",
                vec![Object::Name(old), Object::Real(change.size as f32)],
            ));
            text.chars()
                .flat_map(|character| encoding[&character].to_be_bytes())
                .collect()
        } else {
            change
                .expected_font
                .codec
                .as_ref()
                .map_err(|reason| invalid(reason.clone()))?
                .encode(change.replacement, &change.original)?
        };
        replacement.push(Operation::new(
            "Tj",
            vec![Object::String(bytes, StringFormat::Hexadecimal)],
        ));
        if let Some(restore) = restore {
            replacement.push(restore);
        }
        content.operations.splice(index..=index, replacement);
    }
    Ok((content.encode()?, resources))
}

fn install_resource(
    document: &Document,
    resources: &mut Dictionary,
    kind: &[u8],
    prefix: &str,
    value: Object,
) -> Result<Vec<u8>> {
    let mut entries = match resources.get(kind) {
        Ok(value) => document.dereference(value)?.1.as_dict()?.clone(),
        Err(_) => Dictionary::new(),
    };
    let mut index = 1;
    let name = loop {
        let name = format!("{prefix}{index}").into_bytes();
        if !entries.has(&name) {
            break name;
        }
        index += 1;
        if index > MAX_OPERATIONS {
            return Err(invalid("Too many resource names in this PDF."));
        }
    };
    entries.set(name.clone(), value);
    resources.set(kind, entries);
    Ok(name)
}

fn page_resources(document: &Document, id: ObjectId) -> Result<Dictionary> {
    match inherited(document, id, b"Resources")? {
        Some(value) => Ok(document.dereference(&value)?.1.as_dict()?.clone()),
        None => Ok(Dictionary::new()),
    }
}

fn form_resources(document: &Document, stream: &Stream, parent: &Dictionary) -> Result<Dictionary> {
    match stream.dict.get(b"Resources") {
        Ok(value) => Ok(document.dereference(value)?.1.as_dict()?.clone()),
        Err(_) => Ok(parent.clone()),
    }
}

fn resource<'a>(
    document: &'a Document,
    resources: &'a Dictionary,
    kind: &[u8],
    name: &[u8],
) -> Result<&'a Object> {
    Ok(document
        .dereference(resources.get(kind)?)?
        .1
        .as_dict()?
        .get(name)?)
}

fn mark_block(
    document: &Document,
    resources: &Dictionary,
    operation: &Operation,
) -> Result<Option<String>> {
    if operand(operation, 0)?.as_name()? == b"OC" {
        return Ok(Some(
            "Text inside optional-content layers cannot yet be edited safely.".into(),
        ));
    }
    if operation.operator != "BDC" {
        return Ok(None);
    }
    let value = operand(operation, 1)?;
    let value = if let Ok(name) = value.as_name() {
        resource(document, resources, b"Properties", name)?
    } else {
        value
    };
    let properties = document.dereference(value)?.1.as_dict()?;
    Ok(properties.has(b"ActualText").then(|| {
        "This text uses ActualText replacement metadata and cannot yet be edited safely.".into()
    }))
}

fn shown_bytes(operation: &Operation) -> Result<Vec<u8>> {
    let expected = if operation.operator == "\"" { 3 } else { 1 };
    if operation.operands.len() != expected {
        return Err(invalid(
            "The PDF text operation has an invalid operand count.",
        ));
    }
    if operation.operator == "\"" {
        number(operand(operation, 0)?)?;
        number(operand(operation, 1)?)?;
    }
    if operation.operator == "TJ" {
        let mut bytes = Vec::new();
        for item in operand(operation, 0)?.as_array()? {
            match item {
                Object::String(value, _) => bytes.extend(value),
                Object::Integer(_) => {}
                Object::Real(value) if value.is_finite() => {}
                _ => return Err(invalid("The PDF contains an invalid text-spacing array.")),
            }
        }
        Ok(bytes)
    } else {
        Ok(
            operand(operation, if operation.operator == "\"" { 2 } else { 0 })?
                .as_str()?
                .to_vec(),
        )
    }
}

fn operand(operation: &Operation, index: usize) -> Result<&Object> {
    operation.operands.get(index).ok_or_else(|| {
        invalid(format!(
            "The PDF {} operation is incomplete.",
            operation.operator
        ))
    })
}

fn number(value: &Object) -> Result<f64> {
    let value = f64::from(value.as_float()?);
    if !value.is_finite() {
        return Err(invalid("The PDF contains a non-finite drawing value."));
    }
    Ok(value)
}

fn matrix_operands(values: &[Object]) -> Result<Matrix> {
    if values.len() != 6 {
        return Err(invalid(
            "The PDF contains an invalid transformation matrix.",
        ));
    }
    let mut matrix = IDENTITY;
    for (index, value) in values.iter().enumerate() {
        matrix[index] = number(value)?;
    }
    Ok(matrix)
}

fn multiply(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

fn point(matrix: Matrix, point: [f64; 2]) -> [f64; 2] {
    [
        matrix[0] * point[0] + matrix[2] * point[1] + matrix[4],
        matrix[1] * point[0] + matrix[3] * point[1] + matrix[5],
    ]
}

fn requires_shaping(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(character as u32,
        0x0300..=0x036f | 0x0590..=0x109f | 0x1100..=0x11ff |
        0x1780..=0x18af | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff |
        0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f |
        0x3099..=0x309a | 0xa800..=0xabff | 0xfb1d..=0xfdff |
        0xfe00..=0xfe0f | 0xfe20..=0xfe2f | 0xfe70..=0xfeff)
    })
}
