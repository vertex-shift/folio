use std::{
    collections::{BTreeSet, HashSet},
    time::{Duration, Instant},
};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::document::{DocumentInfo, Result, inherited, invalid};
use crate::page_geometry::PageGeometry;

pub const MAX_PAGES: usize = 50;
pub const MAX_WORDS_PER_PAGE: usize = 10_000;
const MAX_WORDS: usize = 100_000;
const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 32 * 1024 * 1024;
const JOB_LIFETIME: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExistingTextPolicy {
    Skip,
    Force,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub pages: Vec<usize>,
    pub dpi: u16,
    pub language: String,
    pub existing_text: ExistingTextPolicy,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub page: usize,
    pub source_bounds: [f64; 4],
    pub rotation: i32,
    pub user_unit: f64,
    pub pixel_width: u32,
    pub pixel_height: u32,
    pub skipped: bool,
    pub existing_text: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub job_id: u64,
    pub document_id: u64,
    pub revision: u64,
    pub dpi: u16,
    pub pages: Vec<Page>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Word {
    pub text: String,
    pub bounds: [f64; 4],
    pub baseline: [f64; 4],
    pub line: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageResult {
    pub page: usize,
    pub words: Vec<Word>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub document: DocumentInfo,
    pub word_count: usize,
    pub processed_pages: usize,
    pub skipped_pages: Vec<usize>,
    pub no_text_pages: Vec<usize>,
}

pub(crate) fn plan(
    source: &Document,
    document_id: u64,
    revision: u64,
    job_id: u64,
    options: Options,
) -> Result<Plan> {
    if options.language != "eng" {
        return Err(invalid("Offline OCR currently supports English only."));
    }
    if ![150, 300].contains(&options.dpi) {
        return Err(invalid("Choose 150 or 300 DPI for offline OCR."));
    }
    if options.pages.is_empty() || options.pages.len() > MAX_PAGES {
        return Err(invalid("Choose between 1 and 50 pages per OCR job."));
    }
    let selected: BTreeSet<_> = options.pages.iter().copied().collect();
    if selected.len() != options.pages.len() {
        return Err(invalid("Choose each OCR page only once."));
    }
    if let Ok(form) = source.catalog()?.get(b"AcroForm")
        && source.dereference(form)?.1.as_dict()?.has(b"XFA")
    {
        return Err(invalid(
            "XFA documents are not supported by offline OCR. Use a static PDF first.",
        ));
    }
    let ids: Vec<_> = source.get_pages().into_values().collect();
    let mut pages = Vec::with_capacity(selected.len());
    for index in selected {
        let id = *ids
            .get(index)
            .ok_or_else(|| invalid("An OCR page is no longer in this PDF."))?;
        let geometry = PageGeometry::for_page(source, id)?;
        let bounds = geometry.bounds.map(f64::from);
        let rotation = geometry.rotation;
        let user_unit = f64::from(geometry.user_unit);
        let scale = user_unit * f64::from(options.dpi) / 72.0;
        let (mut width, mut height) = (
            (bounds[2] - bounds[0]) * scale,
            (bounds[3] - bounds[1]) * scale,
        );
        if rotation % 180 != 0 {
            std::mem::swap(&mut width, &mut height);
        }
        let (width, height) = (width.ceil(), height.ceil());
        if !width.is_finite()
            || !height.is_finite()
            || width < 1.0
            || height < 1.0
            || width > 16_384.0
            || height > 16_384.0
            || width * height > 24_000_000.0
        {
            return Err(invalid(format!(
                "Page {} exceeds the 24-megapixel or 16,384-pixel OCR limit at {} DPI. Choose 150 DPI or a smaller page; resolution is never lowered silently.",
                index + 1,
                options.dpi
            )));
        }
        let existing_text = page_has_text(source, id)?;
        pages.push(Page {
            page: index,
            source_bounds: bounds,
            rotation,
            user_unit,
            pixel_width: width as u32,
            pixel_height: height as u32,
            skipped: existing_text && matches!(options.existing_text, ExistingTextPolicy::Skip),
            existing_text,
        });
    }
    Ok(Plan {
        job_id,
        document_id,
        revision,
        dpi: options.dpi,
        pages,
    })
}

fn resources(source: &Document, id: ObjectId) -> Result<Dictionary> {
    match inherited(source, id, b"Resources")? {
        Some(value) => Ok(source.dereference(&value)?.1.as_dict()?.clone()),
        None => Ok(Dictionary::new()),
    }
}

fn page_has_text(source: &Document, id: ObjectId) -> Result<bool> {
    let mut content = Vec::new();
    if let Ok(value) = source.get_dictionary(id)?.get(b"Contents") {
        let object = source.dereference(value)?.1;
        let streams = match object {
            Object::Array(values) => values.iter().collect(),
            Object::Stream(_) => vec![value],
            Object::Null => Vec::new(),
            _ => return Err(invalid("A page has unsupported content streams.")),
        };
        for value in streams {
            let bytes = source
                .dereference(value)?
                .1
                .as_stream()?
                .get_plain_content()?;
            if content.len() + bytes.len() > MAX_CONTENT_BYTES {
                return Err(invalid(
                    "This page exceeds the 32 MiB decoded OCR content limit.",
                ));
            }
            content.extend(bytes);
            content.push(b'\n');
        }
    }
    content_has_text(
        source,
        &content,
        &resources(source, id)?,
        &mut HashSet::new(),
        &mut 0,
        0,
    )
}

fn content_has_text(
    source: &Document,
    bytes: &[u8],
    resources: &Dictionary,
    stack: &mut HashSet<ObjectId>,
    decoded: &mut usize,
    depth: usize,
) -> Result<bool> {
    *decoded = decoded.saturating_add(bytes.len());
    if depth > 32 || *decoded > MAX_CONTENT_BYTES {
        return Err(invalid(
            "This page's content is too large or deeply nested for safe OCR text detection.",
        ));
    }
    for op in Content::decode(bytes)?.operations {
        if matches!(op.operator.as_str(), "Tj" | "TJ" | "'" | "\"") {
            fn has_string(object: &Object) -> bool {
                match object {
                    Object::String(value, _) => !value.is_empty(),
                    Object::Array(values) => values.iter().any(has_string),
                    _ => false,
                }
            }
            if op.operands.iter().any(has_string) {
                return Ok(true);
            }
        }
        if op.operator == "Do" {
            let name = op
                .operands
                .first()
                .ok_or_else(|| invalid("A page has an invalid XObject invocation."))?
                .as_name()?;
            let objects = source
                .dereference(resources.get(b"XObject")?)?
                .1
                .as_dict()?;
            let (id, object) = source.dereference(objects.get(name)?)?;
            let stream = object.as_stream()?;
            if stream.dict.get(b"Subtype")?.as_name()? == b"Form" {
                if let Some(id) = id
                    && !stack.insert(id)
                {
                    return Err(invalid(
                        "A cyclic Form XObject cannot be checked safely for OCR.",
                    ));
                }
                let child_resources = match stream.dict.get(b"Resources") {
                    Ok(value) => source.dereference(value)?.1.as_dict()?,
                    Err(lopdf::Error::DictKey(_)) => resources,
                    Err(error) => return Err(error.into()),
                };
                let child = stream.get_plain_content()?;
                if content_has_text(source, &child, child_resources, stack, decoded, depth + 1)? {
                    return Ok(true);
                }
                if let Some(id) = id {
                    stack.remove(&id);
                }
            }
        }
    }
    Ok(false)
}

pub(crate) struct Job {
    pub plan: Plan,
    pub results: Vec<PageResult>,
    started: Instant,
    word_count: usize,
    text_bytes: usize,
}

impl Job {
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            results: Vec::new(),
            started: Instant::now(),
            word_count: 0,
            text_bytes: 0,
        }
    }

    pub fn expired(&self) -> bool {
        self.started.elapsed() >= JOB_LIFETIME
    }

    pub fn stage(&mut self, mut result: PageResult) -> Result<()> {
        let expected = self
            .plan
            .pages
            .iter()
            .filter(|page| !page.skipped)
            .nth(self.results.len())
            .ok_or_else(|| invalid("All OCR pages have already been staged."))?;
        if result.page != expected.page {
            return Err(invalid(
                "OCR pages must be submitted exactly once in document order.",
            ));
        }
        if result.words.len() > MAX_WORDS_PER_PAGE
            || self.word_count + result.words.len() > MAX_WORDS
        {
            return Err(invalid(
                "OCR is limited to 10,000 words per page and 100,000 words per job.",
            ));
        }
        let face = font_face()?;
        let mut bytes = 0;
        let mut previous_line = 0;
        for word in &mut result.words {
            word.text = word.text.nfc().collect();
            validate_word(word, expected, &face)?;
            if word.line < previous_line {
                return Err(invalid(
                    "The OCR reading order is invalid. No text was added.",
                ));
            }
            previous_line = word.line;
            bytes += word.text.len();
        }
        if self.text_bytes + bytes > MAX_TEXT_BYTES {
            return Err(invalid("The OCR result exceeds the 8 MiB text limit."));
        }
        self.word_count += result.words.len();
        self.text_bytes += bytes;
        self.results.push(result);
        Ok(())
    }

    pub fn ensure_complete(&self) -> Result<()> {
        if self.results.len() != self.plan.pages.iter().filter(|page| !page.skipped).count() {
            return Err(invalid("OCR is incomplete. No text was added to this PDF."));
        }
        Ok(())
    }
}

fn font_face() -> Result<ttf_parser::Face<'static>> {
    ttf_parser::Face::parse(crate::writing::FONT, 0)
        .map_err(|error| invalid(format!("The bundled OCR font is invalid: {error}")))
}

fn validate_word(word: &Word, page: &Page, face: &ttf_parser::Face<'_>) -> Result<()> {
    let [left, top, right, bottom] = word.bounds;
    let [x0, y0, x1, y1] = word.baseline;
    let height = bottom - top;
    if word
        .bounds
        .iter()
        .chain(word.baseline.iter())
        .any(|value| !value.is_finite())
        || left < 0.0
        || top < 0.0
        || right > f64::from(page.pixel_width)
        || bottom > f64::from(page.pixel_height)
        || right <= left
        || height <= 0.0
        || (x0 - left).abs() > 0.5
        || (x1 - right).abs() > 0.5
        || x1 <= x0
        || ((y1 - y0) / (x1 - x0)).abs() > 0.25
        || y0.min(y1) <= top
        || y0.max(y1) > bottom + height * 1.5 + 2.0
    {
        return Err(invalid(format!(
            "Page {} has unsupported OCR word/baseline geometry. No text was added.",
            page.page + 1
        )));
    }
    if word.text.is_empty() || word.text.chars().count() > 128 || word.line > 100_000 {
        return Err(invalid(
            "The OCR engine returned an invalid word or reading order.",
        ));
    }
    for character in word.text.chars() {
        let code = character as u32;
        if character.is_whitespace()
            || character.is_control()
            || !matches!(code, 0x21..=0x24f | 0x370..=0x52f | 0x2000..=0x206f | 0x20a0..=0x20cf)
            || face.glyph_index(character).is_none()
        {
            return Err(invalid(format!(
                "The OCR result contains unsupported font/language character U+{code:04X}. No text was added; English with precomposed Latin text is recommended."
            )));
        }
    }
    if !word.text.chars().any(|character| {
        face.glyph_index(character)
            .and_then(|glyph| face.glyph_hor_advance(glyph))
            .is_some_and(|advance| advance > 0)
    }) {
        return Err(invalid(
            "The OCR result contains a word without usable font metrics.",
        ));
    }
    Ok(())
}

fn pdf_point(page: &Page, dpi: u16, x: f64, y: f64) -> [f64; 2] {
    let [left, bottom, right, top] = page.source_bounds;
    let scale = page.user_unit * f64::from(dpi) / 72.0;
    let (x, y) = (x / scale, y / scale);
    match page.rotation {
        0 => [left + x, top - y],
        90 => [left + y, bottom + x],
        180 => [right - x, bottom + y],
        270 => [right - y, top - x],
        _ => unreachable!("validated right-angle page rotation"),
    }
}

fn pdf_vector(page: &Page, dpi: u16, x: f64, y: f64) -> [f64; 2] {
    let origin = pdf_point(page, dpi, 0.0, 0.0);
    let point = pdf_point(page, dpi, x, y);
    [point[0] - origin[0], point[1] - origin[1]]
}

pub(crate) fn apply(source: &mut Document, plan: &Plan, results: &[PageResult]) -> Result<()> {
    let text = results
        .iter()
        .flat_map(|page| &page.words)
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let (font, encoded) = crate::writing::embed_text_font(source, &text)?;
    let face = font_face()?;
    let units = f64::from(face.units_per_em());
    let ascent = f64::from(face.ascender()) / units;
    let ids: Vec<_> = source.get_pages().into_values().collect();
    for result in results.iter().filter(|result| !result.words.is_empty()) {
        let page = plan
            .pages
            .iter()
            .find(|page| page.page == result.page)
            .ok_or_else(|| invalid("The OCR result does not match its page plan."))?;
        let id = ids[result.page];
        let mut page_resources = resources(source, id)?;
        let mut fonts = match page_resources.get(b"Font") {
            Ok(value) => source.dereference(value)?.1.as_dict()?.clone(),
            Err(lopdf::Error::DictKey(_)) => Dictionary::new(),
            Err(error) => return Err(error.into()),
        };
        let mut name = format!("FolioOCR{}", plan.job_id);
        while fonts.has(name.as_bytes()) {
            name.push('_');
        }
        fonts.set(name.clone(), font);
        page_resources.set("Font", fonts);
        let mut operations = vec![
            Operation::new("q", vec![]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(name.into_bytes()), 1.into()]),
            Operation::new("Tr", vec![3.into()]),
            Operation::new("Tc", vec![0.into()]),
            Operation::new("Tw", vec![0.into()]),
            Operation::new("Tz", vec![100.into()]),
            Operation::new("Ts", vec![0.into()]),
        ];
        for (index, word) in result.words.iter().enumerate() {
            let [x0, y0, x1, y1] = word.baseline;
            let slope = (y1 - y0) / (x1 - x0);
            let normalizer = (1.0 + slope * slope).sqrt();
            let advance: f64 = word
                .text
                .chars()
                .map(|character| {
                    let glyph = face.glyph_index(character).expect("staged glyph validated");
                    f64::from(face.glyph_hor_advance(glyph).unwrap_or(face.units_per_em())) / units
                })
                .sum();
            let width = (x1 - x0) * normalizer;
            let height = ((y0 + y1) / 2.0 - word.bounds[1]) / normalizer / ascent;
            let horizontal = pdf_vector(
                page,
                plan.dpi,
                width / advance / normalizer,
                slope * width / advance / normalizer,
            );
            let vertical = pdf_vector(
                page,
                plan.dpi,
                slope * height / normalizer,
                -height / normalizer,
            );
            let position = pdf_point(page, plan.dpi, x0, y0);
            let matrix = [
                horizontal[0],
                horizontal[1],
                vertical[0],
                vertical[1],
                position[0],
                position[1],
            ];
            if matrix.iter().any(|value| {
                !(*value as f32).is_finite()
                    || (*value - f64::from(*value as f32)).abs() * page.user_unit > 0.05
            }) {
                return Err(invalid(format!(
                    "Page {} uses coordinates beyond safe OCR text precision. No text was added.",
                    page.page + 1
                )));
            }
            operations.push(Operation::new(
                "Tm",
                matrix
                    .iter()
                    .map(|value| Object::Real(*value as f32))
                    .collect(),
            ));
            let mut value: Vec<u8> = word
                .text
                .chars()
                .flat_map(|character| encoded[&character].to_be_bytes())
                .collect();
            if result
                .words
                .get(index + 1)
                .is_some_and(|next| next.line == word.line)
            {
                value.extend(encoded[&' '].to_be_bytes());
            }
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(value, StringFormat::Hexadecimal)],
            ));
        }
        operations.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![])]);
        let mut stream = Stream::new(
            dictionary! { "FolioOCR" => true },
            Content { operations }.encode()?,
        );
        stream.compress()?;
        let stream = source.add_object(stream);
        let original = match source.get_dictionary(id)?.get(b"Contents") {
            Ok(value) => match source.dereference(value)?.1 {
                Object::Array(values) => values.clone(),
                Object::Stream(_) => vec![value.clone()],
                Object::Null => Vec::new(),
                _ => return Err(invalid("The source page has unsupported content streams.")),
            },
            Err(lopdf::Error::DictKey(_)) => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        // Prefixing a q/BT/ET/Q stream starts in the pristine page state and restores it
        // before any original operators run, including source CTMs and clipping paths.
        let contents: Vec<_> = std::iter::once(Object::Reference(stream))
            .chain(original)
            .collect();
        let dictionary = source.get_dictionary_mut(id)?;
        dictionary.set("Resources", page_resources);
        dictionary.set("Contents", contents);
    }
    Ok(())
}
