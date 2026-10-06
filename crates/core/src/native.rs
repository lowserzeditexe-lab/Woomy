//! Project-owned native artifacts. Game functions may be compiled here but are
//! never executed: synthetic differential execution is a separate test harness.
use crate::{Error, ProjectService, Result};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use woomy_codegen::{compiler::NativeCompiler, Manifest};
use woomy_ir::{Operation, Terminator};

impl ProjectService {
    fn native_directory(&self) -> Result<std::path::PathBuf> {
        let dir = self.output.path("generated/cpp")?;
        std::fs::create_dir_all(&dir)?;
        self.output.path("generated/cpp")
    }
    pub fn codegen(&self, id: &str) -> Result<Value> {
        let verified = self.verify_ir(id)?;
        if verified["verification"]["structural_valid"] != true {
            return Err(Error::Analysis(
                "CODEGEN_UNSUPPORTED: database IR verification failed".into(),
            ));
        }
        let ir = self.read_ir(id)?;
        let mut generated = woomy_codegen::generate(&ir);
        self.native_directory()?;
        // Opaque content key, never a user-supplied filename or compiler option.
        let stem = format!("generated/cpp/{}", generated.manifest.cache_key);
        if let Some(source) = generated.source {
            let path = format!("{stem}.cpp");
            self.output.write_atomic(&path, source.as_bytes())?;
            generated.manifest.source_file = Some(path);
        }
        let text = serde_json::to_string_pretty(&generated.manifest)?;
        self.output
            .write_atomic(&format!("{stem}.json"), text.as_bytes())?;
        let m = &generated.manifest;
        self.db.execute("INSERT INTO codegen_artifacts VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(function) DO UPDATE SET cache_key=excluded.cache_key,backend_version=excluded.backend_version,native_abi_version=excluded.native_abi_version,status=excluded.status,manifest=excluded.manifest",params![id,m.cache_key,m.backend_version,m.native_abi_version,m.status,text])?;
        Ok(serde_json::to_value(m)?)
    }
    fn current_manifest(&self, ir: &woomy_ir::Function) -> Result<Manifest> {
        let expected = woomy_codegen::eligibility(ir);
        let text: Option<String> = self
            .db
            .query_row(
                "SELECT manifest FROM codegen_artifacts WHERE function=?1 AND cache_key=?2",
                params![ir.id, expected.cache_key],
                |r| r.get(0),
            )
            .optional()?;
        let Some(text) = text else {
            return Ok(expected);
        };
        let mut manifest: Manifest = serde_json::from_str(&text)?;
        if manifest.cache_key != expected.cache_key
            || manifest.ir_hash != expected.ir_hash
            || manifest.backend_version != expected.backend_version
            || manifest.native_abi_version != expected.native_abi_version
            || manifest.call_model_version != expected.call_model_version
            || manifest.function_summary_version != expected.function_summary_version
            || manifest.guest_memory_version != expected.guest_memory_version
            || manifest.runtime_abi_version != expected.runtime_abi_version
            || manifest.image_context_versions != expected.image_context_versions
            || manifest.function != ir.id
            || manifest.module_hash != ir.module_hash
        {
            return Ok(expected);
        }
        if let Some(path) = &manifest.source_file {
            let path = self.output.path(path)?;
            if !path.is_file()
                || std::fs::metadata(&path)?.len() > 32 * 1024 * 1024
                || Some(crate::hash_bytes(&std::fs::read(path)?)) != manifest.source_hash
            {
                manifest.status = "STALE_ARTIFACT".into();
            }
        }
        Ok(manifest)
    }
    fn current_build(&self, manifest: &Manifest) -> Result<Option<Value>> {
        let row:Option<(String,Option<String>)>=self.db.query_row("SELECT record,executable FROM native_builds WHERE function=?1 AND cache_key=?2 ORDER BY id DESC LIMIT 1",params![manifest.function,manifest.cache_key],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((record, path)) = row else {
            return Ok(None);
        };
        let mut record: Value = serde_json::from_str(&record)?;
        if let Some(relative) = path {
            let path = self.output.path(&relative)?;
            if manifest.status != "GENERATED"
                || record["source_hash"] != json!(manifest.source_hash)
                || !path.is_file()
                || std::fs::metadata(&path)?.len() > 64 * 1024 * 1024
                || record["output_hash"] != crate::hash_bytes(&std::fs::read(path)?)
            {
                record["status"] = json!("STALE_ARTIFACT");
            }
            record["executable"] = json!(relative);
        }
        Ok(Some(record))
    }
    pub fn codegen_status(&self, id: &str) -> Result<Value> {
        let ir = self.read_ir(id)?;
        let manifest = self.current_manifest(&ir)?;
        let build = self.current_build(&manifest)?;
        let validation_runs = self.current_validation_runs(&ir)?;
        Ok(
            json!({"manifest":manifest,"build":build,"native_executed":!validation_runs.is_empty(),"validation_runs":validation_runs,"real_game_context_match":false,"semantically_validated":false,"scope":"Compilation alone does not establish game function equivalence"}),
        )
    }
    pub fn build_native(&self, id: &str) -> Result<Value> {
        let generated = self.codegen(id)?;
        if generated["status"] != "GENERATED" {
            return Ok(json!({"status":"CODEGEN_UNSUPPORTED","manifest":generated}));
        }
        let manifest: Manifest = serde_json::from_value(generated)?;
        let compiler = match NativeCompiler::detect() {
            Ok(c) => c,
            Err(reason) => {
                let record = json!({"status":"TOOLCHAIN_NOT_FOUND","reason":reason,"source_hash":manifest.source_hash});
                self.db.execute("INSERT INTO native_builds(function,cache_key,status,record) VALUES(?1,?2,'TOOLCHAIN_NOT_FOUND',?3)",params![id,manifest.cache_key,serde_json::to_string(&record)?])?;
                return Ok(record);
            }
        };
        // Regenerate from verified IR, never compile a source file edited on disk.
        let ir = self.read_ir(id)?;
        let source = woomy_codegen::generate(&ir)
            .source
            .ok_or_else(|| Error::Analysis("CODEGEN_UNSUPPORTED".into()))?;
        if Some(crate::hash_bytes(source.as_bytes())) != manifest.source_hash {
            return Err(Error::Analysis("IR changed during native build".into()));
        }
        let build = compiler.build(&source, &self.native_directory()?)?;
        let executable = if let Some(bytes) = build.executable {
            let path = format!("generated/cpp/{}.exe", manifest.cache_key);
            self.output.write_atomic(&path, &bytes)?;
            Some(path)
        } else {
            None
        };
        let record = serde_json::to_string(&build.record)?;
        self.output.write_atomic(
            &format!("generated/cpp/{}.build.json", manifest.cache_key),
            record.as_bytes(),
        )?;
        self.db.execute("INSERT INTO native_builds(function,cache_key,status,executable,record) VALUES(?1,?2,?3,?4,?5)",params![id,manifest.cache_key,build.record.status,executable,record])?;
        Ok(
            json!({"function":id,"status":build.record.status,"build":build.record,"executable":executable,"native_executed":false,"semantically_validated":false}),
        )
    }
    /// Counts are recomputed from current IR/cache keys and checked files. Old
    /// binaries remain inspectable files but cannot count as current artifacts.
    pub fn native_counts(&self) -> Result<Value> {
        let mut eligible = 0;
        let mut generated = 0;
        let mut compiled = 0;
        let mut unsupported = 0;
        let mut lifted = 0;
        let mut partial = 0;
        let mut invalid = 0;
        let mut executed = 0;
        let mut synthetic_matched = 0;
        let mut synthetic_cases = 0u64;
        let mut synthetic_mismatch = 0;
        let mut stmt=self.db.prepare("SELECT i.artifact FROM ir_functions i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' ORDER BY i.function")?;
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            let text = row?;
            if text.len() > 16 * 1024 * 1024 {
                return Err(Error::Analysis("IR artifact exceeds budget".into()));
            }
            let ir: woomy_ir::Function = serde_json::from_str(&text)?;
            let runs = self.current_validation_runs(&ir)?;
            executed += usize::from(!runs.is_empty());
            synthetic_matched += usize::from(
                runs.iter()
                    .any(|r| r["status"] == "DIFFERENTIAL_MATCH_SYNTHETIC_DOMAIN"),
            );
            synthetic_mismatch += usize::from(runs.iter().any(|r| r["status"] == "MISMATCH"));
            synthetic_cases += runs
                .iter()
                .filter(|r| r["domain"] == "GAME_FUNCTION_SYNTHETIC_DOMAIN")
                .filter_map(|r| r["cases"].as_u64())
                .sum::<u64>();
            match ir.status.as_str() {
                "LIFTED" => lifted += 1,
                "PARTIALLY_LIFTED" => partial += 1,
                _ => invalid += 1,
            };
            let eligibility = woomy_codegen::eligibility(&ir);
            if eligibility.status == "ELIGIBLE" {
                eligible += 1;
            } else {
                unsupported += 1;
            }
            let m = self.current_manifest(&ir)?;
            if m.status == "GENERATED" {
                generated += 1;
            }
            if self
                .current_build(&m)?
                .is_some_and(|b| b["status"] == "COMPILED_NOT_SEMANTICALLY_VALIDATED")
            {
                compiled += 1;
            }
        }
        Ok(
            json!({"fully_lifted":lifted,"partially_lifted":partial,"invalid_ir":invalid,"codegen_eligible":eligible,"cpp_generated":generated,"compiled_native":compiled,"native_validated":0,"native_mismatch":synthetic_mismatch,"native_executed":executed,"synthetic_domain_functions_matched":synthetic_matched,"synthetic_domain_cases":synthetic_cases,"real_game_context_match":0,"real_context_execution_ready":0,"validation_scope":"Executed counts include explicit synthetic domains; native_validated is reserved for justified real contexts","unsupported":unsupported,"backend_version":woomy_codegen::CPP_BACKEND_VERSION,"native_abi_version":woomy_codegen::NATIVE_ABI_VERSION}),
        )
    }
    pub fn ir_coverage(&self) -> Result<Value> {
        self.refresh_module_hashes()?;
        let categories = [
            "conditional_branch",
            "CR_semantics",
            "XER_semantics",
            "relocation_lowering",
            "unsupported_integer",
            "unsupported_memory",
            "FPU",
            "paired_single",
            "unknown_instruction",
            "unknown_call_effect",
            "indirect_control_flow",
            "opaque_memory",
            "other",
        ];
        let mut histogram: BTreeMap<&str, usize> = categories.into_iter().map(|s| (s, 0)).collect();
        let mut opcode_histogram = BTreeMap::<String, usize>::new();
        let mut functions = Vec::new();
        let mut counts = BTreeMap::<String, usize>::new();
        let mut stmt=self.db.prepare("SELECT i.artifact FROM ir_functions i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' ORDER BY i.function")?;
        for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
            let text = row?;
            if text.len() > 16 * 1024 * 1024 {
                return Err(Error::Analysis("IR artifact exceeds budget".into()));
            }
            let ir: woomy_ir::Function = serde_json::from_str(&text)?;
            let verification = woomy_ir::verify(&ir);
            let status = if verification.structural_valid {
                ir.status.as_str()
            } else {
                "INVALID"
            };
            *counts.entry(status.to_owned()).or_default() += 1;
            let mut blockers = BTreeMap::new();
            let mut sites = BTreeSet::new();
            for b in &ir.blocks {
                for i in &b.instructions {
                    let category = match &i.operation {
                        Operation::RelocationImmediate { relocation, .. }
                            if relocation.local_address.is_none() =>
                        {
                            Some("relocation_lowering")
                        }
                        Operation::UnknownInstruction {
                            opcode, effects, ..
                        } => {
                            *opcode_histogram.entry(opcode.clone()).or_default() += 1;
                            let source = b.sources.iter().find(|s| s.address == i.source.address);
                            if source.is_some_and(|s| {
                                s.status.starts_with("RELOCATION") || s.status == "IMPORT_REFERENCE"
                            }) {
                                Some("relocation_lowering")
                            } else if source
                                .is_some_and(|s| s.status == "UNKNOWN" || s.status == "INVALID")
                            {
                                Some("unknown_instruction")
                            } else if opcode.starts_with("ps_") {
                                Some("paired_single")
                            } else if opcode.starts_with('f')
                                || opcode.starts_with("lf")
                                || opcode.starts_with("stf")
                            {
                                Some("FPU")
                            } else if ["bc", "bclr", "bcctr"].contains(&opcode.as_str()) {
                                Some("conditional_branch")
                            } else if opcode.starts_with("cmp")
                                || opcode.starts_with("cr")
                                || opcode == "mfcr"
                                || opcode == "mtcrf"
                            {
                                Some("CR_semantics")
                            } else if effects.defs.contains(&woomy_ppc::Register::Xer) {
                                Some("XER_semantics")
                            } else if opcode.starts_with('l') || opcode.starts_with("st") {
                                Some("unsupported_memory")
                            } else if effects
                                .defs
                                .iter()
                                .any(|r| matches!(r, woomy_ppc::Register::Cr(_)))
                            {
                                Some("CR_semantics")
                            } else {
                                Some("unsupported_integer")
                            }
                        }
                        Operation::Call { .. } => Some("unknown_call_effect"),
                        Operation::OpaqueMemoryEffect { .. } => Some("opaque_memory"),
                        Operation::UnknownValue { .. } | Operation::UnknownSideEffect { .. } => {
                            Some("other")
                        }
                        _ => None,
                    };
                    if let Some(category) = category {
                        if sites.insert((b.id, i.source.address)) {
                            *histogram.get_mut(category).unwrap() += 1;
                            *blockers.entry(category).or_insert(0usize) += 1;
                        }
                    }
                }
                if matches!(
                    b.terminator,
                    Terminator::UnresolvedIndirectBranch { .. }
                        | Terminator::ConditionalLrBranch { .. }
                        | Terminator::OpaqueTerminator { .. }
                ) && sites.insert((b.id, b.terminator_source.address))
                {
                    let category = if matches!(
                        b.terminator,
                        Terminator::UnresolvedIndirectBranch { .. }
                            | Terminator::ConditionalLrBranch { .. }
                    ) {
                        "indirect_control_flow"
                    } else {
                        "other"
                    };
                    *histogram.get_mut(category).unwrap() += 1;
                    *blockers.entry(category).or_insert(0) += 1;
                }
            }
            functions.push(json!({"function":ir.id,"status":status,"blockers":blockers}));
        }
        Ok(
            json!({"total":functions.len(),"statuses":counts,"histogram_unit":"one primary blocker per function/block/PPC source site; shared code may occur in multiple functions","histogram":histogram,"unknown_operation_opcodes":opcode_histogram,"functions":functions}),
        )
    }
}
