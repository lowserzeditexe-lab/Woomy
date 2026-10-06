#[path = "../../../tests/support/mod.rs"]
mod support;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
};
fn run(root: &std::path::Path, words: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_woomy"))
        .arg("--project")
        .arg(root)
        .args(words)
        .output()
        .unwrap()
}
fn dump() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for s in ["code", "content", "meta"] {
        fs::create_dir(d.path().join(s)).unwrap();
    }
    fs::write(d.path().join("code/synthetic.rpx"), support::cafe_fixture()).unwrap();
    d
}
#[test]
fn command_mode_json_exit_codes_and_help() {
    let missing = tempfile::tempdir().unwrap();
    let r = run(missing.path(), &["setup", "--yes", "--no-ai", "--json"]);
    assert_eq!(r.status.code(), Some(3));
    assert!(serde_json::from_slice::<Value>(&r.stdout).is_ok());
    assert!(!missing.path().join(".woomy").exists());
    let d = dump();
    let setup = run(d.path(), &["setup", "--yes", "--no-ai", "--json"]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let status: Value = serde_json::from_slice(&setup.stdout).unwrap();
    assert_eq!(status["files"], 1);
    let r = run(d.path(), &["analyze", "--json"]);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["functions"], 2);
    let r = run(d.path(), &["functions", "--limit", "1", "--json"]);
    let v: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(v["has_more"], true);
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    let function = v["items"][0]["id"].as_str().unwrap();
    for command in [
        "values",
        "definitions",
        "dataflow_status",
        "indirect",
        "indirect_targets",
        "jump_tables",
    ] {
        let r = run(d.path(), &[command, function, "--limit", "2", "--json"]);
        assert!(
            r.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&r.stderr)
        );
        let page: Value = serde_json::from_slice(&r.stdout).unwrap();
        assert!(page["items"].is_array());
        assert!(page["items"].as_array().unwrap().len() <= 2);
    }
    let r = run(d.path(), &["ir", function, "--limit", "1", "--json"]);
    assert!(r.status.success());
    let ir: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(ir["pagination"]["unit"], "blocks");
    let r = run(d.path(), &["verify-ir", function, "--json"]);
    assert!(r.status.success());
    let report: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(report["verification"]["structural_valid"], true);
    assert_eq!(
        report["verification"]["executable_semantics_validated"],
        false
    );
    let r = run(d.path(), &["ir", "coverage", "--json"]);
    assert!(r.status.success());
    let coverage: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(coverage["total"], 2);
    let r = run(d.path(), &["codegen", function, "--json"]);
    assert!(r.status.success());
    let generated: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(generated["status"], "NOT_ELIGIBLE");
    let r = run(d.path(), &["build", "status", function, "--json"]);
    assert!(r.status.success());
    let status: Value = serde_json::from_slice(&r.stdout).unwrap();
    assert_eq!(status["native_executed"], false);
    for command in [
        "help",
        "status",
        "setup",
        "scan",
        "analyze",
        "modules",
        "module",
        "functions",
        "function",
        "disasm",
        "cfg",
        "values",
        "definitions",
        "dataflow_status",
        "indirect",
        "indirect_targets",
        "jump_tables",
        "ir",
        "verify-ir",
        "codegen",
        "build",
        "xref",
        "imports",
        "exports",
        "problems",
        "ai",
        "config",
        "pause",
        "resume",
        "cancel",
        "clear",
        "exit",
    ] {
        let r = run(d.path(), &["help", command, "--json"]);
        assert!(r.status.success());
        let v: Value = serde_json::from_slice(&r.stdout).unwrap();
        assert_eq!(v["commands"].as_array().unwrap().len(), 1);
    }
    assert_eq!(run(d.path(), &["bogus", "--json"]).status.code(), Some(2));
    assert_eq!(
        run(d.path(), &["ai", "test", "--json"]).status.code(),
        Some(5)
    );
}
#[test]
fn first_launch_shell_and_second_launch() {
    let d = dump();
    let mut p = Command::new(env!("CARGO_BIN_EXE_woomy"))
        .arg("--project")
        .arg(d.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin
        .take()
        .unwrap()
        .write_all(b"y\r\ns\r\nstatus\r\nhelp cfg\r\nexit\r\n")
        .unwrap();
    let output = p.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("First-Time Setup"));
    assert!(text.contains("woomy>"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unknown command"));
    assert!(text.contains("cfg <function-id>"));
    let mut p = Command::new(env!("CARGO_BIN_EXE_woomy"))
        .arg("--project")
        .arg(d.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin
        .take()
        .unwrap()
        .write_all(b"status\r\nexit\r\n")
        .unwrap();
    let output = p.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unknown command"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("First-Time Setup"));
}
#[test]
fn eof_does_not_confirm_setup() {
    let d = dump();
    let out = run(d.path(), &[]);
    assert_eq!(out.status.code(), Some(6));
    assert!(!d.path().join(".woomy").exists());
}

#[test]
fn shell_can_pause_running_scan_and_exit_gracefully() {
    let d = dump();
    assert!(run(d.path(), &["setup", "--yes", "--no-ai"])
        .status
        .success());
    let content = fs::File::create(d.path().join("content/synthetic-data.bin")).unwrap();
    content.set_len(16 * 1024 * 1024).unwrap();
    drop(content);
    let mut child = Command::new(env!("CARGO_BIN_EXE_woomy"))
        .arg("--project")
        .arg(d.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"analyze\npause\nstatus\nexit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Pause requested"));
    let status = run(d.path(), &["status", "--json"]);
    assert!(status.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["database"],
        "OK"
    );
}

#[test]
fn forced_exit_during_scan_is_recoverable() {
    let d = dump();
    assert!(run(d.path(), &["setup", "--yes", "--no-ai"])
        .status
        .success());
    let content = fs::File::create(d.path().join("content/synthetic-data.bin")).unwrap();
    content.set_len(64 * 1024 * 1024).unwrap();
    drop(content);
    let mut child = Command::new(env!("CARGO_BIN_EXE_woomy"))
        .arg("--project")
        .arg(d.path())
        .args(["analyze", "--json"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // This progress event follows the durable RUNNING checkpoint and precedes
    // hashing. Terminate only this test process, while its scan is incomplete.
    let mut progress = BufReader::new(child.stderr.take().unwrap());
    let mut line = String::new();
    progress.read_line(&mut line).unwrap();
    assert!(line.contains("[1/4] Scanning"));
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let status = run(d.path(), &["status", "--json"]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["database"], "OK");
    assert_eq!(status["last_run"]["status"], "INTERRUPTED");
    let resumed = run(d.path(), &["resume", "--json"]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&resumed.stdout).unwrap()["parsed_modules"],
        1
    );
    let status = run(d.path(), &["status", "--json"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["last_run"]["status"],
        "COMPLETED"
    );
    assert_eq!(
        fs::read(d.path().join("code/synthetic.rpx")).unwrap(),
        support::cafe_fixture()
    );
}
