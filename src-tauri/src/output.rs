use std::{
    collections::{BTreeMap, HashSet},
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use lopdf::{Document, Object, ObjectId};
use serde::{Deserialize, Serialize};

use crate::{
    document::{MAX_DOCUMENT_BYTES, Result, Session, invalid},
    workspace::Workspace,
};

pub const MAX_OUTPUT_PAGES: usize = 500;
pub const MAX_PRINT_PAGES: usize = 200;
pub const MAX_PAGE_PIXELS: u64 = 24_000_000;
pub const MAX_PRINT_PIXELS: u64 = 100_000_000;
pub const MAX_PAGE_DIMENSION: u32 = 16_384;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_OPTIMIZE_OBJECTS: usize = 200_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Print,
    Png,
    Jpeg,
    Text,
    Docx,
    Optimize,
}

impl OutputKind {
    pub fn extension(self, pages: usize) -> &'static str {
        match self {
            Self::Png | Self::Jpeg if pages > 1 => "zip",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Text => "txt",
            Self::Docx => "docx",
            Self::Optimize => "pdf",
            Self::Print => "png",
        }
    }

    pub fn mime(self, pages: usize) -> &'static str {
        match self.extension(pages) {
            "zip" => "application/zip",
            "png" => "image/png",
            "jpg" => "image/jpeg",
            "txt" => "text/plain;charset=utf-8",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            _ => "application/pdf",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputRequest {
    pub document_id: u64,
    pub revision: u64,
    pub kind: OutputKind,
    pub pages: Vec<usize>,
    pub dpi: Option<u16>,
    pub quality: Option<u8>,
    #[serde(default)]
    pub allow_unencrypted: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputPlan {
    pub job_id: u64,
    pub document_id: u64,
    pub revision: u64,
    pub kind: OutputKind,
    pub pages: Vec<usize>,
    pub dpi: Option<u16>,
    pub quality: Option<u8>,
    pub suggested_name: String,
    pub source_bytes: usize,
    pub password_protected: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationReport {
    pub before_bytes: usize,
    pub candidate_bytes: usize,
    pub after_bytes: usize,
    pub reduced: bool,
    pub compressed_streams: usize,
    pub removed_objects: usize,
}

pub(crate) struct OptimizedCopy {
    pub bytes: Vec<u8>,
    pub report: OptimizationReport,
}

struct Job {
    plan: OutputPlan,
    entries: Vec<Vec<u8>>,
    staged: usize,
    byte_count: usize,
    print_pixels: u64,
    optimized: Option<OptimizedCopy>,
}

#[derive(Default)]
pub(crate) struct OutputJobs {
    jobs: BTreeMap<u64, Job>,
    next_id: u64,
}

impl OutputJobs {
    pub fn is_pending(&self) -> bool {
        !self.jobs.is_empty()
    }

    pub fn close_document(&mut self, document_id: u64) {
        self.invalidate_document(document_id);
    }

    pub(crate) fn invalidate_document(&mut self, document_id: u64) {
        self.jobs
            .retain(|_, job| job.plan.document_id != document_id);
    }

    pub fn has_document(&self, document_id: u64) -> bool {
        self.jobs
            .values()
            .any(|job| job.plan.document_id == document_id)
    }

    fn job(&self, id: u64) -> Result<&Job> {
        self.jobs
            .get(&id)
            .ok_or_else(|| invalid("This output was canceled or closed. Prepare it again."))
    }

    fn job_mut(&mut self, id: u64) -> Result<&mut Job> {
        self.jobs
            .get_mut(&id)
            .ok_or_else(|| invalid("This output was canceled or closed. Prepare it again."))
    }
}

impl Workspace {
    pub fn begin_output(&mut self, request: OutputRequest) -> Result<OutputPlan> {
        if self.outputs.is_pending() {
            return Err(invalid(
                "Finish or cancel the prepared output before starting another output job.",
            ));
        }
        let session = self.ensure_export_ready(request.document_id, request.revision)?;
        if self.has_redaction_job(request.document_id) {
            return Err(invalid("Clear pending redaction output before preparing another export."));
        }
        validate_request(session, &request)?;
        let info = session.info()?;
        let stem = safe_stem(&info.name);
        let extension = request.kind.extension(request.pages.len());
        let suggested_name = match request.kind {
            OutputKind::Png | OutputKind::Jpeg if request.pages.len() == 1 => {
                format!("{stem}-page-{:04}.{extension}", request.pages[0] + 1)
            }
            OutputKind::Png | OutputKind::Jpeg => format!("{stem}-pages.zip"),
            OutputKind::Text => format!("{stem}-text.txt"),
            OutputKind::Docx => format!("{stem}-document-content.docx"),
            OutputKind::Optimize => format!("{stem}-optimized.pdf"),
            OutputKind::Print => "Folio print snapshot".into(),
        };
        let job_id = self
            .outputs
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Output identifiers are exhausted. Restart Folio."))?;
        let plan = OutputPlan {
            job_id,
            document_id: request.document_id,
            revision: request.revision,
            kind: request.kind,
            pages: request.pages,
            dpi: request.dpi,
            quality: request.quality,
            suggested_name,
            source_bytes: info.byte_length,
            password_protected: info.password_protected,
        };
        self.outputs.next_id = job_id;
        self.outputs.jobs.insert(
            job_id,
            Job {
                plan: plan.clone(),
                entries: Vec::new(),
                staged: 0,
                byte_count: 0,
                print_pixels: 0,
                optimized: None,
            },
        );
        Ok(plan)
    }

    pub fn check_output(&self, job_id: u64) -> Result<&Session> {
        let job = self.outputs.job(job_id)?;
        if self.has_redaction_job(job.plan.document_id) {
            return Err(invalid("Clear pending redaction output before exporting."));
        }
        self.ensure_export_ready(job.plan.document_id, job.plan.revision)
    }

    pub fn output_plan(&self, job_id: u64) -> Result<OutputPlan> {
        self.check_output(job_id)?;
        Ok(self.outputs.job(job_id)?.plan.clone())
    }

    pub fn output_render_bytes(&self, job_id: u64) -> Result<Vec<u8>> {
        self.check_output(job_id)?;
        if self.outputs.job(job_id)?.plan.kind == OutputKind::Optimize {
            return Err(invalid(
                "Optimization must use protected source bytes, not a renderer copy.",
            ));
        }
        let bytes = self.check_output(job_id)?.render_bytes()?;
        if self.outputs.job(job_id)?.plan.kind == OutputKind::Docx
            && bytes.len() > crate::conversion::MAX_DOCX_SOURCE_BYTES
        {
            return Err(invalid("DOCX conversion requires a renderer snapshot no larger than 64 MiB."));
        }
        Ok(bytes)
    }

    pub fn stage_output_page(&mut self, job_id: u64, index: usize, bytes: Vec<u8>) -> Result<()> {
        self.check_output(job_id)?;
        let job = self.outputs.job_mut(job_id)?;
        if job.plan.kind == OutputKind::Optimize
            || index != job.staged
            || index >= job.plan.pages.len()
        {
            return Err(invalid(
                "Output pages must be prepared once, sequentially, in the selected order.",
            ));
        }
        let mut pixels = 0;
        if job.plan.kind == OutputKind::Docx {
            crate::conversion::validate_docx_text(&bytes)?;
        } else if job.plan.kind == OutputKind::Text {
            if bytes.len() > MAX_TEXT_BYTES || std::str::from_utf8(&bytes).is_err() {
                return Err(invalid(
                    "Text export requires valid UTF-8 within the 16 MiB text limit.",
                ));
            }
        } else {
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err(invalid(
                    "A rendered page exceeds the 64 MiB image limit. Choose a lower resolution.",
                ));
            }
            let (width, height) = image_dimensions(job.plan.kind, &bytes)?;
            pixels = u64::from(width) * u64::from(height);
            if width == 0
                || height == 0
                || width > MAX_PAGE_DIMENSION
                || height > MAX_PAGE_DIMENSION
                || pixels > MAX_PAGE_PIXELS
            {
                return Err(invalid(
                    "A page exceeds 24 megapixels or 16,384 pixels per dimension. Choose a lower resolution.",
                ));
            }
            if job.plan.kind == OutputKind::Print && job.print_pixels + pixels > MAX_PRINT_PIXELS {
                return Err(invalid(
                    "The print snapshot exceeds 100 megapixels. Choose a lower resolution or a smaller page range.",
                ));
            }
        }
        let overhead = if job.plan.kind == OutputKind::Docx {
            0
        } else if job.plan.kind == OutputKind::Text {
            if job.staged == 0 { 0 } else { 3 }
        } else {
            160
        };
        let byte_count = job
            .byte_count
            .checked_add(bytes.len() + overhead)
            .ok_or_else(|| invalid("The output byte count is too large."))?;
        let limit = if job.plan.kind == OutputKind::Docx {
            crate::conversion::MAX_DOCX_TEXT_BYTES
        } else if job.plan.kind == OutputKind::Text {
            MAX_TEXT_BYTES
        } else {
            MAX_DOCUMENT_BYTES
        };
        if byte_count > limit {
            return Err(invalid(if job.plan.kind == OutputKind::Docx {
                "The selected DOCX content exceeds the 8 MiB text limit. Export fewer pages."
            } else if job.plan.kind == OutputKind::Text {
                "The selected text exceeds the 16 MiB text limit. Export fewer pages."
            } else {
                "The prepared output exceeds 128 MiB. Choose fewer pages or a lower resolution/quality."
            }));
        }
        job.byte_count = byte_count;
        job.print_pixels += pixels;
        job.staged += 1;
        if job.plan.kind != OutputKind::Print {
            job.entries.push(bytes);
        }
        Ok(())
    }

    pub fn prepare_optimized_output(&mut self, job_id: u64) -> Result<OptimizationReport> {
        self.check_output(job_id)?;
        if self.outputs.job(job_id)?.plan.kind != OutputKind::Optimize {
            return Err(invalid("This output job is not an optimization request."));
        }
        let optimized = self.check_output(job_id)?.optimized_copy()?;
        let report = optimized.report.clone();
        self.outputs.job_mut(job_id)?.optimized = Some(optimized);
        Ok(report)
    }

    pub fn check_output_ready(&self, job_id: u64) -> Result<OutputPlan> {
        self.check_output(job_id)?;
        let job = self.outputs.job(job_id)?;
        if job.plan.kind == OutputKind::Optimize {
            if job.optimized.is_none() {
                return Err(invalid(
                    "Prepare and review the optimization result before saving a copy.",
                ));
            }
        } else if job.staged != job.plan.pages.len() {
            return Err(invalid(
                "The output is incomplete. No partial copy was saved.",
            ));
        } else if matches!(job.plan.kind, OutputKind::Text | OutputKind::Docx)
            && job.entries.iter().all(|entry| {
                std::str::from_utf8(entry)
                    .expect("UTF-8 checked")
                    .trim()
                    .is_empty()
            })
        {
            return Err(invalid(if job.plan.kind == OutputKind::Docx {
                "No editable text was found on these pages. Image-only scans require a separate OCR operation."
            } else {
                "No selectable text was found on these pages. Image-only pages need OCR."
            }));
        }
        Ok(job.plan.clone())
    }

    pub fn output_bytes(&self, job_id: u64) -> Result<Vec<u8>> {
        let plan = self.check_output_ready(job_id)?;
        let job = self.outputs.job(job_id)?;
        match plan.kind {
            OutputKind::Print => Err(invalid(
                "Print snapshots cannot be saved through the document-save command.",
            )),
            OutputKind::Optimize => Ok(job
                .optimized
                .as_ref()
                .expect("readiness checked")
                .bytes
                .clone()),
            OutputKind::Text => {
                let mut output = Vec::with_capacity(job.byte_count);
                for (index, entry) in job.entries.iter().enumerate() {
                    if index != 0 {
                        output.extend_from_slice(b"\n\x0c\n");
                    }
                    output.extend_from_slice(entry);
                }
                Ok(output)
            }
            OutputKind::Docx => crate::conversion::docx(&job.entries),
            OutputKind::Png | OutputKind::Jpeg if plan.pages.len() == 1 => {
                Ok(job.entries[0].clone())
            }
            OutputKind::Png | OutputKind::Jpeg => image_zip(&plan, &job.entries),
        }
    }

    pub fn write_output_copy(&self, job_id: u64, path: &Path) -> Result<usize> {
        let plan = self.check_output_ready(job_id)?;
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case(plan.kind.extension(plan.pages.len())))
        {
            return Err(invalid(
                "Use the filename extension selected for this output.",
            ));
        }
        let session = self.check_output(job_id)?;
        self.ensure_output_available(session.id, path)?;
        session.ensure_separate_output(path)?;
        let bytes = self.output_bytes(job_id)?;
        write_new_copy(path, &bytes)?;
        Ok(bytes.len())
    }

    pub fn cancel_output(&mut self, job_id: u64) -> bool {
        self.outputs.jobs.remove(&job_id).is_some()
    }
}

fn validate_request(session: &Session, request: &OutputRequest) -> Result<()> {
    let info = session.info()?;
    let permissions = session.permissions();
    if request.kind == OutputKind::Optimize {
        session.ensure_writable()?;
        if !request.pages.is_empty() || request.dpi.is_some() || request.quality.is_some() {
            return Err(invalid(
                "Lossless optimization applies to the full PDF without raster settings.",
            ));
        }
        return Ok(());
    }
    if request.kind == OutputKind::Docx && info.byte_length > crate::conversion::MAX_DOCX_SOURCE_BYTES {
        return Err(invalid("DOCX conversion requires a source snapshot no larger than 64 MiB."));
    }
    let max_pages = if request.kind == OutputKind::Docx {
        crate::conversion::MAX_DOCX_PAGES
    } else if request.kind == OutputKind::Print {
        MAX_PRINT_PAGES
    } else {
        MAX_OUTPUT_PAGES
    };
    if request.pages.is_empty() || request.pages.len() > max_pages {
        return Err(invalid(format!(
            "Select from 1 to {max_pages} pages for this output."
        )));
    }
    let mut used = HashSet::new();
    if request
        .pages
        .iter()
        .any(|page| *page >= info.page_count || !used.insert(*page))
    {
        return Err(invalid(
            "Output page numbers must be present in this PDF and must not repeat.",
        ));
    }
    if request.kind == OutputKind::Print {
        if !permissions.print_allowed {
            return Err(invalid(
                "This PDF does not allow printing. Reopen it with the owner password.",
            ));
        }
        if request.dpi.is_some_and(|dpi| dpi > 150) && !permissions.high_quality_print_allowed {
            return Err(invalid(
                "This PDF permits only low-resolution printing (up to 150 DPI).",
            ));
        }
    } else {
        if !permissions.copy_allowed {
            return Err(invalid(
                "This PDF does not allow copying text or images. Reopen it with the owner password.",
            ));
        }
        if info.password_protected && !request.allow_unencrypted {
            return Err(invalid(
                "Acknowledge that PNG, JPEG, ZIP, text and DOCX exports are unencrypted and retain neither PDF permissions nor cryptographic signatures.",
            ));
        }
    }
    if matches!(request.kind, OutputKind::Text | OutputKind::Docx) {
        if request.dpi.is_some() || request.quality.is_some() {
            return Err(invalid(
                "Text export does not accept raster resolution or image quality settings.",
            ));
        }
    } else if !matches!(request.dpi, Some(96 | 150 | 300)) {
        return Err(invalid("Choose 96, 150 or 300 DPI."));
    }
    if request.kind == OutputKind::Jpeg {
        if !request
            .quality
            .is_some_and(|quality| (60..=100).contains(&quality))
        {
            return Err(invalid("JPEG quality must be from 60 to 100."));
        }
    } else if request.quality.is_some() {
        return Err(invalid("Only JPEG exports accept a lossy quality setting."));
    }
    Ok(())
}

pub fn safe_stem(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or("");
    let leaf = if leaf.to_ascii_lowercase().ends_with(".pdf") {
        &leaf[..leaf.len() - 4]
    } else {
        leaf
    };
    let mut stem = leaf
        .chars()
        .take(80)
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '-' | '_') {
                value
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if stem.is_empty() || reserved.contains(&stem.to_ascii_uppercase().as_str()) {
        stem = "Folio".into();
    }
    stem
}

pub fn output_path(mut path: PathBuf, plan: &OutputPlan) -> Result<PathBuf> {
    let extension = plan.kind.extension(plan.pages.len());
    if path.extension().is_none() {
        path.set_extension(extension);
    } else if !path
        .extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        return Err(invalid(format!(
            "Use a .{extension} filename for this output."
        )));
    }
    Ok(path)
}

pub(crate) fn write_new_copy(path: &Path, bytes: &[u8]) -> Result<()> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(invalid(
            "This output never replaces existing files or folders. Choose a new filename.",
        ));
    }
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            invalid(
                "A file appeared at that filename. Choose a new filename; nothing was overwritten.",
            )
        } else {
            crate::document::Error::Io(error.error)
        }
    })?;
    Ok(())
}

fn image_dimensions(kind: OutputKind, bytes: &[u8]) -> Result<(u32, u32)> {
    if kind != OutputKind::Jpeg {
        let reader = png::Decoder::new(Cursor::new(bytes))
            .read_info()
            .map_err(|_| invalid("A rendered page is not a readable PNG image."))?;
        return Ok((reader.info().width, reader.info().height));
    }
    if !bytes.starts_with(&[0xff, 0xd8]) || !bytes.ends_with(&[0xff, 0xd9]) {
        return Err(invalid("A rendered page is not a complete JPEG image."));
    }
    let mut index = 2;
    while index + 4 <= bytes.len() {
        if bytes[index] != 0xff {
            return Err(invalid("The JPEG page has malformed image markers."));
        }
        while index < bytes.len() && bytes[index] == 0xff {
            index += 1;
        }
        let marker = *bytes
            .get(index)
            .ok_or_else(|| invalid("The JPEG page is incomplete."))?;
        index += 1;
        if marker == 0xda || marker == 0xd9 {
            break;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let length_bytes = bytes
            .get(index..index + 2)
            .ok_or_else(|| invalid("The JPEG page is incomplete."))?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 || index + length > bytes.len() {
            return Err(invalid("The JPEG page has an invalid segment length."));
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) && length >= 8 {
            let height = u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]);
            let width = u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]);
            return Ok((u32::from(width), u32::from(height)));
        }
        index += length;
    }
    Err(invalid("The JPEG page has no supported image dimensions."))
}

fn image_zip(plan: &OutputPlan, entries: &[Vec<u8>]) -> Result<Vec<u8>> {
    let extension = plan.kind.extension(1);
    let names: Vec<_> = plan.pages.iter()
        .map(|page| format!("page-{:04}.{extension}", page + 1))
        .collect();
    let entries: Vec<_> = names.iter().zip(entries)
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    stored_zip(&entries)
}

pub(crate) fn stored_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    if entries.len() > u16::MAX as usize
        || entries.iter().any(|(name, bytes)| !name.is_ascii()
            || name.len() > u16::MAX as usize || bytes.len() > MAX_DOCUMENT_BYTES)
        || entries.iter().map(|(name, bytes)| bytes.len() + name.len() * 2 + 98).sum::<usize>() + 22 > MAX_DOCUMENT_BYTES
    {
        return Err(invalid("The ZIP exceeds the bounded 128 MiB output format."));
    }
    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in entries {
        let offset = output.len() as u32;
        let length = bytes.len() as u32;
        let crc = crc32fast::hash(bytes);
        output.extend(0x04034b50_u32.to_le_bytes());
        for value in [20_u16, 0, 0, 0, 33] {
            output.extend(value.to_le_bytes());
        }
        for value in [crc, length, length] {
            output.extend(value.to_le_bytes());
        }
        output.extend((name.len() as u16).to_le_bytes());
        output.extend(0_u16.to_le_bytes());
        output.extend(name.as_bytes());
        output.extend_from_slice(bytes);
        central.extend(0x02014b50_u32.to_le_bytes());
        for value in [20_u16, 20, 0, 0, 0, 33] {
            central.extend(value.to_le_bytes());
        }
        for value in [crc, length, length] {
            central.extend(value.to_le_bytes());
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend(value.to_le_bytes());
        }
        for value in [0_u32, offset] {
            central.extend(value.to_le_bytes());
        }
        central.extend(name.as_bytes());
    }
    let offset = output.len() as u32;
    let length = central.len() as u32;
    output.extend(central);
    output.extend(0x06054b50_u32.to_le_bytes());
    for value in [0_u16, 0, entries.len() as u16, entries.len() as u16] {
        output.extend(value.to_le_bytes());
    }
    for value in [length, offset] {
        output.extend(value.to_le_bytes());
    }
    output.extend(0_u16.to_le_bytes());
    if output.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid(
            "The ZIP exceeds the 128 MiB output limit. Export fewer pages.",
        ));
    }
    Ok(output)
}

pub(crate) fn optimize(source: &Document, original: &[u8]) -> Result<OptimizedCopy> {
    if crate::signatures::has_signatures(source) {
        return Err(invalid(
            "Signed PDFs cannot be optimized: a rewrite would invalidate their signatures.",
        ));
    }
    if source.objects.len() > MAX_OPTIMIZE_OBJECTS {
        return Err(invalid(
            "Safe optimization is limited to 200,000 PDF objects.",
        ));
    }
    let reachable = reachable_objects(source)?;
    let mut output = source.clone();
    let removed_objects = output.objects.len() - reachable.len();
    output.objects.retain(|id, _| reachable.contains(id));
    let mut compressed_streams = 0;
    let mut stream_bytes = 0_usize;
    for object in output.objects.values_mut() {
        if let Object::Stream(stream) = object {
            stream_bytes = stream_bytes
                .checked_add(stream.content.len())
                .ok_or_else(|| invalid("The PDF stream byte count is too large."))?;
            if stream_bytes > MAX_DOCUMENT_BYTES {
                return Err(invalid(
                    "Safe optimization is limited to 128 MiB of encoded stream data.",
                ));
            }
            // Existing filters, predictors, image codecs and stream bytes are never decoded or replaced.
            if stream.allows_compression
                && !stream.dict.has(b"Filter")
                && !stream.dict.has(b"DecodeParms")
            {
                stream.compress()?;
                if stream.dict.has(b"Filter") {
                    compressed_streams += 1;
                }
            }
        }
    }
    let candidate = crate::protection::serialize(&output)?;
    let reduced = candidate.len() < original.len();
    let report = OptimizationReport {
        before_bytes: original.len(),
        candidate_bytes: candidate.len(),
        after_bytes: if reduced {
            candidate.len()
        } else {
            original.len()
        },
        reduced,
        compressed_streams: if reduced { compressed_streams } else { 0 },
        removed_objects: if reduced { removed_objects } else { 0 },
    };
    Ok(OptimizedCopy {
        bytes: if reduced {
            candidate
        } else {
            original.to_vec()
        },
        report,
    })
}

fn reachable_objects(document: &Document) -> Result<HashSet<ObjectId>> {
    let mut visited = HashSet::new();
    let mut stack = document
        .trailer
        .iter()
        .map(|(_, object)| (object, 0_usize))
        .collect::<Vec<_>>();
    let mut inspected = 0_usize;
    while let Some((object, depth)) = stack.pop() {
        inspected += 1;
        if depth > 128 || inspected > 2_000_000 {
            return Err(invalid(
                "This PDF is too deeply nested or complex for safe optimization.",
            ));
        }
        match object {
            Object::Reference(id) if visited.insert(*id) => {
                let object = document.objects.get(id)
                    .ok_or_else(|| invalid("This PDF has an unresolved object reference; no optimized copy was created."))?;
                stack.push((object, 0));
            }
            Object::Array(array) => stack.extend(array.iter().map(|object| (object, depth + 1))),
            Object::Dictionary(dictionary) => {
                stack.extend(dictionary.iter().map(|(_, object)| (object, depth + 1)))
            }
            Object::Stream(stream) => {
                stack.extend(stream.dict.iter().map(|(_, object)| (object, depth + 1)))
            }
            _ => {}
        }
    }
    Ok(visited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn optimizer_checks_signature_objects_independently_of_ui_or_session_read_only_state() {
        let bytes = Session::blank(72.0, 72.0, 1).unwrap().bytes().to_vec();
        let mut document = Document::load_mem(&bytes).unwrap();
        document.add_object(dictionary! {
            "Type" => "Sig", "Contents" => Object::string_literal("Synthetic marker only"),
        });
        let result = optimize(&document, &bytes);
        assert!(result.is_err_and(|error| error.to_string().contains("Signed PDFs")));
    }
}
