//! Woomy-owned adapter build and identity, not an implementation of Cafe services.
use crate::{project::read_bounded, Error, ProjectService, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub const M10_UNITS: [&str; 9] = [
    "abi/src/manmenmi_abi.cpp",
    "cafe/src/guest_object_registry.cpp",
    "cafe/src/cafe_thread.cpp",
    "cafe/src/cafe_process.cpp",
    "cafe/src/service_registry.cpp",
    "cafe/src/cafe_runtime.cpp",
    "cafe/src/coreinit.cpp",
    "core/src/result.cpp",
    "core/src/log.cpp",
];
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterManifest {
    pub bridge_version: u32,
    pub runtime_versions: [u32; 4],
    pub process_versions: manmenmi::ProcessVersions,
    pub source_root: PathBuf,
    pub sources: BTreeMap<String, String>,
    pub adapter_source_hash: String,
    pub executable: String,
    pub executable_hash: String,
    pub compiler_version: String,
}
fn source_hashes(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut paths: Vec<PathBuf> = M10_UNITS.iter().map(|p| root.join(p)).collect();
    for directory in ["core/include", "cafe/include", "abi/include"] {
        for entry in walkdir::WalkDir::new(root.join(directory))
            .follow_links(false)
            .max_depth(8)
            .sort_by_file_name()
        {
            let entry = entry.map_err(|_| Error::Config("M10 header traversal".into()))?;
            if entry.file_type().is_symlink() {
                return Err(Error::Config("M10 source links are not supported".into()));
            }
            if entry.file_type().is_file() {
                paths.push(entry.into_path());
            }
            if paths.len() > 256 {
                return Err(Error::Config("M10 source count budget".into()));
            }
        }
    }
    let mut hashes = BTreeMap::new();
    let mut total = 0;
    for path in paths {
        let bytes = read_bounded(&path, 1024 * 1024)?;
        total += bytes.len();
        if total > 16 * 1024 * 1024 {
            return Err(Error::Config("M10 source byte budget".into()));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| Error::Config("M10 source path".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        hashes.insert(relative, crate::hash_bytes(&bytes));
    }
    Ok(hashes)
}
impl ProjectService {
    pub fn connect_manmenmi(&self, path: &Path) -> Result<Value> {
        let root = std::fs::canonicalize(path)?;
        let sources = source_hashes(&root)?;
        let directory = self.output.path("generated/runtime")?;
        std::fs::create_dir_all(&directory)?;
        self.output.path("generated/runtime")?;
        let compiler =
            woomy_codegen::compiler::NativeCompiler::detect().map_err(Error::Analysis)?;
        let build = compiler.build_adapter(
            manmenmi::bridge::ADAPTER_SOURCE,
            &[
                root.join("cafe/include"),
                root.join("core/include"),
                root.join("abi/include"),
            ],
            &M10_UNITS.iter().map(|p| root.join(p)).collect::<Vec<_>>(),
            &directory,
        )?;
        self.output.write_atomic(
            "reports/manmenmi-adapter-build.json",
            &serde_json::to_vec_pretty(&build.record)?,
        )?;
        let executable = build.executable.ok_or_else(|| {
            Error::Analysis(format!("M10_ADAPTER_BUILD_FAILED: {:?}", build.record))
        })?;
        if source_hashes(&root)? != sources {
            return Err(Error::Analysis("M10_SOURCES_CHANGED_DURING_BUILD".into()));
        }
        let hash = crate::hash_bytes(&executable);
        let output = format!("generated/runtime/{hash}.exe");
        self.output.write_atomic(&output, &executable)?;
        let manifest = AdapterManifest {
            bridge_version: manmenmi::bridge::BRIDGE_VERSION,
            runtime_versions: [1, 1, 1, 1],
            process_versions: manmenmi::ProcessVersions::CURRENT,
            source_root: root,
            sources,
            adapter_source_hash: crate::hash_bytes(manmenmi::bridge::ADAPTER_SOURCE.as_bytes()),
            executable: output,
            executable_hash: hash,
            compiler_version: compiler.version,
        };
        self.output
            .write_atomic("manmenmi.json", &serde_json::to_vec_pretty(&manifest)?)?;
        Ok(
            json!({"status":"ADAPTER_COMPILED_HANDSHAKE_REQUIRED_AT_EXECUTION","manifest":manifest,"guest_memory_owner":"Woomy","runtime_object_owner":"External Manmenmi M10","game_execution":false}),
        )
    }
    pub(crate) fn manmenmi_adapter(&self) -> Result<(AdapterManifest, PathBuf)> {
        let bytes = read_bounded(&self.output.path("manmenmi.json")?, 1024 * 1024)?;
        let m: AdapterManifest = serde_json::from_slice(&bytes)?;
        if m.bridge_version != manmenmi::bridge::BRIDGE_VERSION
            || m.runtime_versions != [1, 1, 1, 1]
            || m.process_versions != manmenmi::ProcessVersions::CURRENT
            || m.adapter_source_hash
                != crate::hash_bytes(manmenmi::bridge::ADAPTER_SOURCE.as_bytes())
            || source_hashes(&m.source_root)? != m.sources
        {
            return Err(Error::Analysis(
                "STALE_MANMENMI_ADAPTER: runtime connect required".into(),
            ));
        }
        let path = self.output.path(&m.executable)?;
        if crate::hash_bytes(&read_bounded(&path, 64 * 1024 * 1024)?) != m.executable_hash {
            return Err(Error::Analysis("M10_ADAPTER_HASH_MISMATCH".into()));
        }
        Ok((m, path))
    }
}
