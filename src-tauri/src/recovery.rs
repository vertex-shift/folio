use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::document::{Error, MAX_DOCUMENT_BYTES, Result, Session, atomic_write, invalid};

const MAGIC: &[u8; 9] = b"FOLIOREC1";
const MAX_HEADER: usize = 32 * 1024;
const MAX_COPIES: usize = 16;
const MAX_STORAGE: u64 = 256 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
    source: Option<PathBuf>,
    saved: Option<PathBuf>,
    size: usize,
    checksum: u32,
    originals: Vec<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub token: String,
    pub name: String,
    pub byte_length: u64,
    pub modified: u64,
    pub problem: Option<String>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub available: bool,
    pub enabled: bool,
    pub location: Option<String>,
    pub entries: Vec<Entry>,
    pub checkpointed: usize,
    pub excluded: Vec<String>,
    pub error: Option<String>,
}

pub struct Store {
    root: PathBuf,
    _lock: File,
    enabled: bool,
    current: BTreeMap<u64, (String, u64)>,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        let root = std::path::absolute(root)?;
        fs::create_dir_all(&root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("writer.lock"))?;
        lock.try_lock().map_err(|error| {
            invalid(format!("Local recovery storage is in use or cannot be locked: {error}. Close other Folio instances before restarting."))
        })?;
        let enabled = match fs::read(root.join("enabled")) {
            Ok(bytes) if bytes == b"1" => true,
            Ok(bytes) if bytes == b"0" => false,
            Ok(_) => {
                return Err(invalid(
                    "The recovery preference is damaged. No PDFs were loaded or written.",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let store = Self {
            root,
            _lock: lock,
            enabled,
            current: BTreeMap::new(),
        };
        store.clean_temporary_files()?;
        Ok(store)
    }

    fn clean_temporary_files(&self) -> Result<()> {
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".folio-recovery-tmp-")
                && entry.file_type()?.is_file()
            {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }

    fn path(&self, token: &str) -> Result<PathBuf> {
        if token.len() != 32
            || !token
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(invalid("This recovery identifier is invalid."));
        }
        Ok(self.root.join(format!("{token}.folio-recovery")))
    }

    fn files(&self) -> Result<Vec<(String, PathBuf)>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "folio-recovery") {
                let token = path
                    .file_stem()
                    .and_then(|v| v.to_str())
                    .ok_or_else(|| invalid("An unrecognized file is in recovery storage."))?
                    .to_owned();
                self.path(&token)?;
                if !entry.file_type()?.is_file() {
                    return Err(invalid(
                        "Recovery storage contains a non-regular file. No copy was read.",
                    ));
                }
                files.push((token, path));
            }
        }
        Ok(files)
    }

    fn header(path: &Path) -> Result<(File, Header)> {
        if !fs::symlink_metadata(path)?.file_type().is_file() {
            return Err(invalid(
                "Recovery copies must be regular files, not symbolic links.",
            ));
        }
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        if size > (MAX_DOCUMENT_BYTES + MAX_HEADER + 13) as u64 {
            return Err(invalid("This recovery copy exceeds its size limit."));
        }
        let mut magic = [0; 9];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(invalid("This recovery copy has an unsupported format."));
        }
        let mut length = [0; 4];
        file.read_exact(&mut length)?;
        let length = u32::from_le_bytes(length) as usize;
        if length > MAX_HEADER {
            return Err(invalid("This recovery copy has an invalid header."));
        }
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        let header: Header = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(format!("The recovery header is damaged: {error}")))?;
        if header.name.len() > 4096
            || header.size > MAX_DOCUMENT_BYTES
            || size != (13 + length + header.size) as u64
        {
            return Err(invalid(
                "This recovery copy is incomplete or exceeds its limits.",
            ));
        }
        Ok((file, header))
    }

    pub fn status(&self) -> Result<Status> {
        let mut entries = Vec::new();
        for (token, path) in self.files()? {
            if self.current.values().any(|(owned, _)| owned == &token) {
                continue;
            }
            let metadata = fs::metadata(&path)?;
            let (name, problem) = match Self::header(&path) {
                Ok((_, header)) => (header.name, None),
                Err(error) => ("Unreadable recovery copy".into(), Some(error.to_string())),
            };
            entries.push(Entry {
                token,
                name,
                byte_length: metadata.len(),
                modified: metadata
                    .modified()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| invalid("The recovery timestamp is invalid."))?
                    .as_secs(),
                problem,
            });
        }
        entries.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.token.cmp(&b.token)));
        Ok(Status {
            available: true,
            enabled: self.enabled,
            location: Some(self.root.to_string_lossy().into_owned()),
            entries,
            checkpointed: self.current.len(),
            ..Status::default()
        })
    }

    pub fn set_enabled(&mut self, enabled: bool) -> Result<()> {
        // Disable first: even if cleanup fails, no subsequent action may write PDF data.
        atomic_write(
            &self.root.join("enabled"),
            if enabled { b"1" } else { b"0" },
        )?;
        self.enabled = enabled;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        if !enabled {
            self.current.clear();
            self.clean_temporary_files()?;
            for (_, path) in self.files()? {
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    pub fn sync(&mut self, eligible: &[&Session]) -> Result<()> {
        let obsolete: Vec<_> = self
            .current
            .iter()
            .filter(|(id, _)| !eligible.iter().any(|session| session.id == **id))
            .map(|(id, (token, _))| (*id, token.clone()))
            .collect();
        for (id, token) in obsolete {
            self.discard_file(&token)?;
            self.current.remove(&id);
        }
        if !self.enabled {
            return Ok(());
        }
        for session in eligible {
            if !session.recovery_allowed() {
                return Err(invalid(
                    "Protected and read-only PDFs must never enter recovery storage.",
                ));
            }
            if session.name.len() > 4096 {
                return Err(invalid("This PDF's recovery name exceeds its 4 KiB limit."));
            }
            if self
                .current
                .get(&session.id)
                .is_some_and(|(_, rev)| *rev == session.revision())
            {
                continue;
            }
            let token = match self.current.get(&session.id) {
                Some((token, _)) => token.clone(),
                None => {
                    let mut random = [0; 16];
                    getrandom::fill(&mut random).map_err(|error| {
                        invalid(format!("Recovery identifier generation failed: {error}"))
                    })?;
                    random.iter().map(|byte| format!("{byte:02x}")).collect()
                }
            };
            let path = self.path(&token)?;
            let files = self.files()?;
            let used = files.iter().try_fold(0_u64, |sum, (_, path)| {
                fs::metadata(path).map(|meta| sum + meta.len())
            })?;
            if (!path.exists() && files.len() >= MAX_COPIES)
                || used + session.bytes().len() as u64 + MAX_HEADER as u64 + 13 > MAX_STORAGE
            {
                return Err(invalid(
                    "Recovery storage reached its 16-copy / 256 MiB limit. Save PDFs or discard old recovery copies; recent edits are not checkpointed.",
                ));
            }
            let header = Header {
                name: session.name.clone(),
                source: session
                    .source_path
                    .as_ref()
                    .map(std::path::absolute)
                    .transpose()?,
                saved: session
                    .saved_path
                    .as_ref()
                    .map(std::path::absolute)
                    .transpose()?,
                size: session.bytes().len(),
                checksum: crc32fast::hash(session.bytes()),
                originals: session
                    .recovery_originals()
                    .iter()
                    .map(std::path::absolute)
                    .collect::<std::io::Result<_>>()?,
            };
            let header = serde_json::to_vec(&header).map_err(|error| invalid(error.to_string()))?;
            if header.len() > MAX_HEADER {
                return Err(invalid("This PDF's recovery metadata exceeds its limit."));
            }
            let mut temporary = tempfile::Builder::new()
                .prefix(".folio-recovery-tmp-")
                .tempfile_in(&self.root)?;
            temporary.write_all(MAGIC)?;
            temporary.write_all(&(header.len() as u32).to_le_bytes())?;
            temporary.write_all(&header)?;
            temporary.write_all(session.bytes())?;
            temporary.as_file().sync_all()?;
            temporary
                .persist(path)
                .map_err(|error| Error::Io(error.error))?;
            #[cfg(unix)]
            File::open(&self.root)?.sync_all()?;
            self.current.insert(session.id, (token, session.revision()));
        }
        Ok(())
    }

    pub fn read(&self, token: &str, id: u64) -> Result<Session> {
        if self.current.values().any(|(owned, _)| owned == token) {
            return Err(invalid("This recovery copy is already open."));
        }
        let (file, header) = Self::header(&self.path(token)?)?;
        let mut bytes = Vec::with_capacity(header.size);
        file.take(MAX_DOCUMENT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != header.size || crc32fast::hash(&bytes) != header.checksum {
            return Err(invalid(
                "This recovery copy is damaged. It was not opened; you can discard it.",
            ));
        }
        let mut session = Session::recovered(
            bytes,
            header.name,
            header.source,
            header.saved,
            header.originals,
            id,
        )?;
        session.name = format!("{} (recovered).pdf", session.name.trim_end_matches(".pdf"));
        Ok(session)
    }

    pub fn adopt(&mut self, token: String, session: &Session) {
        self.current.insert(session.id, (token, session.revision()));
    }

    fn discard_file(&self, token: &str) -> Result<()> {
        match fs::remove_file(self.path(token)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::Io(error)),
        }?;
        #[cfg(unix)]
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    pub fn discard(&mut self, token: &str) -> Result<()> {
        if self.current.values().any(|(owned, _)| owned == token) {
            return Err(invalid(
                "Close or save the recovered tab before discarding its checkpoint.",
            ));
        }
        self.discard_file(token)
    }

    pub fn clean_exit(&mut self) -> Result<()> {
        let tokens: Vec<_> = self
            .current
            .values()
            .map(|(token, _)| token.clone())
            .collect();
        for token in tokens {
            self.discard_file(&token)?;
        }
        self.current.clear();
        Ok(())
    }
}
