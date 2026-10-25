use std::path::PathBuf;

use crate::document::{DocumentInfo, Error, Result, Session};
use crate::redaction::{Job, Plan, Region};

#[derive(Default)]
pub struct Workspace {
    session: Option<Session>,
    next_id: u64,
    redaction: Option<Job>,
    next_redaction_id: u64,
}

impl Workspace {
    pub fn current(&self) -> Result<&Session> {
        self.session
            .as_ref()
            .ok_or_else(|| Error::Invalid("Open a PDF to begin.".to_owned()))
    }

    pub fn current_mut(&mut self, id: u64, revision: u64) -> Result<&mut Session> {
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| Error::Invalid("Open a PDF to begin.".to_owned()))?;
        if session.id != id || session.revision() != revision {
            return Err(Error::Invalid(
                "The document changed before this action completed. Try the action again."
                    .to_owned(),
            ));
        }
        Ok(session)
    }

    pub fn check_revision(&self, id: u64, revision: u64) -> Result<&Session> {
        let session = self.current()?;
        if session.id != id || session.revision() != revision {
            return Err(Error::Invalid(
                "The document changed before this action completed. Try the action again."
                    .to_owned(),
            ));
        }
        Ok(session)
    }

    pub fn is_dirty(&self) -> bool {
        self.session.as_ref().is_some_and(Session::is_dirty)
    }

    pub fn begin_redaction(
        &mut self,
        id: u64,
        revision: u64,
        dpi: u16,
        regions: &[Region],
    ) -> Result<Plan> {
        let job_id = self.next_redaction_id.checked_add(1).ok_or_else(|| {
            Error::Invalid("Redaction identifiers are exhausted. Restart Folio.".into())
        })?;
        let plan = self
            .check_revision(id, revision)?
            .redaction_plan(job_id, dpi, regions)?;
        self.next_redaction_id = job_id;
        // A new generation releases an abandoned export without letting its old requests finish this one.
        self.redaction = Some(Job::new(plan.clone()));
        Ok(plan)
    }

    pub fn check_redaction(&self, job_id: u64) -> Result<&Session> {
        let job = self
            .redaction
            .as_ref()
            .filter(|job| job.plan.job_id == job_id)
            .ok_or_else(|| {
                Error::Invalid(
                    "This redaction export was canceled or superseded. Export again.".into(),
                )
            })?;
        self.check_revision(job.plan.document_id, job.plan.revision)
    }

    pub fn stage_redaction_page(&mut self, job_id: u64, page: usize, png: &[u8]) -> Result<()> {
        self.check_redaction(job_id)?;
        self.redaction
            .as_mut()
            .expect("job checked above")
            .stage(page, png)
    }

    pub fn redaction_bytes(&mut self, job_id: u64) -> Result<Vec<u8>> {
        self.check_redaction(job_id)?;
        self.redaction.as_mut().expect("job checked above").bytes()
    }

    pub fn cancel_redaction(&mut self, job_id: u64) -> bool {
        if self
            .redaction
            .as_ref()
            .is_some_and(|job| job.plan.job_id == job_id)
        {
            self.redaction = None;
            true
        } else {
            false
        }
    }

    pub fn bootstrap(&mut self) -> Result<DocumentInfo> {
        match &self.session {
            Some(session) => session.info(),
            None => self.open_demo(true),
        }
    }

    pub fn open_path(&mut self, path: PathBuf, discard_changes: bool) -> Result<DocumentInfo> {
        self.check_discard(discard_changes)?;
        let session = Session::from_path(path, self.next_id + 1)?;
        self.replace(session)
    }

    pub fn open_bytes(
        &mut self,
        bytes: Vec<u8>,
        name: String,
        discard_changes: bool,
    ) -> Result<DocumentInfo> {
        self.check_discard(discard_changes)?;
        let session = Session::from_bytes(bytes, name, None, self.next_id + 1)?;
        self.replace(session)
    }

    pub fn open_demo(&mut self, discard_changes: bool) -> Result<DocumentInfo> {
        self.check_discard(discard_changes)?;
        self.replace(Session::demo(self.next_id + 1)?)
    }

    pub fn create_document(
        &mut self,
        width: f32,
        height: f32,
        discard_changes: bool,
    ) -> Result<DocumentInfo> {
        self.check_discard(discard_changes)?;
        self.replace(Session::blank(width, height, self.next_id + 1)?)
    }

    fn check_discard(&self, discard_changes: bool) -> Result<()> {
        if self.is_dirty() && !discard_changes {
            return Err(Error::Invalid(
                "There are unsaved changes. Save a copy or explicitly discard them first."
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn replace(&mut self, session: Session) -> Result<DocumentInfo> {
        let info = session.info()?;
        self.next_id = session.id;
        self.session = Some(session);
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Edit;

    #[test]
    fn failed_open_keeps_current_document_and_unsaved_changes_need_consent() {
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
        assert!(workspace.open_demo(false).is_err());
        assert!(
            workspace
                .open_bytes(b"bad".to_vec(), "bad.pdf".into(), true)
                .is_err()
        );
        assert!(workspace.is_dirty());
        assert_eq!(workspace.current().unwrap().id, first.id);
        let second = workspace.open_demo(true).unwrap();
        assert_ne!(first.id, second.id);
        assert!(workspace.current_mut(first.id, first.revision).is_err());
    }
}
