use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::document::{DocumentInfo, Error, Result, Session};
use crate::redaction::{Job, Plan, Region};

#[derive(Default)]
pub struct Workspace {
    sessions: BTreeMap<u64, Session>,
    active_id: Option<u64>,
    initialized: bool,
    next_id: u64,
    redactions: BTreeMap<u64, Job>,
    next_redaction_id: u64,
    ocrs: BTreeMap<u64, crate::ocr::Job>,
    next_ocr_id: u64,
    pending_open: BTreeMap<u64, PendingOpen>,
    next_open_id: u64,
    drafts: HashSet<u64>,
    authoring_images: crate::authoring::ImageStaging,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub documents: Vec<DocumentInfo>,
    pub active_id: Option<u64>,
    pub opening: Vec<OpenOutcome>,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum OpenOutcome {
    Opened {
        document: DocumentInfo,
        imported_at: Option<usize>,
    },
    PasswordRequired {
        request_id: u64,
        name: String,
        incorrect: bool,
        importing: bool,
    },
    Failed {
        name: String,
        message: String,
    },
}

#[derive(Clone, Copy)]
enum OpenKind {
    Document,
    Import {
        document_id: u64,
        revision: u64,
        at: usize,
    },
}

struct PendingOpen {
    id: u64,
    bytes: Vec<u8>,
    name: String,
    path: Option<PathBuf>,
    kind: OpenKind,
}

impl OpenOutcome {
    pub fn opened(document: DocumentInfo) -> Self {
        Self::Opened {
            document,
            imported_at: None,
        }
    }
}

impl Workspace {
    pub fn snapshot(&mut self) -> Result<WorkspaceInfo> {
        if !self.initialized {
            self.bootstrap()?;
        }
        Ok(WorkspaceInfo {
            documents: self
                .sessions
                .values()
                .map(Session::info)
                .collect::<Result<_>>()?,
            active_id: self.active_id,
            opening: self
                .pending_open
                .values()
                .filter(|pending| matches!(pending.kind, OpenKind::Document))
                .map(|pending| OpenOutcome::PasswordRequired {
                    request_id: pending.id,
                    name: pending.name.clone(),
                    incorrect: false,
                    importing: false,
                })
                .collect(),
        })
    }

    pub fn activate(&mut self, document_id: u64) -> Result<DocumentInfo> {
        let info = self.document(document_id)?.info()?;
        self.active_id = Some(document_id);
        Ok(info)
    }

    pub fn document(&self, document_id: u64) -> Result<&Session> {
        self.sessions
            .get(&document_id)
            .ok_or_else(|| Error::Invalid("This PDF tab is no longer open.".into()))
    }

    pub fn close_document(
        &mut self,
        id: u64,
        revision: u64,
        discard_changes: bool,
    ) -> Result<WorkspaceInfo> {
        let document = self.check_revision(id, revision)?;
        if !discard_changes
            && (document.is_dirty()
                || self.drafts.contains(&id)
                || self.authoring_images.contains(id))
        {
            return Err(Error::Invalid("This PDF has unsaved changes or pending marks. Save or explicitly discard them before closing the tab.".into()));
        }
        self.sessions.remove(&id);
        self.drafts.remove(&id);
        self.authoring_images.clear(id);
        self.redactions.retain(|_, job| job.plan.document_id != id);
        self.ocrs.retain(|_, job| job.plan.document_id != id);
        self.pending_open.retain(|_, pending| {
            !matches!(pending.kind,
            OpenKind::Import { document_id, .. } if document_id == id)
        });
        if self.active_id == Some(id) {
            self.active_id = self
                .sessions
                .range((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
                .next()
                .map(|(id, _)| *id)
                .or_else(|| self.sessions.keys().next_back().copied());
        }
        self.snapshot()
    }

    pub fn set_draft(&mut self, id: u64, pending: bool) -> Result<()> {
        self.document(id)?;
        if pending {
            self.drafts.insert(id);
            self.ocrs.retain(|_, job| job.plan.document_id != id);
        } else {
            self.drafts.remove(&id);
        }
        Ok(())
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.is_dirty() || !self.drafts.is_empty() || !self.authoring_images.is_empty()
    }

    pub fn stage_authoring_image(
        &mut self,
        id: u64,
        revision: u64,
        bytes: &[u8],
    ) -> Result<crate::authoring::ImageAsset> {
        self.check_revision(id, revision)?.ensure_writable()?;
        self.authoring_images.stage(id, revision, bytes)
    }

    pub fn insert_authoring_image(
        &mut self,
        id: u64,
        revision: u64,
        asset_id: u64,
        placement: crate::authoring::ImagePlacement,
    ) -> Result<DocumentInfo> {
        self.check_revision(id, revision)?.ensure_writable()?;
        let image = self.authoring_images.get(id, revision, asset_id)?;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| Error::Invalid("This PDF tab is no longer open.".into()))?;
        let result = session.insert_image(image, placement)?;
        self.authoring_images.clear(id);
        Ok(result)
    }

    pub fn cancel_authoring_image(&mut self, id: u64, asset_id: u64) -> Result<bool> {
        self.document(id)?;
        self.authoring_images.cancel(id, asset_id)
    }

    pub fn ensure_export_ready(&self, id: u64, revision: u64) -> Result<&Session> {
        let session = self.check_revision(id, revision)?;
        if self.drafts.contains(&id) || self.authoring_images.contains(id) {
            return Err(Error::Invalid(
                "Apply or discard pending edits before saving or exporting this PDF. Use Export redacted copy for pending redactions.".into(),
            ));
        }
        Ok(session)
    }

    pub fn ensure_output_available(&self, id: u64, path: &Path) -> Result<()> {
        for session in self.sessions.values().filter(|session| session.id != id) {
            for other in [&session.source_path, &session.saved_path]
                .into_iter()
                .flatten()
            {
                if path == other
                    || path.exists()
                        && other.exists()
                        && path.canonicalize()? == other.canonicalize()?
                {
                    return Err(Error::Invalid(
                        "That file is open in another PDF tab. Choose a different filename.".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn check_signing(&self, id: u64, revision: u64) -> Result<&Session> {
        let session = self.ensure_export_ready(id, revision)?;
        if self
            .redactions
            .values()
            .any(|job| job.plan.document_id == id)
        {
            return Err(Error::Invalid(
                "Apply or cancel pending writing and clear pending redaction marks before signing. A signature must not export an unapplied or unredacted original.".into(),
            ));
        }
        session.ensure_signable()?;
        Ok(session)
    }

    pub fn signed_copy(
        &self,
        id: u64,
        revision: u64,
        identity: &[u8],
        password: &str,
    ) -> Result<Vec<u8>> {
        self.check_signing(id, revision)?
            .signed_copy(identity, password)
    }

    pub fn export_signed_copy(
        &self,
        id: u64,
        revision: u64,
        identity: &[u8],
        password: &str,
        path: &Path,
    ) -> Result<()> {
        self.ensure_output_available(id, path)?;
        self.check_signing(id, revision)?
            .ensure_separate_output(path)?;
        let bytes = self.signed_copy(id, revision, identity, password)?;
        self.ensure_output_available(id, path)?;
        self.check_signing(id, revision)?
            .ensure_separate_output(path)?;
        crate::document::atomic_write(path, &bytes)
    }

    pub fn request_open_paths(&mut self, paths: Vec<PathBuf>) -> Vec<OpenOutcome> {
        paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.request_open_path(path)
                    .unwrap_or_else(|error| OpenOutcome::Failed {
                        name,
                        message: error.to_string(),
                    })
            })
            .collect()
    }

    pub fn request_open_path(&mut self, path: PathBuf) -> Result<OpenOutcome> {
        self.initialized = true;
        let (bytes, name) = Session::read_path(&path)?;
        self.request_open(bytes, name, Some(path), OpenKind::Document)
    }

    pub fn request_open_bytes(&mut self, bytes: Vec<u8>, name: String) -> Result<OpenOutcome> {
        self.request_open(bytes, name, None, OpenKind::Document)
    }

    pub fn request_import_path(
        &mut self,
        id: u64,
        revision: u64,
        at: usize,
        path: PathBuf,
    ) -> Result<OpenOutcome> {
        self.check_revision(id, revision)?.ensure_writable()?;
        let (bytes, name) = Session::read_path(&path)?;
        self.request_open(
            bytes,
            name,
            Some(path),
            OpenKind::Import {
                document_id: id,
                revision,
                at,
            },
        )
    }

    pub fn request_import_bytes(
        &mut self,
        id: u64,
        revision: u64,
        at: usize,
        bytes: Vec<u8>,
        name: String,
    ) -> Result<OpenOutcome> {
        self.check_revision(id, revision)?.ensure_writable()?;
        self.request_open(
            bytes,
            name,
            None,
            OpenKind::Import {
                document_id: id,
                revision,
                at,
            },
        )
    }

    fn request_open(
        &mut self,
        bytes: Vec<u8>,
        name: String,
        path: Option<PathBuf>,
        kind: OpenKind,
    ) -> Result<OpenOutcome> {
        self.next_open_id = self.next_open_id.checked_add(1).ok_or_else(|| {
            Error::Invalid("Opening identifiers are exhausted. Restart Folio.".into())
        })?;
        self.initialized = true;
        let pending = PendingOpen {
            id: self.next_open_id,
            bytes,
            name,
            path,
            kind,
        };
        self.attempt_open(pending, None)
    }

    pub fn unlock_document(&mut self, request_id: u64, password: &str) -> Result<OpenOutcome> {
        if password.len() > 1024 {
            return Err(Error::Invalid(
                "The PDF password exceeds the 1,024-byte limit.".into(),
            ));
        }
        if !self.pending_open.contains_key(&request_id) {
            return Err(Error::Invalid(
                "This password request was canceled or superseded. Open the PDF again.".into(),
            ));
        }
        let pending = self
            .pending_open
            .remove(&request_id)
            .expect("request checked above");
        self.attempt_open(pending, Some(password))
    }

    pub fn cancel_open(&mut self, request_id: u64) -> bool {
        self.pending_open.remove(&request_id).is_some()
    }

    fn attempt_open(
        &mut self,
        pending: PendingOpen,
        password: Option<&str>,
    ) -> Result<OpenOutcome> {
        if let OpenKind::Import {
            document_id,
            revision,
            ..
        } = pending.kind
        {
            self.check_revision(document_id, revision)?;
        }
        let id = self.next_id.checked_add(1).ok_or_else(|| {
            Error::Invalid("Document identifiers are exhausted. Restart Folio.".into())
        })?;
        match Session::from_bytes_with_password(
            pending.bytes.clone(),
            pending.name.clone(),
            pending.path.clone(),
            id,
            password,
        ) {
            Err(Error::PasswordRequired { incorrect }) => {
                let outcome = OpenOutcome::PasswordRequired {
                    request_id: pending.id,
                    name: pending.name.clone(),
                    incorrect,
                    importing: matches!(pending.kind, OpenKind::Import { .. }),
                };
                self.pending_open.insert(pending.id, pending);
                Ok(outcome)
            }
            Err(error) => Err(error),
            Ok(session) => match pending.kind {
                OpenKind::Document => Ok(OpenOutcome::opened(self.add(session)?)),
                OpenKind::Import {
                    document_id,
                    revision,
                    at,
                } => {
                    let document = self
                        .current_mut(document_id, revision)?
                        .import_session(session, at)?;
                    Ok(OpenOutcome::Opened {
                        document,
                        imported_at: Some(at),
                    })
                }
            },
        }
    }

    pub fn current(&self) -> Result<&Session> {
        self.active_id
            .ok_or_else(|| Error::Invalid("Open a PDF to begin.".into()))
            .and_then(|id| self.document(id))
    }

    pub fn current_mut(&mut self, id: u64, revision: u64) -> Result<&mut Session> {
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| Error::Invalid("Open a PDF to begin.".to_owned()))?;
        if session.id != id || session.revision() != revision {
            return Err(Error::Invalid(
                "The document changed before this action completed. Try the action again."
                    .to_owned(),
            ));
        }
        self.ocrs.retain(|_, job| job.plan.document_id != id);
        Ok(session)
    }

    pub fn check_revision(&self, id: u64, revision: u64) -> Result<&Session> {
        let session = self.document(id)?;
        if session.id != id || session.revision() != revision {
            return Err(Error::Invalid(
                "The document changed before this action completed. Try the action again."
                    .to_owned(),
            ));
        }
        Ok(session)
    }

    pub fn is_dirty(&self) -> bool {
        self.sessions.values().any(Session::is_dirty)
    }

    pub fn begin_redaction(
        &mut self,
        id: u64,
        revision: u64,
        dpi: u16,
        regions: &[Region],
    ) -> Result<Plan> {
        if self
            .ocrs
            .values()
            .any(|job| job.plan.document_id == id && !job.expired())
        {
            return Err(Error::Invalid(
                "Cancel offline OCR before preparing a redacted copy.".into(),
            ));
        }
        let job_id = self.next_redaction_id.checked_add(1).ok_or_else(|| {
            Error::Invalid("Redaction identifiers are exhausted. Restart Folio.".into())
        })?;
        let plan = self
            .check_revision(id, revision)?
            .redaction_plan(job_id, dpi, regions)?;
        self.next_redaction_id = job_id;
        self.redactions.retain(|_, job| job.plan.document_id != id);
        self.redactions.insert(job_id, Job::new(plan.clone()));
        Ok(plan)
    }

    pub fn check_redaction(&self, job_id: u64) -> Result<&Session> {
        let job = self.redactions.get(&job_id).ok_or_else(|| {
            Error::Invalid("This redaction export was canceled or superseded. Export again.".into())
        })?;
        self.check_revision(job.plan.document_id, job.plan.revision)
    }

    pub fn stage_redaction_page(&mut self, job_id: u64, page: usize, png: &[u8]) -> Result<()> {
        self.check_redaction(job_id)?;
        self.redactions
            .get_mut(&job_id)
            .expect("job checked above")
            .stage(page, png)
    }

    pub fn redaction_bytes(&mut self, job_id: u64) -> Result<Vec<u8>> {
        self.check_redaction(job_id)?;
        let bytes = self
            .redactions
            .get_mut(&job_id)
            .expect("job checked above")
            .bytes()?;
        self.check_redaction(job_id)?.protect_export(bytes)
    }

    pub fn cancel_redaction(&mut self, job_id: u64) -> bool {
        self.redactions.remove(&job_id).is_some()
    }

    pub fn begin_ocr(
        &mut self,
        id: u64,
        revision: u64,
        options: crate::ocr::Options,
    ) -> Result<crate::ocr::Plan> {
        self.ocrs.retain(|_, job| !job.expired());
        self.ensure_ocr_ready(id, revision)?;
        if self.ocrs.len() >= 4 || self.ocrs.values().any(|job| job.plan.document_id == id) {
            return Err(Error::Invalid("An OCR job is already running for this PDF, or the four-job workspace limit was reached. Cancel it before starting another.".into()));
        }
        let job_id = self.next_ocr_id.checked_add(1).ok_or_else(|| {
            Error::Invalid("OCR identifiers are exhausted. Restart Folio.".into())
        })?;
        let plan = self
            .check_revision(id, revision)?
            .ocr_plan(job_id, options)?;
        self.next_ocr_id = job_id;
        self.ocrs.insert(job_id, crate::ocr::Job::new(plan.clone()));
        Ok(plan)
    }

    fn ensure_ocr_ready(&self, id: u64, revision: u64) -> Result<()> {
        self.ensure_export_ready(id, revision)?.ensure_writable()?;
        if self
            .redactions
            .values()
            .any(|job| job.plan.document_id == id)
        {
            return Err(Error::Invalid("Finish or cancel pending writing/form drafts and clear redaction marks before using OCR.".into()));
        }
        Ok(())
    }

    pub fn stage_ocr_page(&mut self, job_id: u64, result: crate::ocr::PageResult) -> Result<()> {
        self.ocrs.retain(|_, job| !job.expired());
        let job = self.ocrs.get(&job_id)
            .ok_or_else(|| Error::Invalid("This OCR job was canceled, expired, or invalidated by a document change. No text was added.".into()))?;
        self.ensure_ocr_ready(job.plan.document_id, job.plan.revision)?;
        self.ocrs
            .get_mut(&job_id)
            .expect("OCR job checked above")
            .stage(result)
    }

    pub fn commit_ocr(&mut self, job_id: u64) -> Result<crate::ocr::Outcome> {
        let job = self.ocrs.remove(&job_id).ok_or_else(|| {
            Error::Invalid("This OCR job was canceled or invalidated. No text was added.".into())
        })?;
        if job.expired() {
            return Err(Error::Invalid(
                "This OCR job exceeded its 15-minute limit. No text was added.".into(),
            ));
        }
        self.ensure_ocr_ready(job.plan.document_id, job.plan.revision)?;
        self.current_mut(job.plan.document_id, job.plan.revision)?
            .commit_ocr(&job)
    }

    pub fn cancel_ocr(&mut self, job_id: u64) -> bool {
        self.ocrs.remove(&job_id).is_some()
    }

    pub fn bootstrap(&mut self) -> Result<DocumentInfo> {
        match self.active_id {
            Some(id) => self.document(id)?.info(),
            None if !self.initialized => self.open_demo(),
            None => Err(Error::Invalid("Open a PDF to begin.".into())),
        }
    }

    pub fn open_path(&mut self, path: PathBuf) -> Result<DocumentInfo> {
        let session = Session::from_path(path, self.next_id + 1)?;
        self.add(session)
    }

    pub fn open_bytes(&mut self, bytes: Vec<u8>, name: String) -> Result<DocumentInfo> {
        let session = Session::from_bytes(bytes, name, None, self.next_id + 1)?;
        self.add(session)
    }

    pub fn open_demo(&mut self) -> Result<DocumentInfo> {
        self.add(Session::demo(self.next_id + 1)?)
    }

    pub fn create_document(&mut self, width: f32, height: f32) -> Result<DocumentInfo> {
        self.add(Session::blank(width, height, self.next_id + 1)?)
    }

    fn add(&mut self, session: Session) -> Result<DocumentInfo> {
        let info = session.info()?;
        self.next_id = session.id;
        self.active_id = Some(session.id);
        self.sessions.insert(session.id, session);
        self.initialized = true;
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Edit;

    #[test]
    fn failed_open_preserves_work_and_new_tabs_do_not_discard_it() {
        let mut workspace = Workspace::default();
        let first = workspace.bootstrap().unwrap();
        workspace
            .current_mut(first.id, first.revision)
            .unwrap()
            .apply(Edit::Rotate {
                page: 0,
                degrees: 90,
            })
            .unwrap();
        assert!(
            workspace
                .open_bytes(b"bad".to_vec(), "bad.pdf".into())
                .is_err()
        );
        assert!(workspace.is_dirty());
        assert_eq!(workspace.current().unwrap().id, first.id);
        let second = workspace.open_demo().unwrap();
        assert_ne!(first.id, second.id);
        assert!(workspace.current_mut(first.id, first.revision).is_err());
        assert!(workspace.document(first.id).unwrap().is_dirty());
        assert!(workspace.close_document(first.id, 1, false).is_err());
        assert_eq!(workspace.snapshot().unwrap().documents.len(), 2);
    }
}
