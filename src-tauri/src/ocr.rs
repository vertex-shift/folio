use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    time::{Duration, Instant},
};

use lopdf::{
    Dictionary, Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use serde::{Deserialize, Serialize};
mod catalog;
mod font;

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

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Segmentation {
    #[default]
    Auto,
    SingleColumn,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub pages: Vec<usize>,
    pub dpi: u16,
    pub languages: Vec<String>,
    pub layout: String,
    #[serde(default)]
    pub segmentation: Segmentation,
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
    pub source_epoch: u64,
    pub epoch: u64,
    pub dpi: u16,
    pub languages: Vec<String>,
    pub layout: String,
    pub segmentation: Segmentation,
    pub pages: Vec<Page>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Word {
    pub text: String,
    pub bounds: [f64; 4],
    pub baseline: [f64; 4],
    pub line: u32,
    #[serde(rename = "lineBounds")]
    pub line_bounds: [f64; 4],
    pub direction: Direction,
    pub separator: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Ltr,
    Rtl,
}

impl Direction {
    fn key(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }
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
    pub languages: Vec<String>,
}

pub(crate) fn plan(
    source: &Document,
    document_id: u64,
    revision: u64,
    source_epoch: u64,
    job_id: u64,
    options: Options,
) -> Result<Plan> {
    catalog::selection(&options.languages)?;
    if options.layout != "horizontal_print" {
        return Err(invalid(
            "Offline OCR supports horizontal printed scans only, not vertical CJK or handwriting.",
        ));
    }
    let root = source.catalog()?;
    if root.has(b"StructTreeRoot") || root.has(b"MarkInfo") {
        return Err(invalid(
            "Existing tagged or ambiguously structured PDFs are not supported by OCR. Adding unassociated text could stale their logical structure. Recognize the untagged original before authoring a new tagged copy.",
        ));
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
        source_epoch,
        epoch: 0,
        dpi: options.dpi,
        languages: options.languages,
        layout: options.layout,
        segmentation: options.segmentation,
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

    pub fn stage(&mut self, result: PageResult) -> Result<()> {
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
        let languages = catalog::selection(&self.plan.languages)?;
        let mut bytes = 0;
        let mut previous_line = 0;
        let mut previous_word: Option<&Word> = None;
        for word in &result.words {
            validate_word(word, expected)?;
            font::word_font(word, &languages)?;
            if word.line < previous_line {
                return Err(invalid(
                    "The OCR reading order is invalid. No text was added.",
                ));
            }
            if let Some(previous) = previous_word
                && previous.line == word.line
                && (previous.line_bounds != word.line_bounds
                    || previous.direction != word.direction)
            {
                return Err(invalid(
                    "OCR words in one logical line must share its actual line box and reading direction. No text was added.",
                ));
            }
            previous_line = word.line;
            previous_word = Some(word);
            bytes += word.text.len() + word.separator.len();
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

fn validate_word(word: &Word, page: &Page) -> Result<()> {
    let [left, top, right, bottom] = word.bounds;
    let [x0, y0, x1, y1] = word.baseline;
    let height = bottom - top;
    let [line_left, line_top, line_right, line_bottom] = word.line_bounds;
    let line_margin = (line_bottom - line_top) * 0.5 + 2.0;
    if word
        .bounds
        .iter()
        .chain(word.baseline.iter())
        .chain(word.line_bounds.iter())
        .any(|value| !value.is_finite())
        || left < 0.0
        || top < 0.0
        || right > f64::from(page.pixel_width)
        || bottom > f64::from(page.pixel_height)
        || right <= left
        || height <= 0.0
        || line_left < 0.0
        || line_top < 0.0
        || line_right > f64::from(page.pixel_width)
        || line_bottom > f64::from(page.pixel_height)
        || line_right <= line_left
        || line_bottom <= line_top
        || left < line_left - line_margin
        || right > line_right + line_margin
        || top < line_top - line_margin
        || bottom > line_bottom + line_margin
        || (x0 - left).abs() > 0.5
        || (x1 - right).abs() > 0.5
        || x1 <= x0
        || ((y1 - y0) / (x1 - x0)).abs() > 0.25
        || y0.min(y1) < line_top - line_margin
        || y0.max(y1) > line_bottom + line_margin
    {
        return Err(invalid(format!(
            "Page {} has unsupported OCR word/baseline geometry. No text was added.",
            page.page + 1
        )));
    }
    if word.text.is_empty()
        || word.text.chars().count() > 256
        || word.line > 100_000
        || word.separator.len() > 256
        || !word
            .separator
            .bytes()
            .all(|value| matches!(value, b' ' | b'\n' | b'\r' | b'\t'))
    {
        return Err(invalid(
            "The OCR engine returned an invalid word or reading order.",
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
    let languages = catalog::selection(&plan.languages)?;
    let mut groups = BTreeMap::<_, Vec<&Word>>::new();
    let mut selections = BTreeMap::new();
    for result in results {
        for (index, word) in result.words.iter().enumerate() {
            let id = font::word_font(word, &languages)?;
            groups.entry((id, word.direction)).or_default().push(word);
            selections.insert((result.page, index), id);
        }
    }
    let mut embedded = BTreeMap::new();
    if groups.keys().map(|(id, _)| id.bytes().len()).sum::<usize>()
        > catalog::catalog()?.max_font_bytes
    {
        return Err(invalid(
            "The actual OCR font resources exceed the 32 MiB job limit. No text was added.",
        ));
    }
    for ((id, direction), words) in &groups {
        embedded.insert(
            (*id, *direction),
            font::embed(source, *id, *direction, words)?,
        );
    }
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
        let mut names = BTreeMap::new();
        for (key, font) in &embedded {
            if !result
                .words
                .iter()
                .enumerate()
                .any(|(index, word)| (selections[&(result.page, index)], word.direction) == *key)
            {
                continue;
            }
            let mut name = format!("FolioOCR{}_{}_{}", plan.job_id, key.0.key(), key.1.key());
            while fonts.has(name.as_bytes()) {
                name.push('_');
            }
            fonts.set(name.clone(), font.object);
            names.insert(*key, name);
        }
        page_resources.set("Font", fonts);
        let mut operations = vec![Operation::new("q", vec![]), Operation::new("BT", vec![])];
        for (index, word) in result.words.iter().enumerate() {
            let id = selections[&(result.page, index)];
            let key = (id, word.direction);
            let face = ttf_parser::Face::parse(id.bytes(), 0)
                .map_err(|error| invalid(format!("The OCR font is invalid: {error}")))?;
            let ascent = f64::from(face.ascender()) / f64::from(face.units_per_em());
            let [x0, y0, x1, y1] = word.baseline;
            let slope = (y1 - y0) / (x1 - x0);
            let normalizer = (1.0 + slope * slope).sqrt();
            let advance = font::advance(&face, &word.text);
            if !advance.is_finite() || advance <= 0.0 || ascent <= 0.0 {
                return Err(invalid(
                    "OCR has unusable word font metrics. No text was added.",
                ));
            }
            let width = (x1 - x0) * normalizer;
            let height = ((y0 + y1) / 2.0 - word.line_bounds[1]) / normalizer / ascent;
            if !height.is_finite() || height <= 0.0 {
                return Err(invalid(
                    "OCR has an unsupported line baseline above its line box. No text was added.",
                ));
            }
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
            let text = word.text.clone();
            let actual_text = std::iter::once(0xfeff_u16)
                .chain(text.encode_utf16())
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            operations.extend([
                Operation::new("BDC", vec![Object::Name(b"Span".to_vec()),
                    Object::Dictionary(dictionary!{"ActualText"=>Object::String(actual_text,StringFormat::Hexadecimal)})]),
                Operation::new("Tf",vec![Object::Name(names[&key].as_bytes().to_vec()),1.into()]),
                Operation::new("Tr",vec![3.into()]),
                Operation::new("Tc",vec![0.into()]),
                Operation::new("Tw",vec![0.into()]),
                Operation::new("Tz",vec![100.into()]),
                Operation::new("Ts",vec![0.into()]),
            ]);
            operations.push(Operation::new(
                "Tm",
                matrix
                    .iter()
                    .map(|value| Object::Real(*value as f32))
                    .collect(),
            ));
            let value = embedded[&key].encoded[&text].to_be_bytes().to_vec();
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(value, StringFormat::Hexadecimal)],
            ));
            operations.push(Operation::new("EMC", vec![]));
            if !word.separator.is_empty() {
                let unicode = std::iter::once(0xfeff_u16)
                    .chain(word.separator.encode_utf16())
                    .flat_map(u16::to_be_bytes)
                    .collect::<Vec<_>>();
                operations.extend([
                    Operation::new("BDC",vec![Object::Name(b"Span".to_vec()),
                        Object::Dictionary(dictionary!{"ActualText"=>Object::String(unicode,StringFormat::Hexadecimal)})]),
                    Operation::new("Tj",vec![Object::String(embedded[&key].encoded[&word.separator].to_be_bytes().to_vec(),StringFormat::Hexadecimal)]),
                    Operation::new("EMC",vec![]),
                ]);
            }
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
