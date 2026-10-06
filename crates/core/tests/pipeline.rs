#[path = "../../../tests/support/mod.rs"]
mod support;
use std::{fs, sync::atomic::AtomicU8};
use woomy_core::{AnalysisService, Error, ProjectService, Result};
fn setup() -> Result<(tempfile::TempDir, ProjectService)> {
    let d = tempfile::tempdir()?;
    for folder in ["code", "content", "meta"] {
        fs::create_dir(d.path().join(folder))?;
    }
    fs::write(d.path().join("code/synthetic.rpx"), support::cafe_fixture())?;
    let p = ProjectService::create(d.path())?;
    Ok((d, p))
}
#[test]
fn vertical_analysis_resume_invalidation_and_original_integrity() -> Result<()> {
    let (d, mut p) = setup()?;
    let before = fs::read(d.path().join("code/synthetic.rpx"))?;
    let cancel = AtomicU8::new(0);
    let result = AnalysisService::run(&mut p, &cancel, |_| {})?;
    assert_eq!(result.parsed_modules, 1);
    assert_eq!(result.functions, 2);
    assert_eq!(
        p.list("imports", "", 100, 0)?.items[0]["symbol"],
        "OSReport"
    );
    let functions = p.list("functions", "", 100, 0)?.items;
    let f = functions[0]["id"].as_str().unwrap();
    assert!(f.ends_with("00001000"));
    assert!(p
        .list("evidence", f, 100, 0)?
        .items
        .iter()
        .any(|e| e["kind"] == "ENTRY_POINT"));
    assert!(!p.list("cfg", f, 100, 0)?.items.is_empty());
    assert!(!p.list("callees", f, 100, 0)?.items.is_empty());
    assert!(!p.list("dataflow", f, 100, 0)?.items.is_empty());
    let id = f.to_owned();
    let next = AnalysisService::run(&mut p, &cancel, |_| {})?;
    assert_eq!(next.cached_modules, 1);
    p.db.execute("INSERT INTO analysis_runs(status) VALUES('RUNNING')", [])?;
    drop(p);
    let mut p = ProjectService::open(d.path())?;
    assert!(p.interrupted()?);
    let result = AnalysisService::run(&mut p, &cancel, |_| {})?;
    assert_eq!(result.cached_modules, 1);
    assert!(!p.interrupted()?);
    p.db.execute("UPDATE modules SET decoder_version=0", [])?;
    p.invalidate_versions()?;
    assert!(p.require_function(&id).is_err());
    let result = AnalysisService::run(&mut p, &cancel, |_| {})?;
    assert_eq!(result.parsed_modules, 1);
    assert!(p.require_function(&id).is_ok());
    assert_eq!(before, fs::read(d.path().join("code/synthetic.rpx"))?);
    fs::remove_file(d.path().join("code/synthetic.rpx"))?;
    p.scan(&cancel, |_| {})?;
    assert!(p.list("modules", "", 100, 0)?.items.is_empty());
    Ok(())
}
#[test]
fn cancelled_scan_rolls_back() -> Result<()> {
    let (d, mut p) = setup()?;
    p.scan(&AtomicU8::new(0), |_| {})?;
    assert!(matches!(
        p.scan(&AtomicU8::new(1), |_| {}),
        Err(Error::Cancelled)
    ));
    assert_eq!(p.status()?["files"], 1);
    assert!(matches!(
        AnalysisService::run(&mut p, &AtomicU8::new(2), |_| {}),
        Err(Error::Cancelled)
    ));
    assert_eq!(p.list("runs", "", 1, 0)?.items[0]["status"], "PAUSED");
    drop(p);
    let mut p = ProjectService::open(d.path())?;
    assert!(p.interrupted()?);
    assert_eq!(
        AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?.parsed_modules,
        1
    );
    assert!(!p.interrupted()?);
    Ok(())
}
#[test]
fn malformed_module_is_recorded_and_no_panic() -> Result<()> {
    let (d, mut p) = setup()?;
    fs::write(d.path().join("code/synthetic.rpx"), b"bad")?;
    assert!(AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {}).is_err());
    assert_eq!(
        p.list("problems", "", 100, 0)?.items[0]["category"],
        "PARSE"
    );
    Ok(())
}
#[test]
fn scan_hash_change_marks_stale() -> Result<()> {
    let (d, mut p) = setup()?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    let mut bytes = support::cafe_fixture();
    bytes.push(0);
    fs::write(d.path().join("code/synthetic.rpx"), bytes)?;
    p.scan(&AtomicU8::new(0), |_| {})?;
    assert_eq!(p.status()?["stale_modules"], 1);
    assert!(p.list("functions", "", 100, 0)?.items.is_empty());
    Ok(())
}
#[test]
fn metadata_unknown_and_xml_entities() -> Result<()> {
    let (d, p) = setup()?;
    drop(p);
    fs::write(
        d.path().join("meta/meta.xml"),
        b"<menu><longname_en>Test &amp; Demo</longname_en><title_id>1234</title_id></menu>",
    )?;
    let m = ProjectService::metadata(d.path())?;
    assert_eq!(m.name.as_deref(), Some("Test & Demo"));
    assert!(m.version.is_none());
    Ok(())
}
#[test]
fn cancellation_during_module_leaves_no_half_module() -> Result<()> {
    let (_d, mut p) = setup()?;
    let cancel = AtomicU8::new(0);
    let r = AnalysisService::run(&mut p, &cancel, |s| {
        if s.starts_with("[4/4]") {
            cancel.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    assert!(matches!(r, Err(Error::Cancelled)));
    assert!(p.list("modules", "", 100, 0)?.items.is_empty());
    assert_eq!(p.list("runs", "", 1, 0)?.items[0]["status"], "CANCELLED");
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    assert_eq!(p.list("modules", "", 10, 0)?.items.len(), 1);
    Ok(())
}

#[test]
fn hardlink_output_cannot_modify_dump() -> Result<()> {
    let (d, p) = setup()?;
    let source = d.path().join("code/synthetic.rpx");
    let before = fs::read(&source)?;
    fs::hard_link(&source, d.path().join(".woomy/logs/unsafe.log"))?;
    assert!(p.output.path("logs/unsafe.log").is_err());
    assert_eq!(fs::read(source)?, before);
    Ok(())
}

#[test]
fn module_changed_between_launches_is_stale_before_query() -> Result<()> {
    let (d, mut p) = setup()?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    drop(p);
    let mut bytes = support::cafe_fixture();
    bytes.push(0);
    fs::write(d.path().join("code/synthetic.rpx"), bytes)?;
    let p = ProjectService::open(d.path())?;
    assert_eq!(p.status()?["stale_modules"], 1);
    assert!(p.list("functions", "", 10, 0)?.items.is_empty());
    Ok(())
}

#[test]
fn imported_call_has_symbol_but_no_fabricated_address() -> Result<()> {
    let (d, mut p) = setup()?;
    let mut data = support::cafe_fixture();
    let m = woomy_binary::parse(&data)?;
    let reloc = m.sections[7].offset as usize;
    support::put32(&mut data, reloc, 0x100c);
    support::put32(&mut data, reloc + 4, 0x20a);
    support::put32(&mut data, 64 + 7 * 40 + 28, 1);
    fs::write(d.path().join("code/synthetic.rpx"), data)?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    let f = p.list("functions", "00001000", 1, 0)?.items[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let calls = p.list("callees", &f, 10, 0)?;
    assert_eq!(calls.items[0]["kind"], "Import");
    assert!(calls.items[0]["target"].is_null());
    assert!(calls.items[0]["evidence"]
        .as_str()
        .unwrap()
        .contains("OSReport"));
    assert!(p
        .list("disasm", &f, 100, 0)?
        .items
        .iter()
        .any(|i| i["address"] == 0x1010));
    Ok(())
}

#[test]
fn indirect_feedback_discovers_callees_and_persists_proofs_and_values() -> Result<()> {
    let (d, mut p) = setup()?;
    let mut bytes = support::cafe_fixture();
    let m = woomy_binary::parse(&bytes)?;
    let text = m.sections[1].offset as usize;
    let symbols = m.sections[4].offset as usize;
    bytes[symbols + 28] = 3; // Section anchor, not a pre-existing function seed.
    let words = [
        0x38601018u32,
        0x7c6903a6,
        0x4e800421,
        0x4e800020,
        0,
        0,
        0x48000009,
        0x4e800020,
        0x4e800020,
        0,
    ];
    for (i, w) in words.into_iter().enumerate() {
        support::put32(&mut bytes, text + i * 4, w);
    }
    fs::write(d.path().join("code/synthetic.rpx"), &bytes)?;
    let result = AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    assert_eq!(result.functions, 3); // 1000 -> indirect 1018 -> direct 1020.
    let f = p.list("functions", "00001000", 1, 0)?.items[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let site = p.list("indirect", &f, 10, 0)?.items.remove(0);
    assert_eq!(site["status"], "RESOLVED");
    assert_eq!(site["target"], 0x1018);
    assert!(site["evidence"]
        .as_str()
        .unwrap()
        .contains("reaching_definitions"));
    assert_eq!(
        p.list("indirect_targets", &f, 10, 0)?.items[0]["kind"],
        "STATIC"
    );
    assert_eq!(p.list("callees", &f, 10, 0)?.items[0]["target"], 0x1018);
    assert_eq!(
        p.list("dataflow_status", &f, 1, 0)?.items[0]["status"],
        "CONVERGED"
    );
    assert!(!p.list("values", &f, 100, 0)?.items.is_empty());
    assert!(!p.list("definitions", &f, 100, 0)?.items.is_empty());
    assert_eq!(
        p.db.query_row("SELECT count(*) FROM call_edges", [], |r| r
            .get::<_, u32>(0))?,
        2
    );
    assert_eq!(
        p.db.query_row(
            "SELECT provenance FROM function_evidence WHERE kind='INDIRECT_CALL_TARGET'",
            [],
            |r| r.get::<_, String>(0)
        )?,
        "INFERRED"
    );
    assert_eq!(fs::read(d.path().join("code/synthetic.rpx"))?, bytes);
    let saved = p.list("indirect", &f, 100, 0)?.items;
    p.db.execute("UPDATE modules SET dataflow_version=0", [])?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    assert_eq!(p.list("indirect", &f, 100, 0)?.items, saved);
    Ok(())
}

#[test]
fn switch_feedback_persists_all_cases_and_default_without_scanning() -> Result<()> {
    let (d, mut p) = setup()?;
    let mut bytes = support::cafe_fixture();
    let words = [
        0x28030002u32,
        0x4181002c,
        0x38802000,
        0x5465103a,
        0x7cc4282e,
        0x7cc903a6,
        0x4e800420,
        0,
        0x4e800020,
        0x4e800020,
        0x4e800020,
        0,
        0x4e800020,
    ];
    let m = woomy_binary::parse(&bytes)?;
    bytes[m.sections[4].offset as usize + 28] = 3;
    for (section, words) in [
        (1, words.as_slice()),
        (2, [0x1020u32, 0x1024, 0x1028].as_slice()),
    ] {
        let offset = bytes.len() as u32;
        support::put32(&mut bytes, 64 + section * 40 + 16, offset);
        support::put32(&mut bytes, 64 + section * 40 + 20, words.len() as u32 * 4);
        bytes.extend(words.iter().flat_map(|w| w.to_be_bytes()));
    }
    support::put32(&mut bytes, 64 + 2 * 40 + 8, 2); // Immutable synthetic table.
    support::put32(&mut bytes, 64 + 7 * 40 + 20, 0); // No data relocation in this fixture.
    fs::write(d.path().join("code/synthetic.rpx"), &bytes)?;
    let r = AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    assert_eq!(r.functions, 1);
    assert_eq!(r.instructions, 11);
    let tables = p.list("jump_tables", "", 10, 0)?;
    assert_eq!(tables.items[0]["status"], "VERIFIED");
    assert_eq!(tables.items[0]["default_target"], 0x1030);
    assert_eq!(
        p.db.query_row("SELECT count(*) FROM jump_table_entries", [], |r| r
            .get::<_, u32>(0))?,
        3
    );
    assert_eq!(
        p.db.query_row(
            "SELECT count(*) FROM cfg_edges WHERE kind='IndirectProven'",
            [],
            |r| r.get::<_, u32>(0)
        )?,
        3
    );
    assert_eq!(
        p.db.query_row(
            "SELECT count(*) FROM cfg_edges WHERE kind='Indirect'",
            [],
            |r| r.get::<_, u32>(0)
        )?,
        0
    );
    assert!(p.list("unresolved", "", 100, 0)?.items.is_empty());
    assert_eq!(fs::read(d.path().join("code/synthetic.rpx"))?, bytes);
    Ok(())
}

#[test]
fn ir_is_persisted_versioned_paginated_and_checks_database_provenance() -> Result<()> {
    let (_d, mut p) = setup()?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    let f = p.list("functions", "00001000", 1, 0)?.items[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let page = p.inspect_ir(&f, 1, 0)?;
    assert_eq!(page["function"]["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(page["pagination"]["has_more"], true);
    assert_eq!(page["verification"]["structural_valid"], true);
    assert_eq!(
        page["verification"]["executable_semantics_validated"],
        false
    );
    assert_eq!(
        p.verify_ir(&f)?["verification"]["status"],
        "STRUCTURALLY_VALID"
    );
    assert_eq!(p.status()?["ir_functions"], 2);
    assert_eq!(p.status()?["ir_structurally_valid"], 2);
    let before = p.inspect_ir(&f, 1000, 0)?;
    p.db.execute("UPDATE modules SET ir_version=0", [])?;
    p.invalidate_versions()?;
    assert!(p.inspect_ir(&f, 10, 0).is_err());
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    assert_eq!(p.inspect_ir(&f, 1000, 0)?, before);
    p.db.execute("UPDATE instructions SET raw=raw+1 WHERE address=4096", [])?;
    let report = p.verify_ir(&f)?;
    assert_eq!(report["verification"]["status"], "INVALID");
    assert!(report["verification"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == "DATABASE_PROVENANCE"));
    Ok(())
}

#[cfg(windows)]
#[test]
fn junction_cannot_escape_output_root() -> Result<()> {
    let (d, p) = setup()?;
    let link = d.path().join(".woomy/escape");
    let target = d.path().join("code");
    let result=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:WOOMY_TEST_LINK -Target $env:WOOMY_TEST_TARGET -ErrorAction Stop | Out-Null"]).env("WOOMY_TEST_LINK",&link).env("WOOMY_TEST_TARGET",&target).output()?;
    assert!(result.status.success(), "junction fixture creation failed");
    let rejected = p.output.path("escape/synthetic.rpx").is_err();
    fs::remove_dir(&link)?;
    assert!(rejected);
    assert!(target.join("synthetic.rpx").is_file());
    Ok(())
}

#[test]
fn native_artifacts_are_versioned_bounded_and_never_execute_game_code() -> Result<()> {
    let (d, mut p) = setup()?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    let rows = p.list("functions", "", 100, 0)?.items;
    let id = rows
        .iter()
        .find_map(|r| r["id"].as_str().filter(|s| s.ends_with("00001020")))
        .unwrap();
    let source_before = fs::read(d.path().join("code/synthetic.rpx"))?;
    let coverage = p.ir_coverage()?;
    assert_eq!(coverage["total"], 2);
    assert_eq!(coverage, p.ir_coverage()?);
    assert!(p.codegen("../../code/synthetic.rpx").is_err());
    let generated = p.codegen(id)?;
    assert_eq!(generated["status"], "GENERATED");
    let source = generated["source_file"].as_str().unwrap();
    let path = p.output.path(source)?;
    assert_eq!(generated, p.codegen(id)?);
    for field in [
        "native_abi_version",
        "runtime_abi_version",
        "guest_memory_version",
        "call_model_version",
        "function_summary_version",
    ] {
        let mut obsolete = generated.clone();
        obsolete[field] = serde_json::json!(0);
        p.db.execute(
            "UPDATE codegen_artifacts SET manifest=?1 WHERE function=?2",
            rusqlite::params![serde_json::to_string(&obsolete)?, id],
        )?;
        assert_eq!(p.codegen_status(id)?["manifest"]["status"], "ELIGIBLE");
    }
    for field in ["image", "loader", "relocation", "context"] {
        let mut obsolete = generated.clone();
        obsolete["image_context_versions"][field] = serde_json::json!(0);
        p.db.execute(
            "UPDATE codegen_artifacts SET manifest=?1 WHERE function=?2",
            rusqlite::params![serde_json::to_string(&obsolete)?, id],
        )?;
        assert_eq!(p.codegen_status(id)?["manifest"]["status"], "ELIGIBLE");
    }
    p.codegen(id)?;
    assert_eq!(p.codegen_status(id)?["semantically_validated"], false);
    fs::write(&path, b"malicious edited source")?;
    assert_eq!(
        p.codegen_status(id)?["manifest"]["status"],
        "STALE_ARTIFACT"
    );
    #[cfg(windows)]
    {
        let built = p.build_native(id)?;
        assert_eq!(built["status"], "COMPILED_NOT_SEMANTICALLY_VALIDATED");
        assert_eq!(built["native_executed"], false);
        assert_eq!(p.status()?["native"]["compiled_native"], 1);
        let exe = p.output.path(built["executable"].as_str().unwrap())?;
        fs::write(&exe, b"tampered")?;
        assert_eq!(p.codegen_status(id)?["build"]["status"], "STALE_ARTIFACT");
        assert_eq!(p.status()?["native"]["compiled_native"], 0);
    }
    fs::remove_file(&path)?;
    fs::hard_link(d.path().join("code/synthetic.rpx"), &path)?;
    assert!(p.codegen(id).is_err());
    fs::remove_file(&path)?;
    assert_eq!(
        source_before,
        fs::read(d.path().join("code/synthetic.rpx"))?
    );
    p.db.execute("UPDATE modules SET ir_version=0", [])?;
    p.invalidate_versions()?;
    assert!(p.codegen_status(id).is_err());
    assert_eq!(p.status()?["native"]["cpp_generated"], 0);
    Ok(())
}

#[test]
fn persisted_call_summaries_imports_and_dependency_versions_are_checked() -> Result<()> {
    let (d, mut p) = setup()?;
    let cancel = AtomicU8::new(0);
    let before = fs::read(d.path().join("code/synthetic.rpx"))?;
    AnalysisService::run(&mut p, &cancel, |_| {})?;
    let functions = p.list("functions", "", 100, 0)?.items;
    let caller = functions[0]["id"].as_str().unwrap();
    let callee = functions[1]["id"].as_str().unwrap();
    assert_eq!(p.function_summary(caller)?["fixed_point"], true);
    let calls = p.calls(caller, None, 100, 0)?;
    assert!(!calls["items"].as_array().unwrap().is_empty());
    assert_eq!(p.runtime_status()?["provider"], "NoRuntime");
    assert_eq!(p.native_status(caller)?["execution_ready"], false);
    assert!(p.calls(caller, None, 0, 0).is_err());
    let site = calls["items"][0]["site"].as_u64().unwrap() as u32;
    assert_eq!(
        p.calls(caller, Some(site), 100, 0)?["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    p.db.execute(
        "UPDATE call_contracts SET version=0 WHERE function=?1",
        [caller],
    )?;
    assert!(p.calls(caller, None, 100, 0).is_err());
    p.db.execute(
        "UPDATE call_contracts SET version=?1",
        [woomy_ir::runtime::CALL_MODEL_VERSION],
    )?;
    p.db.execute(
        "UPDATE ir_functions SET artifact=artifact || ' ' WHERE function=?1",
        [callee],
    )?;
    assert!(p
        .function_summary(caller)
        .unwrap_err()
        .to_string()
        .contains("DEPENDENCIES"));
    p.db.execute("UPDATE function_summaries SET version=0", [])?;
    p.invalidate_versions()?;
    assert!(p.function_summary(caller).is_err());
    assert_eq!(
        AnalysisService::run(&mut p, &cancel, |_| {})?.parsed_modules,
        1
    );
    assert_eq!(p.function_summary(caller)?["fixed_point"], true);
    assert_eq!(
        p.db.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))?,
        woomy_core::database::SCHEMA_VERSION as u32
    );
    assert_eq!(before, fs::read(d.path().join("code/synthetic.rpx"))?);
    Ok(())
}
