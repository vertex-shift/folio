use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    document::{Result, invalid},
    page_geometry::PageGeometry,
    workspace::Workspace,
};

pub const MAX_PAGES: usize = 100;
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PAGE_PIXELS: u64 = 8_000_000;
pub const MAX_TOTAL_PIXELS: u64 = 100_000_000;
pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_PAGE_TEXT_BYTES: usize = 256 * 1024;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PAGE_TOKENS: usize = 20_000;
pub const MAX_TOKENS: usize = 100_000;
pub const MAX_SPANS: usize = 4000;
pub const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;

pub const SCOPE: &str = "Page-index matching only: embedded text in PDF.js extraction order (including invisible/off-crop text it returns), visible page pixels on white at the selected DPI/RGB tolerance, and MediaBox/effective CropBox/rotation/UserUnit geometry. Only unmatched trailing indexes are labeled added/removed; middle insertions shift later pairings. No OCR, semantic reflow, moved-page matching, document identity, metadata, bookmarks, other hidden PDF objects, attachments, signature validity or trust comparison. Form/annotation appearances are included only as rendered. Matching text/pixels never proves that the PDFs are identical.";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceRef {
    pub document_id: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub dpi: u16,
    pub tolerance: u8,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub before: SourceRef,
    pub after: SourceRef,
    pub settings: Settings,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Before,
    After,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(flatten)]
    pub reference: SourceRef,
    pub name: String,
    pub page_count: usize,
    pub source_bytes: usize,
    pub password_protected: bool,
    pub pages: Vec<PageGeometry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagePlan {
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub comparison_id: u64,
    pub before: Source,
    pub after: Source,
    pub settings: Settings,
    pub pages: Vec<PagePlan>,
    pub scope: &'static str,
    pub suggested_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    Equal,
    Added,
    Removed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextSpan {
    pub kind: SpanKind,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextResult {
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub spans: Vec<TextSpan>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PixelResult {
    pub width: u32,
    pub height: u32,
    pub changed: u64,
    pub total: u64,
    pub max_channel_delta: u8,
    pub bounds: Option<[u32; 4]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageResult {
    pub index: usize,
    pub text: TextResult,
    pub pixels: PixelResult,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportPage<'a> {
    presence: &'static str,
    geometry_changes: Vec<&'static str>,
    before_geometry: Option<&'a PageGeometry>,
    after_geometry: Option<&'a PageGeometry>,
    text_changed: bool,
    #[serde(flatten)]
    result: &'a PageResult,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'a> {
    schema: &'static str,
    schema_version: u8,
    matching: &'static str,
    scope: &'static str,
    protection_notice: &'static str,
    conclusion: &'static str,
    plan: &'a Plan,
    pages: Vec<ReportPage<'a>>,
}

struct Job {
    plan: Plan,
    report: Option<Vec<u8>>,
}

#[derive(Default)]
pub(crate) struct ComparisonJobs {
    next_id: u64,
    job: Option<Job>,
}

impl ComparisonJobs {
    pub(crate) fn invalidate(&mut self, id: u64) {
        if self.job.as_ref().is_some_and(|job| {
            job.plan.before.reference.document_id == id
                || job.plan.after.reference.document_id == id
        }) {
            self.job = None;
        }
    }

    fn get(&self, id: u64) -> Result<&Job> {
        self.job
            .as_ref()
            .filter(|job| job.plan.comparison_id == id)
            .ok_or_else(|| invalid("This comparison was canceled or invalidated by a source change, draft or closed tab. Compare the current PDFs again."))
    }
}

impl Workspace {
    pub fn begin_comparison(&mut self, request: Request) -> Result<Plan> {
        if self.comparisons.job.is_some() {
            return Err(invalid(
                "Clear the previous comparison before starting another. Only one comparison is retained.",
            ));
        }
        if request.before.document_id == request.after.document_id {
            return Err(invalid("Choose two different open PDF tabs."));
        }
        validate_settings(&request.settings)?;
        let before = self.comparison_source(&request.before)?;
        let after = self.comparison_source(&request.after)?;
        let pages = page_plans(&before.pages, &after.pages, request.settings.dpi)?;
        let comparison_id = self
            .comparisons
            .next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Comparison identifiers are exhausted. Restart Folio."))?;
        let plan = Plan {
            comparison_id,
            before,
            after,
            settings: request.settings,
            pages,
            scope: SCOPE,
            suggested_name: "Folio-comparison.json".into(),
        };
        self.comparisons.next_id = comparison_id;
        self.comparisons.job = Some(Job {
            plan: plan.clone(),
            report: None,
        });
        Ok(plan)
    }

    fn comparison_session(&self, reference: &SourceRef) -> Result<&crate::document::Session> {
        let session = self.ensure_export_ready(reference.document_id, reference.revision)?;
        if !session.permissions().copy_allowed {
            return Err(invalid(
                "Both PDFs must permit copying/extraction to compare them. Reopen the restricted PDF with its owner password.",
            ));
        }
        if self.has_redaction_job(reference.document_id) {
            return Err(invalid(
                "Clear pending redaction marks on both PDFs before comparing their content.",
            ));
        }
        Ok(session)
    }

    fn comparison_source(&self, reference: &SourceRef) -> Result<Source> {
        let session = self.comparison_session(reference)?;
        let info = session.info()?;
        source_limits(info.page_count, info.byte_length)?;
        Ok(Source {
            reference: reference.clone(),
            name: info.name,
            page_count: info.page_count,
            source_bytes: info.byte_length,
            password_protected: info.password_protected,
            pages: info.pages.into_iter().map(|page| page.geometry).collect(),
        })
    }

    pub fn check_comparison(&self, comparison_id: u64) -> Result<Plan> {
        let job = self.comparisons.get(comparison_id)?;
        for source in [&job.plan.before, &job.plan.after] {
            self.comparison_session(&source.reference)?;
        }
        Ok(job.plan.clone())
    }

    pub fn comparison_render_bytes(&self, comparison_id: u64, side: Side) -> Result<Vec<u8>> {
        let plan = self.check_comparison(comparison_id)?;
        let source = match side {
            Side::Before => plan.before,
            Side::After => plan.after,
        };
        let bytes = self
            .ensure_export_ready(source.reference.document_id, source.reference.revision)?
            .render_bytes()?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(invalid(
                "The decrypted comparison snapshot exceeds 64 MiB. No content was compared.",
            ));
        }
        self.check_comparison(comparison_id)?;
        Ok(bytes)
    }

    pub fn finish_comparison(&mut self, comparison_id: u64, pages: Vec<PageResult>) -> Result<()> {
        let plan = self.check_comparison(comparison_id)?;
        let bytes = report_bytes(&plan, &pages)?;
        self.check_comparison(comparison_id)?;
        self.comparisons
            .job
            .as_mut()
            .expect("comparison checked")
            .report = Some(bytes);
        Ok(())
    }

    pub fn comparison_report(
        &self,
        comparison_id: u64,
        allow_unencrypted: bool,
    ) -> Result<Vec<u8>> {
        let plan = self.check_comparison(comparison_id)?;
        if (plan.before.password_protected || plan.after.password_protected) && !allow_unencrypted {
            return Err(invalid(
                "Acknowledge that the comparison report contains unencrypted extracted PDF content and does not retain passwords, permissions or signatures.",
            ));
        }
        self.comparisons
            .get(comparison_id)?
            .report
            .clone()
            .ok_or_else(|| invalid("Complete the comparison before exporting its report."))
    }

    pub fn write_comparison_report(
        &self,
        comparison_id: u64,
        allow_unencrypted: bool,
        path: &Path,
    ) -> Result<usize> {
        let plan = self.check_comparison(comparison_id)?;
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case("json"))
        {
            return Err(invalid("Use a .json filename for the comparison report."));
        }
        for source in [&plan.before, &plan.after] {
            self.ensure_output_available(source.reference.document_id, path)?;
            self.ensure_export_ready(source.reference.document_id, source.reference.revision)?
                .ensure_separate_output(path)?;
        }
        let bytes = self.comparison_report(comparison_id, allow_unencrypted)?;
        self.check_comparison(comparison_id)?;
        crate::output::write_new_copy(path, &bytes)?;
        Ok(bytes.len())
    }

    pub fn cancel_comparison(&mut self, comparison_id: u64) -> bool {
        if self
            .comparisons
            .job
            .as_ref()
            .is_some_and(|job| job.plan.comparison_id == comparison_id)
        {
            self.comparisons.job = None;
            true
        } else {
            false
        }
    }
}

fn source_limits(pages: usize, bytes: usize) -> Result<()> {
    if pages == 0 || pages > MAX_PAGES {
        return Err(invalid(
            "Comparison supports 1-100 pages per PDF. No pages were silently skipped.",
        ));
    }
    if bytes > MAX_SOURCE_BYTES {
        return Err(invalid(
            "Comparison supports source snapshots up to 64 MiB each.",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn source_size_and_page_thresholds_are_inclusive() {
    assert!(source_limits(MAX_PAGES, MAX_SOURCE_BYTES).is_ok());
    assert!(source_limits(MAX_PAGES + 1, MAX_SOURCE_BYTES).is_err());
    assert!(source_limits(MAX_PAGES, MAX_SOURCE_BYTES + 1).is_err());
    assert!(source_limits(0, 1).is_err());
}

fn validate_settings(settings: &Settings) -> Result<()> {
    if ![72, 96, 150].contains(&settings.dpi) || settings.tolerance > 64 {
        return Err(invalid(
            "Choose 72, 96 or 150 DPI and an RGB tolerance from 0 to 64.",
        ));
    }
    Ok(())
}

pub fn page_plans(
    before: &[PageGeometry],
    after: &[PageGeometry],
    dpi: u16,
) -> Result<Vec<PagePlan>> {
    if ![72, 96, 150].contains(&dpi)
        || before.is_empty()
        || after.is_empty()
        || before.len().max(after.len()) > MAX_PAGES
    {
        return Err(invalid(
            "Comparison requires two PDFs of 1-100 pages at 72, 96 or 150 DPI.",
        ));
    }
    let mut total = 0u64;
    (0..before.len().max(after.len())).map(|index| {
        let mut width = 0u32;
        let mut height = 0u32;
        for geometry in [before.get(index), after.get(index)].into_iter().flatten() {
            let w = (f64::from(geometry.width_points) * f64::from(dpi) / 72.0).ceil();
            let h = (f64::from(geometry.height_points) * f64::from(dpi) / 72.0).ceil();
            if !w.is_finite() || !h.is_finite() || w < 1.0 || h < 1.0
                || w > f64::from(MAX_DIMENSION) || h > f64::from(MAX_DIMENSION) {
                return Err(invalid(format!("Page {} exceeds 8,192 pixels per dimension at {dpi} DPI. Choose lower DPI.", index + 1)));
            }
            width = width.max(w as u32);
            height = height.max(h as u32);
        }
        let pixels = u64::from(width) * u64::from(height);
        total += pixels;
        if pixels > MAX_PAGE_PIXELS || total > MAX_TOTAL_PIXELS {
            return Err(invalid(format!("Page {} exceeds the comparison budget: 8 megapixels per aligned page pair, 100 megapixels total. Choose lower DPI or smaller PDFs; quality is never silently lowered.", index + 1)));
        }
        Ok(PagePlan { index, width, height })
    }).collect()
}

pub fn geometry_changes(
    before: Option<&PageGeometry>,
    after: Option<&PageGeometry>,
) -> Vec<&'static str> {
    let (Some(before), Some(after)) = (before, after) else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    if before.media_box != after.media_box {
        changes.push("media_box");
    }
    if before.bounds != after.bounds {
        changes.push("effective_crop");
    }
    if before.rotation != after.rotation {
        changes.push("rotation");
    }
    if before.user_unit != after.user_unit {
        changes.push("user_unit");
    }
    if before.width_points != after.width_points || before.height_points != after.height_points {
        changes.push("physical_size");
    }
    changes
}

fn report_bytes(plan: &Plan, pages: &[PageResult]) -> Result<Vec<u8>> {
    if pages.len() != plan.pages.len() {
        return Err(invalid(
            "The comparison report is incomplete. Compare every indexed page before exporting.",
        ));
    }

    let mut text_bytes = 0usize;
    let mut tokens = 0usize;
    let mut spans = 0usize;
    let mut changed = false;
    let mut report_pages = Vec::with_capacity(pages.len());
    for (expected, result) in plan.pages.iter().zip(pages) {
        let before = plan.before.pages.get(expected.index);
        let after = plan.after.pages.get(expected.index);
        let pixels = &result.pixels;
        let text = &result.text;
        let total = u64::from(expected.width) * u64::from(expected.height);
        if result.index != expected.index
            || pixels.width != expected.width
            || pixels.height != expected.height
            || pixels.total != total
            || pixels.changed > total
            || (pixels.changed == 0) != pixels.bounds.is_none()
            || (pixels.changed > 0) != (pixels.max_channel_delta > plan.settings.tolerance)
        {
            return Err(invalid(
                "The pixel comparison report has invalid dimensions, counts, tolerance or page order.",
            ));
        }
        if let Some([x, y, width, height]) = pixels.bounds
            && (width == 0
                || height == 0
                || x.checked_add(width).is_none_or(|end| end > expected.width)
                || y.checked_add(height)
                    .is_none_or(|end| end > expected.height)
                || u64::from(width) * u64::from(height) < pixels.changed)
        {
            return Err(invalid(
                "A pixel-difference bounding box is outside its page.",
            ));
        }
        let mut before_bytes = 0usize;
        let mut after_bytes = 0usize;
        let mut text_changed = false;
        for span in &text.spans {
            if span.text.is_empty() {
                return Err(invalid("Comparison text spans cannot be empty."));
            }
            if span.kind != SpanKind::Added {
                before_bytes += span.text.len();
            }
            if span.kind != SpanKind::Removed {
                after_bytes += span.text.len();
            }
            text_changed |= span.kind != SpanKind::Equal;
        }
        if before_bytes > MAX_PAGE_TEXT_BYTES
            || after_bytes > MAX_PAGE_TEXT_BYTES
            || text.before_tokens > MAX_PAGE_TOKENS
            || text.after_tokens > MAX_PAGE_TOKENS
            || before.is_none() && (before_bytes != 0 || text.before_tokens != 0)
            || after.is_none() && (after_bytes != 0 || text.after_tokens != 0)
        {
            return Err(invalid(
                "A page exceeds the 256 KiB/20,000-token text budget, or contains text for a missing page.",
            ));
        }
        text_bytes += before_bytes + after_bytes;
        tokens += text.before_tokens + text.after_tokens;
        spans += text.spans.len();
        if text_bytes > MAX_TEXT_BYTES || tokens > MAX_TOKENS || spans > MAX_SPANS {
            return Err(invalid(
                "The comparison exceeds 2 MiB extracted text, 100,000 tokens or 4,000 text spans. No truncated report was exported.",
            ));
        }
        let geometry_changes = geometry_changes(before, after);
        let presence = if before.is_none() {
            "added"
        } else if after.is_none() {
            "removed"
        } else {
            "matched"
        };
        changed |= presence != "matched"
            || text_changed
            || pixels.changed > 0
            || !geometry_changes.is_empty();
        report_pages.push(ReportPage {
            presence,
            geometry_changes,
            before_geometry: before,
            after_geometry: after,
            text_changed,
            result,
        });
    }
    let report = Report {
        schema: "folio.local-comparison",
        schema_version: 1,
        matching: "page_index",
        scope: SCOPE,
        protection_notice: "This JSON contains unencrypted extracted document content. It retains no PDF passwords, permissions or signatures.",
        conclusion: if changed {
            "Differences detected in the compared page text, pixels or geometry. Other document content was not compared."
        } else {
            "No differences detected in the compared page text, pixels and geometry. This does not mean the PDFs are identical."
        },
        plan,
        pages: report_pages,
    };
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| invalid(format!("Could not encode the comparison report: {error}")))?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(invalid(
            "The JSON comparison report exceeds 4 MiB. No truncated report was exported.",
        ));
    }
    Ok(bytes)
}
