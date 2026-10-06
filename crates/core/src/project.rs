use crate::{database, Error, Result};
use fs2::FileExt;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU8, Ordering},
    time::UNIX_EPOCH,
};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectInfo {
    pub name: String,
    pub format_version: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DumpConfig {
    pub root: String,
    pub code: String,
    pub content: String,
    pub meta: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub platform: String,
    pub architecture: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfig {
    pub auto_resume: bool,
    pub worker_count: String,
    pub performance: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiConfig {
    pub enabled: bool,
    pub provider: String,
    pub model: String,
    pub max_context_bytes: usize,
    pub max_output_tokens: u32,
    pub max_requests_session: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub project: ProjectInfo,
    pub dump: DumpConfig,
    pub target: Target,
    pub analysis: AnalysisConfig,
    pub ai: AiConfig,
}
impl ProjectConfig {
    fn new(name: String) -> Self {
        Self {
            project: ProjectInfo {
                name,
                format_version: 1,
            },
            dump: DumpConfig {
                root: "..".into(),
                code: "../code".into(),
                content: "../content".into(),
                meta: "../meta".into(),
            },
            target: Target {
                platform: "windows".into(),
                architecture: "x86_64".into(),
            },
            analysis: AnalysisConfig {
                auto_resume: true,
                worker_count: "1".into(),
                performance: "balanced".into(),
            },
            ai: AiConfig {
                enabled: false,
                provider: String::new(),
                model: String::new(),
                max_context_bytes: 32_768,
                max_output_tokens: 2048,
                max_requests_session: 10,
            },
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.project.format_version != 1
            || self.dump.root != ".."
            || self.dump.code != "../code"
            || self.dump.content != "../content"
            || self.dump.meta != "../meta"
        {
            return Err(Error::Config("unsupported format or dump paths".into()));
        }
        if !["eco", "balanced", "maximum"].contains(&self.analysis.performance.as_str())
            || self.analysis.worker_count != "1"
        {
            return Err(Error::Config(
                "this milestone uses one bounded analysis worker".into(),
            ));
        }
        if !(1024..=131_072).contains(&self.ai.max_context_bytes)
            || !(128..=8192).contains(&self.ai.max_output_tokens)
            || !(1..=100).contains(&self.ai.max_requests_session)
        {
            return Err(Error::Config("AI resource limits out of range".into()));
        }
        if !["", "openai", "mock"].contains(&self.ai.provider.as_str()) {
            return Err(Error::Config("unsupported AI provider".into()));
        }
        Ok(())
    }
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct Metadata {
    pub name: Option<String>,
    pub title_id: Option<String>,
    pub region: Option<String>,
    pub version: Option<String>,
}

/// All application writes go through this canonical root, never dump paths.
pub struct SafeOutput {
    root: PathBuf,
}
impl SafeOutput {
    pub fn new(root: &Path) -> Result<Self> {
        reject_link(root)?;
        let root = fs::canonicalize(root)?;
        Ok(Self { root })
    }
    pub fn path(&self, relative: &str) -> Result<PathBuf> {
        let p = Path::new(relative);
        if p.components().any(|c| !matches!(c, Component::Normal(_)))
            || relative.contains(':')
            || relative.contains('\\')
            || relative.is_empty()
        {
            return Err(Error::Project("unsafe output path".into()));
        }
        let mut out = self.root.clone();
        for c in p.components() {
            out.push(c);
            if out.exists() || fs::symlink_metadata(&out).is_ok() {
                reject_link(&out)?;
                reject_hardlink(&out)?;
                if !fs::canonicalize(&out)?.starts_with(&self.root) {
                    return Err(Error::Project("output escapes .woomy".into()));
                }
            }
        }
        Ok(out)
    }
    pub fn write_atomic(&self, relative: &str, data: &[u8]) -> Result<()> {
        let dest = self.path(relative)?;
        let parent = dest
            .parent()
            .ok_or_else(|| Error::Project("output parent".into()))?;
        let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
        tmp.write_all(data)?;
        tmp.as_file().sync_all()?;
        tmp.persist(dest).map_err(|e| Error::Io(e.error))?;
        Ok(())
    }
}
fn reject_hardlink(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let file = File::open(path)?;
        if winapi_util::file::information(&file)?.number_of_links() > 1 {
            return Err(Error::Project("hard-linked output file refused".into()));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(path)?.nlink() > 1 {
            return Err(Error::Project("hard-linked output file refused".into()));
        }
    }
    Ok(())
}
fn reject_link(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(Error::Project("reparse point refused".into()));
        }
    }
    if meta.file_type().is_symlink() {
        return Err(Error::Project("symbolic link refused".into()));
    }
    Ok(())
}
pub struct ProjectService {
    pub root: PathBuf,
    pub output: SafeOutput,
    pub config: ProjectConfig,
    pub metadata: Metadata,
    pub db: Connection,
    _lock: File,
}
impl ProjectService {
    pub fn detect(root: &Path) -> bool {
        ["code", "content", "meta"]
            .iter()
            .all(|p| root.join(p).is_dir() && reject_link(&root.join(p)).is_ok())
    }
    pub fn exists(root: &Path) -> bool {
        root.join(".woomy/project.toml").is_file()
    }
    pub fn metadata(root: &Path) -> Result<Metadata> {
        let path = root.join("meta/meta.xml");
        if !path.exists() {
            return Ok(Metadata::default());
        }
        reject_link(&path)?;
        let xml = read_bounded(&path, 1024 * 1024)?;
        let xml = std::str::from_utf8(&xml)
            .map_err(|_| Error::Project("invalid metadata encoding".into()))?;
        let doc = roxmltree::Document::parse(xml)
            .map_err(|_| Error::Project("invalid metadata XML".into()))?;
        let field = |name: &str| {
            doc.descendants()
                .find(|n| n.has_tag_name(name))
                .and_then(|n| n.text())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Ok(Metadata {
            name: field("longname_en")
                .or_else(|| field("shortname_en"))
                .or_else(|| {
                    doc.descendants()
                        .find(|n| n.tag_name().name().starts_with("longname_"))
                        .and_then(|n| n.text())
                        .map(str::to_owned)
                }),
            title_id: field("title_id"),
            region: field("region"),
            version: field("title_version"),
        })
    }
    pub fn create(root: &Path) -> Result<Self> {
        if !Self::detect(root) {
            return Err(Error::Project(
                "No Wii U dump detected. Expected code/, content/, meta/.".into(),
            ));
        }
        let root = fs::canonicalize(root)?;
        let dir = root.join(".woomy");
        if !dir.exists() {
            fs::create_dir(&dir)?;
        }
        let output = SafeOutput::new(&dir)?;
        // A failed initial setup may leave only these directories. Never overwrite an existing project.
        if output.path("project.toml")?.exists() {
            return Err(Error::Project("project already exists".into()));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(output.path("project.lock")?)?;
        lock.try_lock_exclusive()
            .map_err(|_| Error::Project("project is already open".into()))?;
        for folder in [
            "cache",
            "generated",
            "adaptations",
            "logs",
            "checkpoints",
            "reports",
        ] {
            let p = output.path(folder)?;
            if !p.exists() {
                fs::create_dir(p)?;
            }
        }
        let metadata = Self::metadata(&root)?;
        let config = ProjectConfig::new(metadata.name.clone().unwrap_or_else(|| "Unknown".into()));
        check_db_paths(&output)?;
        let mut db = Connection::open(output.path("woomy.db")?)?;
        database::migrate(&mut db)?;
        let project = Self {
            root,
            output,
            config,
            metadata,
            db,
            _lock: lock,
        };
        project.save_config()?;
        project.log("project created")?;
        Ok(project)
    }
    pub fn open(root: &Path) -> Result<Self> {
        if !Self::detect(root) {
            return Err(Error::Project("missing or unsafe dump directories".into()));
        }
        let root = fs::canonicalize(root)?;
        let output = SafeOutput::new(&root.join(".woomy"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(output.path("project.lock")?)?;
        lock.try_lock_exclusive().map_err(|_| {
            Error::Project("project is already open in another Woomy process".into())
        })?;
        let text = read_bounded(&output.path("project.toml")?, 64 * 1024)?;
        let config: ProjectConfig = toml::from_str(
            std::str::from_utf8(&text)
                .map_err(|_| Error::Config("invalid TOML encoding".into()))?,
        )
        .map_err(|_| Error::Config("invalid project.toml".into()))?;
        config.validate()?;
        check_db_paths(&output)?;
        if !output.path("woomy.db")?.is_file() {
            return Err(Error::Project("database is missing".into()));
        }
        let mut db = Connection::open(output.path("woomy.db")?)?;
        database::migrate(&mut db)?;
        db.execute(
            "UPDATE analysis_runs SET status='INTERRUPTED' WHERE status='RUNNING'",
            [],
        )?;
        db.execute(
            "UPDATE ai_jobs SET status='WAITING_AI' WHERE status='RUNNING'",
            [],
        )?;
        let metadata = Self::metadata(&root)?;
        let p = Self {
            root,
            output,
            config,
            metadata,
            db,
            _lock: lock,
        };
        p.invalidate_versions()?;
        p.refresh_module_hashes()?;
        Ok(p)
    }
    pub fn save_config(&self) -> Result<()> {
        self.config.validate()?;
        let text = toml::to_string_pretty(&self.config)
            .map_err(|_| Error::Config("cannot encode config".into()))?;
        self.output.write_atomic("project.toml", text.as_bytes())
    }
    pub fn log(&self, message: &str) -> Result<()> {
        // Callers pass fixed events/counts only. No API bodies, credentials or user input.
        let mut f = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.output.path("logs/woomy.log")?)?;
        writeln!(
            f,
            "{} {message}",
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        )?;
        Ok(())
    }
    pub fn input(&self, relative: &str) -> Result<PathBuf> {
        let p = Path::new(relative);
        if p.components().any(|c| !matches!(c, Component::Normal(_)))
            || relative.contains(':')
            || relative.contains('\\')
        {
            return Err(Error::Project("unsafe input path".into()));
        }
        if !["code", "content", "meta"].iter().any(|s| p.starts_with(s)) {
            return Err(Error::Project("input outside dump".into()));
        }
        let mut result = self.root.clone();
        for c in p.components() {
            result.push(c);
            reject_link(&result)?;
        }
        let result = fs::canonicalize(result)?;
        if !result.starts_with(&self.root) {
            return Err(Error::Project("input escapes dump".into()));
        }
        Ok(result)
    }
    pub fn invalidate_versions(&self) -> Result<()> {
        self.db.execute(
            "DELETE FROM analysis_products WHERE versions!=?1",
            [serde_json::to_string(
                &woomy_ir::runtime::ImageContextVersions::CURRENT,
            )?],
        )?;
        self.db.execute("DELETE FROM function_validation_runs WHERE context_version!=?1 OR json_extract(summary,'$.versions.image')!=?2 OR json_extract(summary,'$.versions.loader')!=?3 OR json_extract(summary,'$.versions.relocation')!=?4", params![woomy_ir::runtime::EXECUTION_CONTEXT_VERSION,woomy_ir::runtime::CAFE_IMAGE_VERSION,woomy_ir::runtime::CAFE_LOADER_VERSION,woomy_ir::runtime::RELOCATION_APPLIER_VERSION])?;
        self.db.execute("UPDATE modules SET status='STALE' WHERE EXISTS(SELECT 1 FROM functions f LEFT JOIN function_summaries s ON s.function=f.id WHERE f.module=modules.id AND (s.function IS NULL OR s.version!=?1 OR s.call_model_version!=?2))",params![woomy_ir::runtime::FUNCTION_SUMMARY_VERSION,woomy_ir::runtime::CALL_MODEL_VERSION])?;
        self.db.execute("UPDATE modules SET status='STALE' WHERE parser_version!=?1 OR decoder_version!=?2 OR cfg_version!=?3 OR dataflow_version!=?4 OR ir_version!=?5",params![woomy_binary::RPX_PARSER_VERSION,woomy_ppc::PPC_DECODER_VERSION,woomy_ppc::CFG_ANALYZER_VERSION,woomy_ppc::DATAFLOW_VERSION,woomy_ir::IR_VERSION])?;
        Ok(())
    }
    pub fn refresh_module_hashes(&self) -> Result<()> {
        let mut stmt = self
            .db
            .prepare("SELECT id,path,hash FROM modules WHERE status!='STALE'")?;
        let modules = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, path, hash) in modules {
            let current = self
                .input(&path)
                .and_then(|p| read_bounded(&p, woomy_binary::MAX_MODULE_BYTES));
            if current
                .as_ref()
                .map(|b| crate::hash_bytes(b))
                .ok()
                .as_deref()
                != Some(&hash)
            {
                self.db
                    .execute("UPDATE modules SET status='STALE' WHERE id=?1", [id])?;
            }
        }
        Ok(())
    }
    pub fn scan(
        &mut self,
        cancel: &AtomicU8,
        mut progress: impl FnMut(&str),
    ) -> Result<ScanSummary> {
        let mut summary = ScanSummary::default();
        let root = self.root.clone();
        let tx = self.db.transaction()?;
        tx.execute("UPDATE files SET seen=0", [])?;
        for folder in ["code", "content", "meta"] {
            for entry in WalkDir::new(root.join(folder))
                .follow_links(false)
                .sort_by_file_name()
                .max_depth(64)
            {
                if cancel.load(Ordering::Relaxed) != 0 {
                    return Err(Error::Cancelled);
                }
                let e = entry.map_err(|_| Error::Project("cannot enumerate dump".into()))?;
                reject_link(e.path())?;
                if e.file_type().is_dir() {
                    if e.depth() == 64 {
                        return Err(Error::Project("dump directory depth limit".into()));
                    }
                    continue;
                }
                if !e.file_type().is_file() {
                    return Err(Error::Project("non-regular dump file".into()));
                }
                let relative = e
                    .path()
                    .strip_prefix(&root)
                    .map_err(|_| Error::Project("scan escaped dump".into()))?
                    .to_str()
                    .ok_or_else(|| Error::Project("non Unicode dump path".into()))?
                    .replace('\\', "/");
                let mut f = File::open(e.path())?;
                let before = f.metadata()?;
                let mut hash = Sha256::new();
                let mut buf = [0u8; 65536];
                let mut size = 0u64;
                loop {
                    if cancel.load(Ordering::Relaxed) != 0 {
                        return Err(Error::Cancelled);
                    }
                    let n = f.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buf[..n]);
                    size += n as u64;
                }
                let after = f.metadata()?;
                if before.len() != size
                    || before.len() != after.len()
                    || before.modified()? != after.modified()?
                {
                    return Err(Error::Analysis("dump changed during scan; retry".into()));
                }
                let mtime = stamp(&after)?;
                let hash = format!("{:x}", hash.finalize());
                let extension = e
                    .path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let kind = match extension.as_str() {
                    "rpx" => "RPX",
                    "rpl" => "RPL",
                    "xml" => "METADATA",
                    _ => "DATA",
                };
                tx.execute("INSERT INTO files VALUES(?1,?2,?3,?4,?5,1) ON CONFLICT(path) DO UPDATE SET size=excluded.size,mtime=excluded.mtime,sha256=excluded.sha256,kind=excluded.kind,seen=1",params![relative,size,mtime,hash,kind])?;
                summary.files += 1;
                summary.bytes += size;
                if kind == "RPX" || kind == "RPL" {
                    summary.modules += 1;
                }
                if summary.files.is_multiple_of(256) {
                    progress(&format!("Scanned {} files", summary.files));
                }
                if summary.files > 1_000_000 {
                    return Err(Error::Analysis("file count limit".into()));
                }
            }
        }
        tx.execute("DELETE FROM files WHERE seen=0", [])?;
        tx.execute("UPDATE modules SET status='STALE' WHERE hash != (SELECT sha256 FROM files WHERE path=modules.path)",[])?;
        tx.commit()?;
        self.log(&format!(
            "scan completed: {} files, {} modules",
            summary.files, summary.modules
        ))?;
        Ok(summary)
    }
    pub fn dump_change_state(&self) -> Result<&'static str> {
        let mut stmt = self
            .db
            .prepare("SELECT path,size,mtime FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut n = 0;
        for row in rows {
            let (path, size, time) = row?;
            let Ok(path) = self.input(&path) else {
                return Ok("CHANGED");
            };
            let m = fs::metadata(path)?;
            if m.len() != size || stamp(&m)? != time {
                return Ok("CHANGED");
            }
            n += 1;
        }
        if n == 0 {
            return Ok("NOT SCANNED");
        }
        let mut count = 0;
        for folder in ["code", "content", "meta"] {
            for e in WalkDir::new(self.root.join(folder))
                .follow_links(false)
                .max_depth(64)
            {
                let e = e.map_err(|_| Error::Project("dump enumeration failed".into()))?;
                reject_link(e.path())?;
                if e.file_type().is_file() {
                    count += 1;
                }
            }
        }
        Ok(if count == n {
            "UNCHANGED (size/mtime; scan verifies SHA-256)"
        } else {
            "CHANGED"
        })
    }
}
fn check_db_paths(output: &SafeOutput) -> Result<()> {
    for p in [
        "woomy.db",
        "woomy.db-wal",
        "woomy.db-shm",
        "woomy.db-journal",
    ] {
        output.path(p)?;
    }
    Ok(())
}
fn stamp(m: &fs::Metadata) -> Result<String> {
    Ok(m.modified()?
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Project("file mtime before epoch".into()))?
        .as_nanos()
        .to_string())
}
pub fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let f = File::open(path)?;
    if f.metadata()?.len() > limit as u64 {
        return Err(Error::Analysis("file exceeds read budget".into()));
    }
    let mut data = Vec::new();
    f.take(limit as u64 + 1).read_to_end(&mut data)?;
    if data.len() > limit {
        return Err(Error::Analysis("file exceeds read budget".into()));
    }
    Ok(data)
}
#[derive(Debug, Default, Serialize)]
pub struct ScanSummary {
    pub files: u64,
    pub bytes: u64,
    pub modules: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_dump_no_project() -> Result<()> {
        let d = tempfile::tempdir()?;
        assert!(ProjectService::create(d.path()).is_err());
        assert!(!d.path().join(".woomy").exists());
        Ok(())
    }
    #[test]
    fn traversal_and_atomic_write() -> Result<()> {
        let d = tempfile::tempdir()?;
        let safe = SafeOutput::new(d.path())?;
        for s in [
            "../code/game.rpx",
            "/absolute",
            "C:/code",
            "a/../../b",
            "a:stream",
            "a\\..\\b",
            "",
        ] {
            assert!(safe.path(s).is_err(), "{s}");
        }
        safe.write_atomic("test", b"first")?;
        safe.write_atomic("test", b"second")?;
        assert_eq!(fs::read(d.path().join("test"))?, b"second");
        Ok(())
    }
    #[test]
    fn setup_reopen_and_scan() -> Result<()> {
        let d = tempfile::tempdir()?;
        for s in ["code", "content", "meta"] {
            fs::create_dir(d.path().join(s))?;
        }
        fs::write(d.path().join("code/test.rpx"), b"fixture")?;
        let mut p = ProjectService::create(d.path())?;
        assert!(ProjectService::open(d.path()).is_err());
        let result = p.scan(&AtomicU8::new(0), |_| {})?;
        assert_eq!(result.modules, 1);
        drop(p);
        let p = ProjectService::open(d.path())?;
        assert!(p.dump_change_state()?.starts_with("UNCHANGED"));
        assert_eq!(fs::read(d.path().join("code/test.rpx"))?, b"fixture");
        Ok(())
    }
}
