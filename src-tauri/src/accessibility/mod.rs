mod source;
mod structure;

use std::{
    path::Path,
    time::{Duration, Instant},
};

use lopdf::{Document, Object, dictionary, text_string};
use serde::{Deserialize, Serialize};

use crate::{
    archive::SourceRef,
    document::{MAX_DOCUMENT_BYTES, Result, Session, invalid},
    pdf_objects::{resolve, text, validate_text},
    workspace::Workspace,
};

pub const MAX_PAGES: usize = 100;
pub const MAX_ITEMS: usize = 2_000;
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MARKER: &[u8] = b"FolioTaggedCopy";
pub const BOUNDARY: &str = "Reviewed logical structure, not PDF/UA, WCAG or Acrobat certification. You are responsible for meaningful alternatives, language, heading hierarchy and reading order. Folio preserves source drawings and searchable text in a separate, protected-as-source tagged copy. Generated copies open read-only to prevent stale content associations. No AI, cloud service or external validator is used.";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Text,
    Figure,
    Graphic,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub page: usize,
    pub kind: CandidateKind,
    pub text: Option<String>,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum Role {
    P,
    H1,
    H2,
    H3,
    H4,
    H5,
    H6,
    Figure,
    Artifact,
}

impl Role {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::P => "P",
            Self::H1 => "H1",
            Self::H2 => "H2",
            Self::H3 => "H3",
            Self::H4 => "H4",
            Self::H5 => "H5",
            Self::H6 => "H6",
            Self::Figure => "Figure",
            Self::Artifact => "Artifact",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Assignment {
    pub candidate_id: String,
    pub role: Role,
    pub alt: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CopyRequest {
    #[serde(flatten)]
    pub source: SourceRef,
    pub source_epoch: u64,
    pub title: String,
    pub language: String,
    pub entries: Vec<Assignment>,
    pub reviewed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBinding {
    pub page: usize,
    pub mcid: i64,
    pub stream: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticNode {
    pub id: String,
    pub role: String,
    pub language: Option<String>,
    pub alt: Option<String>,
    pub text: Option<String>,
    pub bindings: Vec<ContentBinding>,
    pub children: Vec<SemanticNode>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub inspection_id: u64,
    #[serde(flatten)]
    pub source: SourceRef,
    pub epoch: u64,
    pub source_epoch: u64,
    pub name: String,
    pub title: String,
    pub language: Option<String>,
    pub status: String,
    pub authoring_allowed: bool,
    pub complete: bool,
    pub candidates: Vec<Candidate>,
    pub structure: Vec<SemanticNode>,
    pub issues: Vec<String>,
    pub boundary: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyPlan {
    pub job_id: u64,
    #[serde(flatten)]
    pub source: SourceRef,
    pub epoch: u64,
    pub source_epoch: u64,
    pub suggested_name: String,
    pub byte_length: usize,
    pub password_protected: bool,
    pub boundary: &'static str,
}

struct Job {
    plan: CopyPlan,
    bytes: Vec<u8>,
    started: Instant,
}

#[derive(Default)]
pub(crate) struct AccessibilityJobs {
    next_id: u64,
    inspection: Option<Inspection>,
    job: Option<Job>,
}

impl AccessibilityJobs {
    pub(crate) fn invalidate(&mut self, id: u64) {
        if self
            .inspection
            .as_ref()
            .is_some_and(|value| value.source.document_id == id)
        {
            self.inspection = None;
        }
        if self.has_document(id) {
            self.job = None;
        }
    }
    pub(crate) fn has_document(&self, id: u64) -> bool {
        self.job
            .as_ref()
            .is_some_and(|value| value.plan.source.document_id == id)
    }
    pub(crate) fn is_pending(&self) -> bool {
        self.job.is_some()
    }
    fn next(&self) -> Result<u64> {
        self.next_id
            .checked_add(1)
            .ok_or_else(|| invalid("Accessibility identifiers are exhausted. Restart Folio."))
    }
}

pub(crate) fn is_folio_tagged_copy(document: &Document) -> bool {
    // This is an edit-safety policy only, never evidence of trustworthy semantics or conformance.
    document.catalog().is_ok_and(|catalog| catalog.has(MARKER))
}

pub(crate) fn language(value: &str) -> Result<()> {
    let parts: Vec<_> = value.split('-').collect();
    let primary = parts.first().copied().unwrap_or("");
    if value.len() > 63
        || parts.len() > 12
        || !(2..=8).contains(&primary.len()) && primary != "x"
        || !primary.bytes().all(|byte| byte.is_ascii_alphabetic())
        || parts.iter().skip(1).any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        || primary == "x" && parts.len() < 2
    {
        return Err(invalid(
            "Enter a well-formed language tag such as en-US, hi-IN or zh-Hant. Registry membership and language choice require human review.",
        ));
    }
    Ok(())
}

struct DocumentInspection {
    status: String,
    language: Option<String>,
    title: String,
    candidates: Vec<Candidate>,
    structure: Vec<SemanticNode>,
    issues: Vec<String>,
    complete: bool,
}

fn inspect_document(document: &Document) -> Result<DocumentInspection> {
    let metadata = crate::metadata::inspect(document)?;
    let lang = document
        .catalog()?
        .get(b"Lang")
        .ok()
        .map(|value| text(document, value, 63))
        .transpose()?;
    if document.catalog()?.has(b"StructTreeRoot") {
        return Ok(match structure::inspect(document) {
            Ok(result) => DocumentInspection {
                status: "tagged".into(),
                language: lang,
                title: metadata.fields.title,
                candidates: vec![],
                structure: result.nodes,
                issues: result.issues,
                complete: result.complete,
            },
            Err(error) => DocumentInspection {
                status: "malformed_or_unsupported".into(),
                language: lang,
                title: metadata.fields.title,
                candidates: vec![],
                structure: vec![],
                issues: vec![error.to_string()],
                complete: false,
            },
        });
    }
    Ok(match source::inventory(document) {
        Ok(items) => DocumentInspection {
            status: "untagged".into(),
            language: lang,
            title: metadata.fields.title,
            candidates: items.into_iter().map(|item| item.view).collect(),
            structure: vec![],
            issues: vec![],
            complete: false,
        },
        Err(error) => DocumentInspection {
            status: "unsupported".into(),
            language: lang,
            title: metadata.fields.title,
            candidates: vec![],
            structure: vec![],
            issues: vec![error.to_string()],
            complete: false,
        },
    })
}

impl Session {
    pub(crate) fn accessibility_inspection(
        &self,
        inspection_id: u64,
        epoch: u64,
    ) -> Result<Inspection> {
        if !self.permissions().copy_allowed {
            return Err(invalid(
                "This PDF restricts copying. Reopen with its owner password to inspect or read its logical structure.",
            ));
        }
        let DocumentInspection {
            status,
            language,
            title,
            candidates,
            structure,
            mut issues,
            complete,
        } = inspect_document(self.accessibility_document())?;
        let allowed = status == "untagged"
            && !candidates.is_empty()
            && self.ensure_management_writable().is_ok();
        if status == "untagged" {
            if let Err(error) = self.ensure_management_writable() {
                issues.push(error.to_string());
            }
            if candidates.is_empty() {
                issues.push("No meaningful supported source candidates were found. No tagged copy can be prepared.".into());
            }
        }
        let result = Inspection {
            inspection_id,
            source: SourceRef {
                document_id: self.id,
                revision: self.revision(),
            },
            epoch,
            source_epoch: self.mutation_epoch(),
            name: self.name.clone(),
            title,
            language,
            status,
            authoring_allowed: allowed,
            complete,
            candidates,
            structure,
            issues,
            boundary: BOUNDARY,
        };
        if serde_json::to_vec(&result)
            .map_err(|error| invalid(error.to_string()))?
            .len()
            > MAX_TEXT_BYTES
        {
            return Err(invalid(
                "The semantic inspection exceeds 2 MiB. No partial structure is presented.",
            ));
        }
        Ok(result)
    }

    pub fn accessibility_copy(&self, request: &CopyRequest) -> Result<Vec<u8>> {
        self.ensure_management_writable()?;
        if request.source.document_id != self.id
            || request.source.revision != self.revision()
            || request.source_epoch != self.mutation_epoch()
        {
            return Err(invalid(
                "The accessibility source changed, including an edit or undo/redo. Inspect the current PDF again; your reviewed values are kept.",
            ));
        }
        tagged_copy(self.accessibility_document(), request)
    }
}

fn tagged_copy(document: &Document, request: &CopyRequest) -> Result<Vec<u8>> {
    if !request.reviewed {
        return Err(invalid(
            "Review every source candidate, logical order, headings, language and meaningful figure alternative before preparing a tagged copy.",
        ));
    }
    validate_text(&request.title, 4096, false)?;
    language(&request.language)?;
    let items = source::inventory(document)?;
    if items.is_empty() || items.len() != request.entries.len() {
        return Err(invalid(
            "Every meaningful source candidate must have exactly one reviewed role or explicit Artifact assignment. No source content is silently hidden.",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut meaningful = 0;
    for entry in &request.entries {
        let item = items
            .iter()
            .find(|item| item.view.id == entry.candidate_id)
            .ok_or_else(|| invalid("A reviewed source candidate no longer exists."))?;
        if !seen.insert(&entry.candidate_id) {
            return Err(invalid("A source candidate was assigned more than once."));
        }
        if let Some(value) = &entry.language {
            language(value)?;
        }
        if item.view.kind == CandidateKind::Text && entry.role == Role::Figure
            || item.view.kind != CandidateKind::Text
                && !matches!(entry.role, Role::Figure | Role::Artifact)
        {
            return Err(invalid(
                "Text needs a paragraph, heading or Artifact role; images and painted graphics need Figure with a human alternative, or explicit Artifact.",
            ));
        }
        if entry.role == Role::Figure {
            validate_text(entry.alt.as_deref().unwrap_or(""), 8192, false)?;
        } else if entry.alt.as_ref().is_some_and(|value| !value.is_empty()) {
            return Err(invalid(
                "Only a Figure accepts alternative text. Decorative content must be explicitly assigned Artifact, not an empty Figure.",
            ));
        }
        if entry.role != Role::Artifact {
            meaningful += 1;
        }
    }
    if meaningful == 0 {
        return Err(invalid(
            "A document cannot be exported with all meaningful content hidden as Artifact.",
        ));
    }

    let mut output = document.clone();
    let root_id = output.new_object_id();
    let document_id = output.new_object_id();
    let mut kids = Vec::new();
    let mut parent_entries = std::collections::BTreeMap::new();
    let mut page_mcids = std::collections::BTreeMap::new();
    let mut form_key = output.get_pages().len() as i64;
    let mut annotations = Vec::new();
    let mut id_names = Vec::new();
    for (index, entry) in request.entries.iter().enumerate() {
        let item = items
            .iter()
            .find(|item| item.view.id == entry.candidate_id)
            .expect("validated inventory");
        let page_id = crate::document::page_id(&output, item.view.page)?;
        if entry.role == Role::Artifact {
            annotations.push(source::Tag {
                item: item.clone(),
                role: entry.role,
                mcid: None,
                element: None,
                scope_key: None,
            });
            continue;
        }
        let element = output.new_object_id();
        let key = if item.managed {
            let key = form_key;
            form_key += 1;
            key
        } else {
            item.view.page as i64
        };
        let mcid = if item.managed {
            0
        } else {
            let value = page_mcids.entry(item.view.page).or_insert(0_i64);
            let mcid = *value;
            *value += 1;
            mcid
        };
        let parents = parent_entries.entry(key).or_insert_with(Vec::new);
        parents.push(Object::Reference(element));
        let id = format!("folio-semantic-{index:04}");
        let mut dictionary = dictionary! {
            "Type"=>"StructElem","S"=>entry.role.name(),"P"=>document_id,"Pg"=>page_id,
            "ID"=>text_string(&id),
        };
        if !item.managed {
            dictionary.set("K", mcid);
        }
        if let Some(value) = &entry.language {
            dictionary.set("Lang", text_string(value));
        }
        if entry.role == Role::Figure {
            dictionary.set(
                "Alt",
                text_string(entry.alt.as_deref().expect("validated alternative")),
            );
        }
        output.objects.insert(element, dictionary.into());
        id_names.push(Object::string_literal(id));
        id_names.push(Object::Reference(element));
        kids.push(Object::Reference(element));
        annotations.push(source::Tag {
            item: item.clone(),
            role: entry.role,
            mcid: Some(mcid),
            element: Some(element),
            scope_key: Some(key),
        });
    }
    source::tag_pages(&mut output, &annotations)?;
    let mut nums = Vec::new();
    for (key, values) in parent_entries {
        nums.push(Object::Integer(key));
        nums.push(Object::Array(values));
    }
    let parents = output.add_object(dictionary! { "Nums"=>nums });
    let ids = output.add_object(dictionary! { "Names"=>id_names });
    output.objects.insert(
        document_id,
        dictionary! { "Type"=>"StructElem","S"=>"Document","P"=>root_id,"K"=>kids }.into(),
    );
    output.objects.insert(
        root_id,
        dictionary! {
            "Type"=>"StructTreeRoot","K"=>vec![Object::Reference(document_id)],
            "ParentTree"=>parents,"ParentTreeNextKey"=>form_key,"IDTree"=>ids,
        }
        .into(),
    );
    let mut fields = crate::metadata::inspect(&output)?.fields;
    fields.title = request.title.clone();
    crate::metadata::update(&mut output, fields)?;
    let catalog = output.catalog_mut()?;
    catalog.set("StructTreeRoot", root_id);
    catalog.set("Lang", text_string(&request.language));
    catalog.set("MarkInfo", dictionary! { "Marked"=>true });
    catalog.set(
        MARKER,
        dictionary! { "V"=>1,"Policy"=>"ReadOnly","NoConformanceClaim"=>true },
    );
    let mut preferences = match catalog.get(b"ViewerPreferences") {
        Ok(value) => resolve(document, value)?.as_dict()?.clone(),
        Err(lopdf::Error::DictKey(_)) => lopdf::Dictionary::new(),
        Err(error) => return Err(error.into()),
    };
    preferences.set("DisplayDocTitle", true);
    catalog.set("ViewerPreferences", preferences);
    let checked = structure::inspect(&output)?;
    if !checked.complete {
        return Err(invalid(format!(
            "The tagged copy did not bind every source item: {}",
            checked.issues.join(" ")
        )));
    }
    let bytes = crate::protection::serialize(&output)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid(
            "The tagged copy exceeds 128 MiB. No partial copy is exported.",
        ));
    }
    Ok(bytes)
}

impl Workspace {
    fn accessibility_ready(&self, source: &SourceRef) -> Result<&Session> {
        let session = self.ensure_export_ready(source.document_id, source.revision)?;
        if self.has_redaction_job(source.document_id) {
            return Err(invalid(
                "Clear pending redaction marks before accessibility inspection or export. Tagging is not redaction.",
            ));
        }
        if !session.permissions().copy_allowed {
            return Err(invalid(
                "This PDF restricts copying. Reopen with its owner password for structured reading or semantic authoring.",
            ));
        }
        Ok(session)
    }

    pub fn inspect_accessibility(&mut self, source: SourceRef) -> Result<Inspection> {
        let id = self.accessibility.next()?;
        let result = self
            .accessibility_ready(&source)?
            .accessibility_inspection(id, self.technical_epoch(source.document_id))?;
        self.accessibility.next_id = id;
        self.accessibility.inspection = Some(result.clone());
        Ok(result)
    }
    pub fn check_accessibility_inspection(&self, inspection_id: u64) -> Result<Inspection> {
        let report = self.accessibility.inspection.as_ref().filter(|value| value.inspection_id == inspection_id)
            .ok_or_else(|| invalid("This semantic inspection was canceled or invalidated by a source change, draft, staged image or closed tab. Inspect the current PDF again."))?;
        let session = self.accessibility_ready(&report.source)?;
        if report.epoch != self.technical_epoch(report.source.document_id)
            || report.source_epoch != session.mutation_epoch()
        {
            return Err(invalid(
                "The semantic inspection source epoch changed. Inspect the current PDF again.",
            ));
        }
        Ok(report.clone())
    }
    pub fn cancel_accessibility_inspection(&mut self, inspection_id: u64) -> bool {
        if self
            .accessibility
            .inspection
            .as_ref()
            .is_some_and(|value| value.inspection_id == inspection_id)
        {
            self.accessibility.inspection = None;
            true
        } else {
            false
        }
    }
    pub fn begin_accessibility_copy(&mut self, request: CopyRequest) -> Result<CopyPlan> {
        if self.accessibility.is_pending() {
            return Err(invalid(
                "Finish or cancel the previous tagged-copy job before preparing another.",
            ));
        }
        let id = self.accessibility.next()?;
        let session = self.accessibility_ready(&request.source)?;
        let bytes = session.accessibility_copy(&request)?;
        let plan = CopyPlan {
            job_id: id,
            source: request.source,
            source_epoch: request.source_epoch,
            epoch: self.technical_epoch(session.id),
            byte_length: bytes.len(),
            password_protected: session.info()?.password_protected,
            suggested_name: format!("{}-tagged.pdf", crate::output::safe_stem(&session.name)),
            boundary: BOUNDARY,
        };
        self.accessibility.next_id = id;
        self.accessibility.job = Some(Job {
            plan: plan.clone(),
            bytes,
            started: Instant::now(),
        });
        Ok(plan)
    }
    pub fn check_accessibility_copy(&self, job_id: u64) -> Result<CopyPlan> {
        let job = self.accessibility.job.as_ref().filter(|value| value.plan.job_id == job_id)
            .ok_or_else(|| invalid("This tagged-copy job was canceled or invalidated by a source edit, draft, staged image or closed tab. Prepare the current PDF again."))?;
        let session = self.accessibility_ready(&job.plan.source)?;
        if job.started.elapsed() > Duration::from_secs(900)
            || job.plan.epoch != self.technical_epoch(session.id)
            || job.plan.source_epoch != session.mutation_epoch()
        {
            return Err(invalid(
                "The tagged-copy job expired or its source epoch changed. Prepare the current PDF again.",
            ));
        }
        Ok(job.plan.clone())
    }
    pub fn accessibility_copy_bytes(&self, job_id: u64) -> Result<Vec<u8>> {
        self.check_accessibility_copy(job_id)?;
        Ok(self
            .accessibility
            .job
            .as_ref()
            .expect("checked job")
            .bytes
            .clone())
    }
    pub fn write_accessibility_copy(&mut self, job_id: u64, path: &Path) -> Result<usize> {
        let plan = self.check_accessibility_copy(job_id)?;
        if path
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|value| !value.eq_ignore_ascii_case("pdf"))
        {
            return Err(invalid("Use a .pdf filename for the tagged copy."));
        }
        self.ensure_output_available(plan.source.document_id, path)?;
        self.accessibility_ready(&plan.source)?
            .ensure_separate_output(path)?;
        let bytes = self.accessibility_copy_bytes(job_id)?;
        self.check_accessibility_copy(job_id)?;
        self.ensure_output_available(plan.source.document_id, path)?;
        self.accessibility_ready(&plan.source)?
            .ensure_separate_output(path)?;
        crate::output::write_new_copy(path, &bytes)?;
        self.cancel_accessibility_copy(job_id);
        Ok(bytes.len())
    }
    pub fn cancel_accessibility_copy(&mut self, job_id: u64) -> bool {
        if self
            .accessibility
            .job
            .as_ref()
            .is_some_and(|value| value.plan.job_id == job_id)
        {
            self.accessibility.job = None;
            true
        } else {
            false
        }
    }
}
