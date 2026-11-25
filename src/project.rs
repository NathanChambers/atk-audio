use crate::Result;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Project {
    pub root: PathBuf,
}

impl Project {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn directory(&self, kind: &str) -> PathBuf {
        self.root.join(".audio").join(kind)
    }

    fn document_path(&self, kind: &str, id: &str) -> Result<PathBuf> {
        validate_id(id)?;
        Ok(self.directory(kind).join(format!("{id}.json")))
    }

    pub fn document(&self, kind: &str, id: &str) -> Result<Value> {
        let path = self.document_path(kind, id)?;
        read_json(&path)?
            .ok_or_else(|| format!("Unknown {}: {id}", kind.trim_end_matches('s')).into())
    }

    pub fn source_path(&self, hash: &str) -> Result<PathBuf> {
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("Invalid source hash".into());
        }

        Ok(self.directory("sources").join(format!("{hash}.wav")))
    }

    pub fn list(&self) -> Result<Value> {
        Ok(
            json!({"assets": self.list_documents("assets")?, "sessions": self.list_documents("sessions")?}),
        )
    }

    fn list_documents(&self, kind: &str) -> Result<Vec<Value>> {
        let entries = match fs::read_dir(self.directory(kind)) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };

        let mut ids = Vec::new();
        for entry in entries {
            let name = entry?.file_name();
            if let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) {
                ids.push(id.to_owned());
            }
        }
        ids.sort_unstable();

        ids.iter().map(|id| {
            let document = self.document(kind, id)?;
            let revision = document.get("revision").unwrap_or(&document["baseRevision"]);
            Ok(json!({"id": document["id"], "asset": document["asset"]["id"], "name": document["asset"]["name"], "revision": revision}))
        }).collect()
    }

    pub fn update(
        &self,
        session_id: &str,
        command: &Value,
        edit: impl FnOnce(&mut Value) -> Result<Value>,
    ) -> Result<Value> {
        let path = self.document_path("sessions", session_id)?;
        let _lock = self.lock()?;
        match command["command"].as_str() {
            Some("session_start") => self.start(&path, session_id, command),
            Some("session_publish") => self.publish(&path, session_id),
            _ => {
                let mut session = self.document("sessions", session_id)?;
                let result = edit(&mut session["asset"])?;
                let draft_revision = session["draftRevision"].as_u64().unwrap_or(0);
                session["draftRevision"] = json!(draft_revision.checked_add(1).ok_or("Draft revision overflow")?);
                write_json(&path, &session)?;
                Ok(result)
            }
        }
    }

    fn start(&self, path: &Path, session_id: &str, command: &Value) -> Result<Value> {
        let asset_id = command["asset"].as_str().unwrap_or_default();
        let published_path = self.document_path("assets", asset_id)?;
        if read_json(path)?.is_some() {
            return Err(format!("Session already exists: {session_id}").into());
        }

        let published = read_json(&published_path)?;
        let (asset, revision) = match published {
            Some(document) => (
                document["asset"].clone(),
                document["revision"].as_u64().unwrap_or(0),
            ),
            None => {
                let name = match &command["name"] {
                    Value::Null => asset_id,
                    Value::String(name) if !name.trim().is_empty() => name,
                    _ => return Err("Expected a name".into()),
                };
                (
                    json!({"id": asset_id, "name": name, "sources": [], "tracks": [], "controls": [], "mappings": []}),
                    0,
                )
            }
        };
        let session =
            json!({"version": 1, "id": session_id, "baseRevision": revision, "draftRevision": 0, "asset": asset});
        write_json(path, &session)?;

        Ok(session)
    }

    fn publish(&self, path: &Path, session_id: &str) -> Result<Value> {
        let mut session = self.document("sessions", session_id)?;
        let asset_id = session["asset"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let published_path = self.document_path("assets", &asset_id)?;
        let published = read_json(&published_path)?.unwrap_or(Value::Null);
        let revision = published["revision"].as_u64().unwrap_or(0);
        let previous_publish = session["lastPublication"].is_string()
            && published["publication"] == session["lastPublication"];
        if session["baseRevision"].as_u64() != Some(revision) && !previous_publish {
            return Err(format!(
                "Publish conflict: asset advanced to revision {revision}. Your draft is saved"
            )
            .into());
        }

        let next_revision = revision.checked_add(1).ok_or("Asset revision overflow")?;
        let publication = unique_name();
        session["baseRevision"] = json!(revision);
        session["lastPublication"] = json!(publication);
        write_json(path, &session)?;

        let next = json!({"version": 1, "id": asset_id, "revision": next_revision, "publication": publication, "asset": session["asset"]});
        write_json(&published_path, &next)?;
        session["baseRevision"] = json!(next_revision);
        write_json(path, &session)?;

        Ok(next)
    }

    pub fn store_source(&self, source: &Value, bytes: &[u8]) -> Result<()> {
        let hash = source["hash"].as_str().unwrap_or_default();
        let path = self.source_path(hash)?;
        if !path.try_exists()? {
            write_atomic(&path, bytes)?;
        }

        Ok(())
    }

    fn lock(&self) -> Result<ProjectLock> {
        for kind in ["assets", "sessions", "sources"] {
            fs::create_dir_all(self.directory(kind))?;
        }

        let path = self.directory("write-lock");
        for attempt in 0..100 {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(ProjectLock(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempt < 99 => {
                    std::thread::sleep(Duration::from_millis(40));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    return Err("Project is busy. Retry after the current write completes".into());
                }
                Err(error) => return Err(error.into()),
            }
        }

        unreachable!("the final attempt returns")
    }
}

struct ProjectLock(PathBuf);

impl Drop for ProjectLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.0);
    }
}

pub(crate) fn validate_id(id: &str) -> Result<()> {
    let alphanumeric = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let valid = id.len() <= 64
        && id.bytes().next().is_some_and(alphanumeric)
        && id
            .bytes()
            .all(|byte| alphanumeric(byte) || matches!(byte, b'_' | b'-'));
    if !valid {
        return Err("Use a lowercase ID with letters, numbers, underscores or hyphens".into());
    }

    Ok(())
}

fn read_json(path: &Path) -> Result<Option<Value>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", unique_name()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;

    let written = file.write_all(bytes);
    drop(file);
    let result = written.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }

    Ok(result?)
}

pub fn unique_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{time}-{sequence}", std::process::id())
}
