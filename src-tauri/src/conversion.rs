use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

use lopdf::{
    Document, Object, ObjectId, Stream, StringFormat,
    content::{Content, Operation},
    dictionary,
};
use serde::{Deserialize, Serialize};

use crate::{
    authoring_image::{MAX_IMAGE_BYTES, PreparedImage},
    document::{MAX_DOCUMENT_BYTES, Result, invalid, rebuild_page_tree},
    workspace::Workspace,
};

pub const MAX_CREATION_FILES: usize = 50;
pub const MAX_CREATION_PAGES: usize = 500;
pub const MAX_CREATION_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CREATION_DECODED_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_TEXT_FILE_BYTES: usize = 1024 * 1024;
pub const MAX_CREATION_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DOCX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_DOCX_PAGES: usize = 200;
pub const MAX_DOCX_PAGE_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_DOCX_TEXT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DOCX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreationKind {
    Images,
    Text,
}

impl CreationKind {
    pub fn file_limit(self) -> usize {
        match self {
            Self::Images => MAX_IMAGE_BYTES,
            Self::Text => MAX_TEXT_FILE_BYTES,
        }
    }

    fn batch_limit(self) -> usize {
        match self {
            Self::Images => MAX_CREATION_INPUT_BYTES,
            Self::Text => MAX_CREATION_TEXT_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreationSettings {
    pub width: f32,
    pub height: f32,
    pub margin: f32,
    pub font_size: f32,
}

impl CreationSettings {
    pub fn validate(self) -> Result<()> {
        if !self.width.is_finite()
            || !self.height.is_finite()
            || !(144.0..=1440.0).contains(&self.width)
            || !(144.0..=1440.0).contains(&self.height)
            || !self.margin.is_finite()
            || !(0.0..=144.0).contains(&self.margin)
            || self.width - self.margin * 2.0 < 72.0
            || self.height - self.margin * 2.0 < 72.0
            || !self.font_size.is_finite()
            || !(8.0..=36.0).contains(&self.font_size)
        {
            return Err(invalid(
                "Use page dimensions from 144 to 1,440 points, a 0-144 point margin leaving at least 72 points of content in each direction, and an 8-36 point text size.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationInputInfo {
    pub index: usize,
    pub name: String,
    pub byte_length: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationPlan {
    pub job_id: u64,
    pub kind: CreationKind,
    pub inputs: Vec<CreationInputInfo>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreationStatus {
    pub job_id: u64,
    pub phase: &'static str,
    pub page_count: Option<usize>,
    pub byte_length: Option<usize>,
}

struct Input {
    name: String,
    bytes: Vec<u8>,
}

struct PreparedPdf {
    bytes: Vec<u8>,
    pages: usize,
}

enum CreationState {
    Selecting(Vec<Input>),
    Preparing(Receiver<Result<PreparedPdf>>),
    Ready(PreparedPdf),
}

struct CreationJob {
    plan: CreationPlan,
    byte_count: usize,
    cancel: Arc<AtomicBool>,
    state: CreationState,
}

impl Drop for CreationJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(crate) struct CreationJobs {
    next_id: u64,
    job: Option<CreationJob>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl CreationJobs {
    pub(crate) fn is_pending(&self) -> bool {
        self.job.is_some()
            || self
                .worker
                .as_ref()
                .is_some_and(|worker| !worker.is_finished())
    }

    fn get(&self, id: u64) -> Result<&CreationJob> {
        self.job
            .as_ref()
            .filter(|job| job.plan.job_id == id)
            .ok_or_else(|| invalid("This creation was canceled or closed. Choose the files again."))
    }

    fn get_mut(&mut self, id: u64) -> Result<&mut CreationJob> {
        self.job
            .as_mut()
            .filter(|job| job.plan.job_id == id)
            .ok_or_else(|| invalid("This creation was canceled or closed. Choose the files again."))
    }
}

impl Workspace {
    pub fn begin_creation(&mut self, kind: CreationKind) -> Result<CreationPlan> {
        if self.creations.is_pending() {
            return Err(invalid("Finish or cancel the current PDF creation first."));
        }
        if let Some(worker) = self.creations.worker.take() {
            worker.join().map_err(|_| {
                invalid("The previous PDF creation worker failed. No document was opened.")
            })?;
        }
        let job_id = self
            .creations
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Creation identifiers are exhausted. Restart Folio."))?;
        let plan = CreationPlan {
            job_id,
            kind,
            inputs: Vec::new(),
        };
        self.creations.next_id = job_id;
        self.creations.job = Some(CreationJob {
            plan: plan.clone(),
            byte_count: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            state: CreationState::Selecting(Vec::new()),
        });
        Ok(plan)
    }

    pub fn creation_plan(&self, job_id: u64) -> Result<CreationPlan> {
        Ok(self.creations.get(job_id)?.plan.clone())
    }

    pub fn stage_creation_file(
        &mut self,
        job_id: u64,
        name: String,
        bytes: Vec<u8>,
    ) -> Result<CreationPlan> {
        let result = (|| {
            let job = self.creations.get_mut(job_id)?;
            let CreationState::Selecting(inputs) = &mut job.state else {
                return Err(invalid("Files cannot change after PDF preparation starts."));
            };
            if inputs.len() >= MAX_CREATION_FILES {
                return Err(invalid("Create a PDF from no more than 50 files."));
            }
            if name.is_empty()
                || name.len() > 512
                || name.chars().any(char::is_control)
                || name.contains(['/', '\\'])
            {
                return Err(invalid("The selected input has an invalid filename."));
            }
            if bytes.is_empty() || bytes.len() > job.plan.kind.file_limit() {
                return Err(invalid(match job.plan.kind {
                    CreationKind::Images => "Each PNG/JPEG must contain 1 byte to 16 MiB.",
                    CreationKind::Text => "Each UTF-8 text file must contain 1 byte to 1 MiB.",
                }));
            }
            let count = job
                .byte_count
                .checked_add(bytes.len())
                .ok_or_else(|| invalid("The selected input byte count is too large."))?;
            if count > job.plan.kind.batch_limit() {
                return Err(invalid(match job.plan.kind {
                    CreationKind::Images => "The selected images exceed the 64 MiB input limit.",
                    CreationKind::Text => "The selected text files exceed the 4 MiB input limit.",
                }));
            }
            let info = CreationInputInfo {
                index: inputs.len(),
                name: name.clone(),
                byte_length: bytes.len(),
            };
            inputs.push(Input { name, bytes });
            job.byte_count = count;
            job.plan.inputs.push(info);
            Ok(job.plan.clone())
        })();
        if result.is_err() {
            self.cancel_creation(job_id)?;
        }
        result
    }

    pub fn prepare_creation(
        &mut self,
        job_id: u64,
        settings: CreationSettings,
        order: Vec<usize>,
    ) -> Result<CreationStatus> {
        let result = (|| {
            settings.validate()?;
            let job = self.creations.get_mut(job_id)?;
            let CreationState::Selecting(inputs) = &mut job.state else {
                return Err(invalid("This PDF has already been prepared."));
            };
            let used: HashSet<_> = order.iter().copied().collect();
            if inputs.is_empty()
                || order.len() != inputs.len()
                || used.len() != inputs.len()
                || order.iter().any(|index| *index >= inputs.len())
            {
                return Err(invalid(
                    "Include every chosen file exactly once in the page order.",
                ));
            }
            let inputs = std::mem::take(inputs);
            let kind = job.plan.kind;
            let cancel = Arc::clone(&job.cancel);
            let (sender, receiver) = mpsc::channel();
            let worker = std::thread::Builder::new()
                .name("folio-pdf-creation".into())
                .spawn(move || {
                    let result = create_pdf(kind, inputs, &order, settings, &cancel);
                    // A canceled job drops its receiver and never registers the result.
                    let _ = sender.send(result);
                })?;
            job.state = CreationState::Preparing(receiver);
            self.creations.worker = Some(worker);
            Ok(CreationStatus {
                job_id,
                phase: "preparing",
                page_count: None,
                byte_length: None,
            })
        })();
        if result.is_err() {
            self.cancel_creation(job_id)?;
        }
        result
    }

    pub fn creation_status(&mut self, job_id: u64) -> Result<CreationStatus> {
        let result = (|| {
            let job = self.creations.get_mut(job_id)?;
            if let CreationState::Preparing(receiver) = &job.state {
                match receiver.try_recv() {
                    Ok(result) => job.state = CreationState::Ready(result?),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        return Err(invalid(
                            "PDF creation stopped unexpectedly. No document was opened.",
                        ));
                    }
                }
            }
            Ok(match &job.state {
                CreationState::Ready(pdf) => CreationStatus {
                    job_id,
                    phase: "ready",
                    page_count: Some(pdf.pages),
                    byte_length: Some(pdf.bytes.len()),
                },
                _ => CreationStatus {
                    job_id,
                    phase: "preparing",
                    page_count: None,
                    byte_length: None,
                },
            })
        })();
        if result.is_err() {
            self.cancel_creation(job_id)?;
        }
        result
    }

    pub fn commit_creation(&mut self, job_id: u64) -> Result<crate::document::DocumentInfo> {
        self.creation_status(job_id)?;
        if !matches!(self.creations.get(job_id)?.state, CreationState::Ready(_)) {
            return Err(invalid(
                "Wait until the complete PDF is ready before opening it.",
            ));
        }
        let mut job = self.creations.job.take().expect("job checked");
        let CreationState::Ready(pdf) =
            std::mem::replace(&mut job.state, CreationState::Selecting(Vec::new()))
        else {
            return Err(invalid("The prepared PDF is unavailable."));
        };
        self.add_created_pdf(
            pdf.bytes,
            match job.plan.kind {
                CreationKind::Images => "From images.pdf",
                CreationKind::Text => "From text.pdf",
            }
            .into(),
        )
    }

    pub fn cancel_creation(&mut self, job_id: u64) -> Result<bool> {
        if self
            .creations
            .job
            .as_ref()
            .is_some_and(|job| job.plan.job_id == job_id)
        {
            self.creations.job = None;
            if let Some(worker) = self.creations.worker.take() {
                worker.join().map_err(|_| invalid("The canceled PDF creation worker failed during cleanup. No document was opened."))?;
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

fn check_canceled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Acquire) {
        return Err(invalid("PDF creation canceled. No document was opened."));
    }
    Ok(())
}

fn normalize_text(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        invalid("Text creation requires strict UTF-8. Invalid bytes are not replaced.")
    })?;
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "    ");
    if text.trim().is_empty() {
        return Err(invalid("The text file contains no readable text."));
    }
    Ok(text)
}

fn create_pdf(
    kind: CreationKind,
    inputs: Vec<Input>,
    order: &[usize],
    settings: CreationSettings,
    cancel: &AtomicBool,
) -> Result<PreparedPdf> {
    let mut document = Document::with_version("1.7");
    let catalog = document.add_object(dictionary! { "Type" => "Catalog" });
    document.trailer.set("Root", catalog);
    let mut pages = Vec::new();
    let mut decoded = 0_usize;
    for &index in order {
        check_canceled(cancel)?;
        let input = &inputs[index];
        let result = match kind {
            CreationKind::Images => (|| {
                let image = PreparedImage::decode(&input.bytes)?;
                decoded = decoded
                    .checked_add(image.rgb.len() + image.alpha.as_ref().map_or(0, Vec::len))
                    .ok_or_else(|| invalid("The decoded image byte count is too large."))?;
                if decoded > MAX_CREATION_DECODED_BYTES {
                    return Err(invalid(
                        "The batch exceeds the 128 MiB decoded-image budget. Choose fewer or smaller images.",
                    ));
                }
                check_canceled(cancel)?;
                let page =
                    crate::pages::blank_page(&mut document, settings.width, settings.height)?;
                let resource = image.embed(&mut document)?;
                let scale = ((settings.width - 2.0 * settings.margin) / image.width as f32)
                    .min((settings.height - 2.0 * settings.margin) / image.height as f32);
                let width = image.width as f32 * scale;
                let height = image.height as f32 * scale;
                let contents = Content {
                    operations: vec![
                        Operation::new("q", vec![]),
                        Operation::new(
                            "cm",
                            [
                                width,
                                0.0,
                                0.0,
                                height,
                                (settings.width - width) / 2.0,
                                (settings.height - height) / 2.0,
                            ]
                            .into_iter()
                            .map(Object::from)
                            .collect(),
                        ),
                        Operation::new("Do", vec![Object::Name(b"Image0".to_vec())]),
                        Operation::new("Q", vec![]),
                    ],
                }
                .encode()?;
                set_page_contents(
                    &mut document,
                    page,
                    contents,
                    dictionary! { "XObject" => dictionary! { "Image0" => resource } },
                )?;
                pages.push(page);
                Ok(())
            })(),
            CreationKind::Text => (|| {
                let text = normalize_text(&input.bytes)?;
                add_text_pages(&mut document, &text, settings, &mut pages, cancel)
            })(),
        };
        result.map_err(|error| invalid(format!("{}: {error}", input.name)))?;
        if pages.len() > MAX_CREATION_PAGES {
            return Err(invalid(
                "The created PDF exceeds 500 pages. Choose fewer files or a larger page.",
            ));
        }
    }
    check_canceled(cancel)?;
    rebuild_page_tree(&mut document, &pages)?;
    document.compress();
    let mut bytes = Vec::new();
    document.save_to(&mut bytes)?;
    check_canceled(cancel)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid(
            "The created PDF exceeds 128 MiB. No document was opened.",
        ));
    }
    Ok(PreparedPdf {
        bytes,
        pages: pages.len(),
    })
}

fn set_page_contents(
    document: &mut Document,
    page: ObjectId,
    contents: Vec<u8>,
    resources: lopdf::Dictionary,
) -> Result<()> {
    let mut stream = Stream::new(dictionary! {}, contents);
    stream.compress()?;
    let stream = document.add_object(stream);
    let dictionary = document.get_dictionary_mut(page)?;
    dictionary.set("Contents", stream);
    dictionary.set("Resources", resources);
    Ok(())
}

fn add_text_pages(
    document: &mut Document,
    text: &str,
    settings: CreationSettings,
    pages: &mut Vec<ObjectId>,
    cancel: &AtomicBool,
) -> Result<()> {
    let face = ttf_parser::Face::parse(crate::writing::FONT, 0)
        .map_err(|error| invalid(format!("The bundled text font is invalid: {error}")))?;
    let scale = settings.font_size / f32::from(face.units_per_em());
    let ascent = f32::from(face.ascender()) * scale;
    let descent = f32::from(face.descender()).abs() * scale;
    let leading = (settings.font_size * 1.45).max(ascent + descent);
    let height = settings.height - settings.margin * 2.0;
    let line_count = ((height - ascent - descent) / leading).floor() as usize + 1;
    if height < ascent + descent || line_count == 0 {
        return Err(invalid(
            "The selected text size does not fit the page and margins.",
        ));
    }
    let width = settings.width - settings.margin * 2.0;
    let mut advances = BTreeMap::new();
    for (index, character) in text.chars().enumerate() {
        if index % 1024 == 0 {
            check_canceled(cancel)?;
        }
        if character != '\n' && !advances.contains_key(&character) {
            let glyph = crate::writing::embedded_text_glyph(&face, character)?;
            let advance =
                f32::from(face.glyph_hor_advance(glyph).unwrap_or(face.units_per_em())) * scale;
            if advance > width {
                return Err(invalid("A glyph does not fit the selected page width."));
            }
            advances.insert(character, advance);
        }
    }
    let (font, encoded) = crate::writing::embed_text_font(document, text)?;
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        check_canceled(cancel)?;
        let mut current = String::new();
        let mut used = 0.0;
        for word in paragraph.split_inclusive(char::is_whitespace) {
            check_canceled(cancel)?;
            let word_width: f32 = word.chars().map(|character| advances[&character]).sum();
            if !current.is_empty() && used + word_width > width {
                lines.push(std::mem::take(&mut current));
                used = 0.0;
            }
            for character in word.chars() {
                let advance = advances[&character];
                if !current.is_empty() && used + advance > width {
                    lines.push(std::mem::take(&mut current));
                    used = 0.0;
                }
                current.push(character);
                used += advance;
            }
            if pages.len() + (lines.len() + 1).div_ceil(line_count) > MAX_CREATION_PAGES {
                return Err(invalid(
                    "The created PDF exceeds 500 pages. No text was truncated.",
                ));
            }
        }
        lines.push(current);
        if pages.len() + lines.len().div_ceil(line_count) > MAX_CREATION_PAGES {
            return Err(invalid(
                "The created PDF exceeds 500 pages. No text was truncated.",
            ));
        }
    }
    if pages.len() + lines.len().div_ceil(line_count) > MAX_CREATION_PAGES {
        return Err(invalid(
            "The created PDF exceeds 500 pages. No text was truncated.",
        ));
    }
    for chunk in lines.chunks(line_count) {
        check_canceled(cancel)?;
        let page = crate::pages::blank_page(document, settings.width, settings.height)?;
        let mut operations = vec![
            Operation::new("BT", vec![]),
            Operation::new(
                "Tf",
                vec![
                    Object::Name(b"FolioFont".to_vec()),
                    settings.font_size.into(),
                ],
            ),
            Operation::new("rg", vec![0.into(), 0.into(), 0.into()]),
            Operation::new("TL", vec![leading.into()]),
            Operation::new(
                "Td",
                vec![
                    settings.margin.into(),
                    (settings.height - settings.margin - ascent).into(),
                ],
            ),
        ];
        for (index, line) in chunk.iter().enumerate() {
            if index > 0 {
                operations.push(Operation::new("T*", vec![]));
            }
            let bytes = line
                .chars()
                .flat_map(|character| encoded[&character].to_be_bytes())
                .collect();
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(bytes, StringFormat::Hexadecimal)],
            ));
        }
        operations.push(Operation::new("ET", vec![]));
        set_page_contents(
            document,
            page,
            Content { operations }.encode()?,
            dictionary! { "Font" => dictionary! { "FolioFont" => font } },
        )?;
        pages.push(page);
    }
    Ok(())
}

pub(crate) fn validate_docx_text(bytes: &[u8]) -> Result<&str> {
    if bytes.len() > MAX_DOCX_PAGE_TEXT_BYTES {
        return Err(invalid(
            "A DOCX page exceeds 256 KiB of extracted text. Choose other pages.",
        ));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| invalid("DOCX page text must be valid UTF-8."))?;
    for character in text.chars() {
        let code = character as u32;
        if !matches!(code, 0x9 | 0xa | 0xd | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x10000..=0x10ffff) {
            return Err(invalid(format!(
                "DOCX cannot represent extracted XML character U+{code:04X}. No text was silently removed."
            )));
        }
    }
    Ok(text)
}

pub(crate) fn docx(entries: &[Vec<u8>]) -> Result<Vec<u8>> {
    let mut document = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>",
    );
    let mut total = 0_usize;
    let mut has_text = false;
    for (index, entry) in entries.iter().enumerate() {
        let text = validate_docx_text(entry)?;
        total += entry.len();
        if total > MAX_DOCX_TEXT_BYTES {
            return Err(invalid("DOCX conversion exceeds 8 MiB of extracted text."));
        }
        has_text |= !text.trim().is_empty();
        if index != 0 {
            document.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for paragraph in text.split('\n') {
            document.push_str("<w:p><w:r>");
            for (index, part) in paragraph.split('\t').enumerate() {
                if index != 0 {
                    document.push_str("<w:tab/>");
                }
                document.push_str("<w:t xml:space=\"preserve\">");
                for character in part.chars() {
                    match character {
                        '&' => document.push_str("&amp;"),
                        '<' => document.push_str("&lt;"),
                        '>' => document.push_str("&gt;"),
                        '"' => document.push_str("&quot;"),
                        '\'' => document.push_str("&apos;"),
                        value => document.push(value),
                    }
                }
                document.push_str("</w:t>");
            }
            document.push_str("</w:r></w:p>");
        }
    }
    if !has_text {
        return Err(invalid(
            "No editable text was found on these pages. Image-only scans require a separate OCR operation.",
        ));
    }
    document.push_str("<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/></w:sectPr></w:body></w:document>");
    let content_types = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    let relationships = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    let bytes = crate::output::stored_zip(&[
        ("[Content_Types].xml", content_types),
        ("_rels/.rels", relationships),
        ("word/document.xml", document.as_bytes()),
    ])?;
    if bytes.len() > MAX_DOCX_BYTES {
        return Err(invalid(
            "The DOCX package exceeds 64 MiB. No copy was saved.",
        ));
    }
    Ok(bytes)
}
