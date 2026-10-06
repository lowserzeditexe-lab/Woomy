//! MSVC discovery uses Visual Studio Setup Configuration through find-msvc-tools.
//! Commands have fixed filenames/arguments and no shell. Compiler outputs are
//! first created in a fresh private build directory, then published by the caller.
use crate::hash;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRecord {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
    pub timed_out: bool,
    pub output_truncated: bool,
}
pub(crate) struct Captured {
    pub record: ProcessRecord,
    pub stdout: Vec<u8>,
}
fn drain(mut stream: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    let mut truncated = false;
    loop {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        let keep = n.min(limit.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&buffer[..keep]);
        truncated |= keep < n;
    }
    Ok((bytes, truncated))
}
pub(crate) fn execute(
    mut command: Command,
    input: Vec<u8>,
    limit: usize,
    timeout: Duration,
) -> io::Result<Captured> {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let start = Instant::now();
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let writer = thread::spawn(move || stdin.write_all(&input));
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || drain(stdout, limit));
    let err = thread::spawn(move || drain(stderr, 1024 * 1024));
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            child.kill()?;
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    // A failed/early-exiting child may close stdin before consuming it; its exit
    // and diagnostic output are the meaningful result in that case.
    let _ = writer.join();
    let (stdout, a) = out
        .join()
        .map_err(|_| io::Error::other("stdout reader failed"))??;
    let (stderr, b) = err
        .join()
        .map_err(|_| io::Error::other("stderr reader failed"))??;
    Ok(Captured {
        record: ProcessRecord {
            exit_code: status.code(),
            stdout: if limit <= 4 * 1024 * 1024 {
                String::from_utf8_lossy(&stdout).into_owned()
            } else {
                String::new()
            },
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            duration_ms: start.elapsed().as_millis(),
            timed_out,
            output_truncated: a || b,
        },
        stdout,
    })
}
pub struct NativeCompiler {
    compiler: find_msvc_tools::Tool,
    linker: find_msvc_tools::Tool,
    pub version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildRecord {
    pub status: String,
    pub compiler_version: String,
    pub source_hash: String,
    pub output_hash: Option<String>,
    pub duration_ms: u128,
    pub compile: Option<ProcessRecord>,
    pub link: Option<ProcessRecord>,
}
pub struct Build {
    pub record: BuildRecord,
    pub executable: Option<Vec<u8>>,
}
fn command(tool: &find_msvc_tools::Tool) -> Command {
    let mut cmd = tool.to_command();
    // CL/LINK accept implicit options through environment variables. Do not let
    // inherited options redirect generated outputs or inject response files.
    for key in ["CL", "_CL_", "LINK", "_LINK_"] {
        cmd.env_remove(key);
    }
    cmd
}
fn compiler_path(path: &Path) -> std::ffi::OsString {
    // MSVC's command parser does not accept Rust's canonical verbatim paths.
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{unc}").into();
    }
    if let Some(local) = text.strip_prefix("\\\\?\\") {
        return local.into();
    }
    path.as_os_str().to_owned()
}
impl NativeCompiler {
    /// Build an explicitly selected external C++ runtime with our adapter.
    /// Sources remain separate translation units; no Cafe objects cross Rust FFI.
    pub fn build_adapter(
        &self,
        source: &str,
        includes: &[std::path::PathBuf],
        units: &[std::path::PathBuf],
        owned_directory: &Path,
    ) -> io::Result<Build> {
        if source.len() > 1024 * 1024 || includes.len() > 8 || units.len() > 32 {
            return Err(io::Error::other("adapter build budget"));
        }
        let start = Instant::now();
        let temp = tempfile::Builder::new()
            .prefix("adapter-")
            .tempdir_in(owned_directory)?;
        std::fs::write(temp.path().join("adapter.cpp"), source)?;
        let mut cmd = command(&self.compiler);
        cmd.current_dir(temp.path()).args([
            "/nologo",
            "/std:c++20",
            "/EHsc",
            "/MT",
            "/O2",
            "/W4",
            "/WX",
            "/c",
            "adapter.cpp",
        ]);
        for path in includes {
            cmd.arg("/I").arg(compiler_path(path));
        }
        let mut objects = vec!["adapter.obj".to_owned()];
        for path in units {
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| io::Error::other("unit filename"))?;
            let object = format!("{name}.obj");
            if objects.contains(&object) {
                return Err(io::Error::other("duplicate unit basename"));
            }
            objects.push(object);
            cmd.arg(compiler_path(path));
        }
        let compiled = execute(cmd, vec![], 4 * 1024 * 1024, Duration::from_secs(120))?;
        let ok = compiled.record.exit_code == Some(0) && !compiled.record.timed_out;
        let mut record = BuildRecord {
            status: "COMPILER_ERROR".into(),
            compiler_version: self.version.clone(),
            source_hash: hash(source.as_bytes()),
            output_hash: None,
            duration_ms: 0,
            compile: Some(compiled.record),
            link: None,
        };
        let mut executable = None;
        if ok {
            let mut cmd = command(&self.linker);
            cmd.current_dir(temp.path())
                .args([
                    "/NOLOGO",
                    "/MACHINE:X64",
                    "/INCREMENTAL:NO",
                    "/BREPRO",
                    "/OUT:adapter.exe",
                ])
                .args(objects);
            let linked = execute(cmd, vec![], 4 * 1024 * 1024, Duration::from_secs(120))?;
            if linked.record.exit_code == Some(0) && !linked.record.timed_out {
                let path = temp.path().join("adapter.exe");
                if std::fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
                    return Err(io::Error::other("adapter binary budget"));
                }
                let bytes = std::fs::read(path)?;
                record.output_hash = Some(hash(&bytes));
                record.status = "COMPILED_NOT_SEMANTICALLY_VALIDATED".into();
                executable = Some(bytes);
            }
            record.link = Some(linked.record);
        }
        record.duration_ms = start.elapsed().as_millis();
        Ok(Build { record, executable })
    }
    pub fn detect() -> Result<Self, String> {
        Self::detect_target("x86_64-pc-windows-msvc")
    }
    pub fn detect_target(target: &str) -> Result<Self, String> {
        if !cfg!(windows) || target != "x86_64-pc-windows-msvc" {
            return Err("TOOLCHAIN_NOT_FOUND: only Windows x64 MSVC is supported".into());
        }
        let compiler =
            find_msvc_tools::find_tool(target, "cl.exe").ok_or("TOOLCHAIN_NOT_FOUND: cl.exe")?;
        let linker = find_msvc_tools::find_tool(target, "link.exe")
            .ok_or("TOOLCHAIN_NOT_FOUND: link.exe")?;
        let mut cmd = command(&compiler);
        cmd.arg("/Bv");
        let captured = execute(cmd, vec![], 1024 * 1024, Duration::from_secs(15))
            .map_err(|e| format!("TOOLCHAIN_NOT_FOUND: {e}"))?;
        if captured.record.timed_out {
            return Err("TOOLCHAIN_NOT_FOUND: version query timed out".into());
        }
        let version = format!("{}{}", captured.record.stdout, captured.record.stderr)
            .lines()
            .filter(|l| l.contains("Compiler Version") || l.contains("compilateur"))
            .take(1)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Self {
            compiler,
            linker,
            version: if version.is_empty() {
                "MSVC; version query unavailable".into()
            } else {
                version
            },
        })
    }
    pub fn build(&self, source: &str, owned_directory: &Path) -> io::Result<Build> {
        if source.len() > 32 * 1024 * 1024 {
            return Err(io::Error::other("C++ source budget exceeded"));
        }
        let start = Instant::now();
        let temp = tempfile::Builder::new()
            .prefix("native-")
            .tempdir_in(owned_directory)?;
        std::fs::write(temp.path().join("function.cpp"), source)?;
        let mut record = BuildRecord {
            status: "COMPILER_ERROR".into(),
            compiler_version: self.version.clone(),
            source_hash: hash(source.as_bytes()),
            output_hash: None,
            duration_ms: 0,
            compile: None,
            link: None,
        };
        let mut cmd = command(&self.compiler);
        cmd.current_dir(temp.path()).args([
            "/nologo",
            "/std:c++17",
            "/EHsc",
            "/MT",
            "/O2",
            "/W4",
            "/c",
            "function.cpp",
            "/Fofunction.obj",
        ]);
        let compiled = execute(cmd, vec![], 4 * 1024 * 1024, Duration::from_secs(120))?;
        let ok = compiled.record.exit_code == Some(0) && !compiled.record.timed_out;
        record.compile = Some(compiled.record);
        let mut executable = None;
        if ok {
            let mut cmd = command(&self.linker);
            cmd.current_dir(temp.path()).args([
                "/NOLOGO",
                "/MACHINE:X64",
                "/INCREMENTAL:NO",
                "/BREPRO",
                "/OUT:function.exe",
                "function.obj",
            ]);
            let linked = execute(cmd, vec![], 4 * 1024 * 1024, Duration::from_secs(120))?;
            if linked.record.exit_code == Some(0) && !linked.record.timed_out {
                let path = temp.path().join("function.exe");
                if std::fs::metadata(&path)?.len() > 64 * 1024 * 1024 {
                    return Err(io::Error::other("Native binary budget exceeded"));
                }
                let bytes = std::fs::read(path)?;
                record.output_hash = Some(hash(&bytes));
                record.status = "COMPILED_NOT_SEMANTICALLY_VALIDATED".into();
                executable = Some(bytes);
            }
            record.link = Some(linked.record);
        }
        record.duration_ms = start.elapsed().as_millis();
        Ok(Build { record, executable })
    }
}
