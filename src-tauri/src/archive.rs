use std::{
    io::Cursor,
    path::Path,
    time::{Duration, Instant},
};

use lopdf::{
    Document, Object, ObjectId, Stream,
    content::{Content, Operation},
    dictionary,
};
use serde::{Deserialize, Serialize};

use crate::{
    document::{MAX_DOCUMENT_BYTES, Result, Session, invalid},
    page_geometry::PageGeometry,
    pdf_objects::resolve,
    workspace::Workspace,
};

pub const MAX_PAGES: usize = 100;
pub const MAX_PAGE_PIXELS: u64 = 24_000_000;
pub const MAX_TOTAL_PIXELS: u64 = 100_000_000;
pub const MAX_DIMENSION: u32 = 16_384;
pub const MAX_PNG_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: usize = MAX_DOCUMENT_BYTES;
pub const MAX_JOB_SECONDS: u64 = 900;
pub const PROFILE_SHA256: &str = "384b832de3412066743b52a75ee906b6fb9fb8d9e09e936fc2c43223815c6e0a";
const PROFILE: &[u8] = include_bytes!("../resources/color/sRGB2014.icc");
const MARKER: &[u8] = b"FolioRasterArchive";
pub const DISCLOSURE: &str = "A fresh, unsigned, unencrypted, image-only PDF/A-2b archival copy at the selected DPI. All selected pages are rasterized on white in sRGB. Original text, fonts, vectors, search/selection, tags/accessibility structure, form fields/interactivity, attachments, active code, original metadata/history and cryptographic signatures are not retained. This is not OCR, accessibility remediation, automatic redaction, or validation of the source PDF. Review appearances before relying on the copy. Folio opens its archival copies read-only; edit the original and export again.";
const XMP: &str = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\
<rdf:Description rdf:about=\"\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">\
<pdfaid:part>2</pdfaid:part><pdfaid:conformance>B</pdfaid:conformance>\
</rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRef {
    pub document_id: u64,
    pub revision: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    #[serde(flatten)]
    pub source: SourceRef,
    pub pages: Vec<usize>,
    pub dpi: u16,
    pub acknowledge_loss: bool,
    #[serde(default)]
    pub allow_unencrypted: bool,
    #[serde(default)]
    pub acknowledge_unsigned: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePlan {
    pub source_page: usize,
    pub geometry: PageGeometry,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub archive_id: u64,
    #[serde(flatten)]
    pub source: SourceRef,
    pub epoch: u64,
    pub dpi: u16,
    pub source_page_count: usize,
    pub pages: Vec<PagePlan>,
    pub suggested_name: String,
    pub password_protected: bool,
    pub signed_source: bool,
    pub profile_sha256: &'static str,
    pub disclosure: &'static str,
}

#[derive(Default)]
pub(crate) struct ArchiveJobs {
    next_id: u64,
    job: Option<Job>,
}

impl ArchiveJobs {
    pub(crate) fn invalidate(&mut self, document_id: u64) {
        if self.has_document(document_id) {
            self.job = None;
        }
    }

    pub(crate) fn has_document(&self, document_id: u64) -> bool {
        self.job
            .as_ref()
            .is_some_and(|job| job.plan.source.document_id == document_id)
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.job.is_some()
    }

    fn get(&self, archive_id: u64) -> Result<&Job> {
        self.job.as_ref().filter(|job| job.plan.archive_id == archive_id)
            .ok_or_else(|| invalid("This archival job was canceled or invalidated by a source edit, draft, staged image or closed tab. Prepare the current PDF again."))
    }
}

impl Workspace {
    pub fn ensure_technical_ready(&self, id: u64, revision: u64) -> Result<&Session> {
        let session = self.ensure_export_ready(id, revision)?;
        if self.has_redaction_job(id) {
            return Err(invalid(
                "Clear pending redaction marks before technical inspection or archival export. An archival copy is not a redaction workflow.",
            ));
        }
        if !session.permissions().copy_allowed {
            return Err(invalid(
                "This PDF does not allow copying/extraction. Reopen it with the owner password for technical inspection or archival export.",
            ));
        }
        Ok(session)
    }

    pub fn begin_archive(&mut self, request: Request) -> Result<Plan> {
        if self.archives.is_pending() {
            return Err(invalid(
                "Finish or cancel the previous archival job. Only one archival copy is retained.",
            ));
        }
        let session =
            self.ensure_technical_ready(request.source.document_id, request.source.revision)?;
        if !request.acknowledge_loss {
            return Err(invalid(
                "Acknowledge rasterization and the loss of text, vectors, accessibility, interactivity and signatures before creating an archival copy.",
            ));
        }
        let info = session.info()?;
        if info.password_protected
            && (!request.allow_unencrypted || !session.permissions().owner_authenticated)
        {
            return Err(invalid(
                "PDF/A cannot be encrypted. Reopen this PDF with its actual owner password AND acknowledge an unencrypted archival copy. Opening-password access, even with permissive rights, is not ownership.",
            ));
        }
        let signed_source = session.is_signed();
        if signed_source && !request.acknowledge_unsigned {
            return Err(invalid(
                "This source is signed/certified. Explicitly acknowledge a fresh unsigned raster copy; no signature or certification is preserved.",
            ));
        }
        let archive_id = self
            .archives
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Archival identifiers are exhausted. Restart Folio."))?;
        let pages = page_plans(
            &info
                .pages
                .iter()
                .map(|page| page.geometry.clone())
                .collect::<Vec<_>>(),
            &request.pages,
            request.dpi,
        )?;
        session.check_archive_appearances()?;
        validate_profile(PROFILE)?;
        let plan = Plan {
            archive_id,
            source: request.source,
            epoch: self.technical_epoch(info.id),
            dpi: request.dpi,
            source_page_count: info.page_count,
            pages,
            suggested_name: format!(
                "{}-archival-{}dpi.pdf",
                crate::output::safe_stem(&info.name),
                request.dpi
            ),
            password_protected: info.password_protected,
            signed_source,
            profile_sha256: PROFILE_SHA256,
            disclosure: DISCLOSURE,
        };
        self.archives.next_id = archive_id;
        self.archives.job = Some(Job::new(plan.clone())?);
        Ok(plan)
    }

    pub fn check_archive(&self, archive_id: u64) -> Result<Plan> {
        let job = self.archives.get(archive_id)?;
        if job.started.elapsed() > Duration::from_secs(MAX_JOB_SECONDS) {
            return Err(invalid(
                "The archival job exceeded 15 minutes. Cancel it and prepare the current PDF again.",
            ));
        }
        self.ensure_technical_ready(job.plan.source.document_id, job.plan.source.revision)?;
        if self.technical_epoch(job.plan.source.document_id) != job.plan.epoch {
            return Err(invalid(
                "The archival source epoch changed. Prepare the current PDF again.",
            ));
        }
        Ok(job.plan.clone())
    }

    pub fn archive_render_bytes(&self, archive_id: u64) -> Result<Vec<u8>> {
        let plan = self.check_archive(archive_id)?;
        let bytes = self
            .ensure_technical_ready(plan.source.document_id, plan.source.revision)?
            .render_bytes()?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid(
                "The archival renderer snapshot exceeds 128 MiB. No archival copy was created.",
            ));
        }
        self.check_archive(archive_id)?;
        Ok(bytes)
    }

    pub fn stage_archive_page(&mut self, archive_id: u64, index: usize, png: &[u8]) -> Result<()> {
        self.check_archive(archive_id)?;
        self.archives
            .job
            .as_mut()
            .expect("archive checked")
            .stage(index, png)
    }

    pub fn archive_bytes(&mut self, archive_id: u64) -> Result<Vec<u8>> {
        self.check_archive(archive_id)?;
        let bytes = self
            .archives
            .job
            .as_mut()
            .expect("archive checked")
            .bytes()?;
        self.check_archive(archive_id)?;
        Ok(bytes)
    }

    pub fn write_archive_copy(&mut self, archive_id: u64, path: &Path) -> Result<usize> {
        let plan = self.check_archive(archive_id)?;
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case("pdf"))
        {
            return Err(invalid("Use a .pdf filename for the archival copy."));
        }
        self.ensure_output_available(plan.source.document_id, path)?;
        self.ensure_technical_ready(plan.source.document_id, plan.source.revision)?
            .ensure_separate_output(path)?;
        let bytes = self.archive_bytes(archive_id)?;
        self.check_archive(archive_id)?;
        self.ensure_output_available(plan.source.document_id, path)?;
        self.ensure_technical_ready(plan.source.document_id, plan.source.revision)?
            .ensure_separate_output(path)?;
        crate::output::write_new_copy(path, &bytes)?;
        Ok(bytes.len())
    }

    pub fn cancel_archive(&mut self, archive_id: u64) -> bool {
        if self
            .archives
            .job
            .as_ref()
            .is_some_and(|job| job.plan.archive_id == archive_id)
        {
            self.archives.job = None;
            true
        } else {
            false
        }
    }
}

pub fn page_plans(
    geometries: &[PageGeometry],
    selected: &[usize],
    dpi: u16,
) -> Result<Vec<PagePlan>> {
    if ![150, 300].contains(&dpi) {
        return Err(invalid(
            "Choose exactly 150 or 300 DPI. Resolution is never lowered automatically.",
        ));
    }
    if selected.is_empty() || selected.len() > MAX_PAGES {
        return Err(invalid(
            "Select from 1 to 100 source pages for an archival copy.",
        ));
    }
    if selected.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(invalid(
            "Archival source pages must be unique and in ascending document order.",
        ));
    }
    let mut total = 0_u64;
    selected.iter().map(|index| {
        let geometry = geometries.get(*index).ok_or_else(|| invalid("An archival page is outside the source document."))?;
        let width = (f64::from(geometry.width_points) * f64::from(dpi) / 72.0).ceil();
        let height = (f64::from(geometry.height_points) * f64::from(dpi) / 72.0).ceil();
        if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0
            || width > f64::from(MAX_DIMENSION) || height > f64::from(MAX_DIMENSION)
            || width * height > MAX_PAGE_PIXELS as f64
            || geometry.width_points > 14_400.0 || geometry.height_points > 14_400.0
        {
            return Err(invalid(format!("Page {} exceeds 24 megapixels, 16,384 pixels per dimension, or the fresh PDF's 14,400-point page limit at {dpi} DPI. Choose 150 DPI or a smaller explicit range; no pages were skipped.", index + 1)));
        }
        total += width as u64 * height as u64;
        if total > MAX_TOTAL_PIXELS {
            return Err(invalid("The selected archival pages exceed 100 megapixels in total. Choose a smaller explicit range or 150 DPI; no quality reduction or page skipping is automatic."));
        }
        Ok(PagePlan { source_page: *index, geometry: geometry.clone(), pixel_width: width as u32, pixel_height: height as u32 })
    }).collect()
}

pub(crate) fn is_folio_archive(document: &Document) -> bool {
    document.catalog().is_ok_and(|catalog| {
        catalog
            .get(MARKER)
            .and_then(Object::as_bool)
            .is_ok_and(|value| value)
    })
}

pub(crate) fn check_appearances(document: &Document) -> Result<()> {
    // This walk inspects dictionaries, never executes actions or decodes page content.
    crate::preflight::walk(document, |object| {
        let dictionary = match object {
            Object::Dictionary(value) => value,
            Object::Stream(value) => &value.dict,
            _ => return Ok(()),
        };
        if dictionary.has(b"XFA")
            || dictionary
                .get(b"NeedAppearances")
                .and_then(Object::as_bool)
                .is_ok_and(|value| value)
        {
            return Err(invalid(
                "XFA or forms requiring regenerated appearances cannot be archived reliably. Use a supported static PDF first.",
            ));
        }
        let subtype = dictionary
            .get(b"Subtype")
            .ok()
            .map(|value| resolve(document, value))
            .transpose()?
            .and_then(|value| value.as_name().ok());
        if matches!(
            subtype,
            Some(b"RichMedia" | b"3D" | b"Movie" | b"Sound" | b"Screen")
        ) {
            return Err(invalid(
                "This PDF contains multimedia/3D appearances not supported by archival rendering. No pages were exported.",
            ));
        }
        if subtype == Some(b"Widget") && !dictionary.has(b"AP") {
            return Err(invalid(
                "A form/signature widget has no persistent appearance. Archival export will not substitute HTML controls or silently omit a value.",
            ));
        }
        Ok(())
    })
}

fn validate_profile(profile: &[u8]) -> Result<()> {
    let hash: String = openssl::sha::sha256(profile)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if profile.len() != 3024
        || hash != PROFILE_SHA256
        || &profile[36..40] != b"acsp"
        || &profile[16..20] != b"RGB "
        || &profile[12..16] != b"mntr"
        || profile[8] != 2
    {
        return Err(invalid(
            "The bundled ICC sRGB2014 profile is missing, changed or incompatible. No PDF/A copy was generated.",
        ));
    }
    Ok(())
}

struct Job {
    plan: Plan,
    started: Instant,
    document: Document,
    pages_id: ObjectId,
    profile_id: ObjectId,
    page_ids: Vec<Object>,
    compressed_bytes: usize,
}

impl Job {
    fn new(plan: Plan) -> Result<Self> {
        validate_profile(PROFILE)?;
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let profile_id =
            document.add_object(Stream::new(dictionary! { "N" => 3 }, PROFILE.to_vec()));
        let intent = document.add_object(dictionary! {
            "Type" => "OutputIntent", "S" => "GTS_PDFA1",
            "OutputConditionIdentifier" => Object::string_literal("sRGB2014"),
            "Info" => Object::string_literal("sRGB IEC61966-2.1"),
            "RegistryName" => Object::string_literal("http://www.color.org"),
            "DestOutputProfile" => profile_id,
        });
        let metadata = document.add_object(Stream::new(
            dictionary! { "Type" => "Metadata", "Subtype" => "XML" },
            XMP.as_bytes().to_vec(),
        ));
        let root = document.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => pages_id, "Metadata" => metadata,
            "OutputIntents" => vec![Object::Reference(intent)], "FolioRasterArchive" => true,
        });
        document.trailer.set("Root", root);
        let mut identifier = [0; 16];
        getrandom::fill(&mut identifier).map_err(|_| {
            invalid("Operating-system randomness is unavailable. No archival copy was generated.")
        })?;
        document.trailer.set(
            "ID",
            vec![
                Object::string_literal(identifier),
                Object::string_literal(identifier),
            ],
        );
        Ok(Self {
            plan,
            started: Instant::now(),
            document,
            pages_id,
            profile_id,
            page_ids: Vec::new(),
            compressed_bytes: 0,
        })
    }

    fn stage(&mut self, index: usize, png: &[u8]) -> Result<()> {
        if index != self.page_ids.len() {
            return Err(invalid(
                "Archival pages must be rendered exactly once in selected source order.",
            ));
        }
        let page = self
            .plan
            .pages
            .get(index)
            .ok_or_else(|| invalid("That page is outside the archival plan."))?;
        let rgb = decode_rgb(png, page)?;
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => i64::from(page.pixel_width), "Height" => i64::from(page.pixel_height),
                "ColorSpace" => vec![Object::Name(b"ICCBased".to_vec()), Object::Reference(self.profile_id)],
                "BitsPerComponent" => 8, "Interpolate" => false, "Intent" => "RelativeColorimetric",
            },
            rgb,
        );
        image.compress()?;
        if self.compressed_bytes + image.content.len() > MAX_OUTPUT_BYTES - 1024 * 1024 {
            return Err(invalid(
                "The archival copy exceeds its 128 MiB output budget. Choose 150 DPI or a smaller explicit range.",
            ));
        }
        let width = page.geometry.width_points;
        let height = page.geometry.height_points;
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        width.into(),
                        0.into(),
                        0.into(),
                        height.into(),
                        0.into(),
                        0.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"PageImage".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()?;
        self.compressed_bytes += image.content.len();
        let image_id = self.document.add_object(image);
        let contents = self
            .document
            .add_object(Stream::new(dictionary! {}, content));
        let page_id = self.document.add_object(dictionary! {
            "Type" => "Page", "Parent" => self.pages_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(width), Object::Real(height)],
            "Resources" => dictionary! { "XObject" => dictionary! { "PageImage" => image_id } },
            "Contents" => contents,
        });
        self.page_ids.push(page_id.into());
        Ok(())
    }

    fn bytes(&mut self) -> Result<Vec<u8>> {
        if self.page_ids.len() != self.plan.pages.len() {
            return Err(invalid(
                "Every requested source page must finish before the archival copy can be exported.",
            ));
        }
        self.document.objects.insert(self.pages_id, dictionary! {
            "Type" => "Pages", "Kids" => self.page_ids.clone(), "Count" => self.page_ids.len() as i64,
        }.into());
        crate::protection::serialize(&self.document)
    }
}

fn decode_rgb(bytes: &[u8], page: &PagePlan) -> Result<Vec<u8>> {
    if bytes.len() > MAX_PNG_BYTES {
        return Err(invalid("An archival page exceeds 64 MiB of encoded PNG."));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_limits(png::Limits {
        bytes: MAX_PAGE_PIXELS as usize * 4 + 1024 * 1024,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|error| invalid(format!("Invalid archival page PNG: {error}")))?;
    let info = reader.info();
    if info.width != page.pixel_width
        || info.height != page.pixel_height
        || info.animation_control.is_some()
        || info.bit_depth != png::BitDepth::Eight
        || !matches!(info.color_type, png::ColorType::Rgb | png::ColorType::Rgba)
    {
        return Err(invalid(
            "Archival pages require static, 8-bit, opaque sRGB RGB/RGBA PNG pixels at the exact reviewed dimensions.",
        ));
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_PAGE_PIXELS as usize * 4)
        .ok_or_else(|| invalid("The archival page exceeds its decoding budget."))?;
    let mut decoded = vec![0; size];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|error| invalid(format!("The archival PNG could not be decoded: {error}")))?;
    let pixels = &decoded[..output.buffer_size()];
    if output.color_type == png::ColorType::Rgb {
        return Ok(pixels.to_vec());
    }
    let mut rgb = Vec::with_capacity(page.pixel_width as usize * page.pixel_height as usize * 3);
    for pixel in pixels.chunks_exact(4) {
        if pixel[3] != 255 {
            return Err(invalid(
                "Archival pixels must already be opaque on white; transparent pixels are not accepted.",
            ));
        }
        rgb.extend_from_slice(&pixel[..3]);
    }
    Ok(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_changed_profiles_fail_closed() {
        assert!(validate_profile(&[]).is_err());
        let mut bytes = PROFILE.to_vec();
        bytes[16] ^= 1;
        assert!(validate_profile(&bytes).is_err());
        validate_profile(PROFILE).unwrap();
    }
}
