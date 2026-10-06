//! Persisted, versioned call evidence; runtime readiness is separate from compilation.
use crate::{Error, ProjectService, Result};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};
use woomy_ir::{
    calls::{Contract, Summary, Target},
    runtime::*,
    Function, Operation,
};
pub(crate) fn persist(tx: &Transaction<'_>, module: &str) -> Result<()> {
    let mut statement=tx.prepare("SELECT i.artifact FROM ir_functions i JOIN functions f ON f.id=i.function WHERE f.module=?1 ORDER BY f.address")?;
    let artifacts = statement
        .query_map([module], |r| r.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let functions: Vec<Function> = artifacts
        .iter()
        .map(|a| serde_json::from_str(a))
        .collect::<std::result::Result<_, _>>()?;
    let summaries = woomy_ir::calls::summarize(&functions);
    let dependency_hash = woomy_codegen::hash(&serde_json::to_vec(&artifacts)?);
    for function in &functions {
        let ir_hash = woomy_codegen::hash(&serde_json::to_vec(function)?);
        tx.execute(
            "INSERT INTO function_summaries VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                function.id,
                FUNCTION_SUMMARY_VERSION,
                CALL_MODEL_VERSION,
                ir_hash,
                dependency_hash,
                serde_json::to_string(&summaries[&function.id])?
            ],
        )?;
        for call in woomy_ir::calls::contracts(function, &functions, &summaries) {
            let kind = match call.target {
                Target::InternalFunction { .. } => "INTERNAL",
                Target::ImportedFunction { .. } => "IMPORTED",
                Target::IndirectFunction { .. } => "INDIRECT",
                Target::RuntimeFunction { .. } => "RUNTIME",
                Target::Unresolved => "UNRESOLVED",
            };
            tx.execute(
                "INSERT INTO call_contracts VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    function.id,
                    call.site,
                    CALL_MODEL_VERSION,
                    serde_json::to_value(call.effects.certainty)?
                        .as_str()
                        .unwrap_or("UNKNOWN"),
                    kind,
                    serde_json::to_string(&call)?
                ],
            )?;
            if let Target::ImportedFunction { library, symbol } = call.target {
                tx.execute(
                    "INSERT INTO runtime_imports VALUES(?1,?2,?3,?4,'UNKNOWN','UNRESOLVED_IMPORT')",
                    params![function.id, call.site, library, symbol],
                )?;
            }
        }
    }
    Ok(())
}
impl ProjectService {
    pub fn function_summary(&self, id: &str) -> Result<Value> {
        let ir = self.read_ir(id)?;
        let (version,call_version,hash,dependency_hash,artifact):(u32,u32,String,String,String)=self.db.query_row("SELECT version,call_model_version,ir_hash,dependency_hash,artifact FROM function_summaries WHERE function=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let mut statement=self.db.prepare("SELECT i.artifact FROM ir_functions i JOIN functions f ON f.id=i.function WHERE f.module=?1 ORDER BY f.address")?;
        let artifacts = statement
            .query_map([&ir.module], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if dependency_hash != woomy_codegen::hash(&serde_json::to_vec(&artifacts)?) {
            return Err(Error::Analysis(
                "STALE_FUNCTION_SUMMARY_DEPENDENCIES: analyze again".into(),
            ));
        }
        if version != FUNCTION_SUMMARY_VERSION
            || call_version != CALL_MODEL_VERSION
            || hash != woomy_codegen::hash(&serde_json::to_vec(&ir)?)
            || artifact.len() > 16 * 1024 * 1024
        {
            return Err(Error::Analysis(
                "STALE_FUNCTION_SUMMARY: analyze again".into(),
            ));
        }
        let summary: Summary = serde_json::from_str(&artifact)?;
        if summary.version != version || summary.function != id {
            return Err(Error::Analysis("INVALID_FUNCTION_SUMMARY".into()));
        }
        Ok(serde_json::to_value(summary)?)
    }
    pub fn calls(&self, id: &str, site: Option<u32>, limit: usize, offset: usize) -> Result<Value> {
        self.function_summary(id)?;
        if limit == 0 || limit > 1000 || offset > 10_000_000 {
            return Err(Error::Config("call query budget".into()));
        }
        let mut statement=self.db.prepare("SELECT version,artifact FROM call_contracts WHERE function=?1 AND (?2 IS NULL OR site=?2) ORDER BY site LIMIT ?3 OFFSET ?4")?;
        let rows = statement
            .query_map(params![id, site, limit + 1, offset], |r| {
                Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = rows.len() > limit;
        let mut items = Vec::new();
        for (version, artifact) in rows.into_iter().take(limit) {
            if version != CALL_MODEL_VERSION || artifact.len() > 1024 * 1024 {
                return Err(Error::Analysis("STALE_CALL_CONTRACT".into()));
            }
            let call: Contract = serde_json::from_str(&artifact)?;
            if call.version != version || call.caller != id {
                return Err(Error::Analysis("INVALID_CALL_CONTRACT".into()));
            }
            items.push(call);
        }
        Ok(json!({"items":items,"limit":limit,"offset":offset,"has_more":has_more}))
    }
    pub fn runtime_status(&self) -> Result<Value> {
        self.refresh_module_hashes()?;
        let native = self.native_counts()?;
        let rows=self.rows("SELECT c.certainty,c.target_kind,count(*) AS count FROM call_contracts c JOIN functions f ON f.id=c.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' GROUP BY c.certainty,c.target_kind ORDER BY c.certainty,c.target_kind",&[])?;
        Ok(
            json!({"call_model_version":CALL_MODEL_VERSION,"function_summary_version":FUNCTION_SUMMARY_VERSION,"guest_memory_version":GUEST_MEMORY_VERSION,"runtime_abi_version":RUNTIME_ABI_VERSION,"provider":"NoRuntime","test_provider":"TEST_RUNTIME_ONLY (synthetic validation library)","manmenmi":"M10_EXTERNAL_ADAPTER_IMPLEMENTED","model_contracts":2,"native_model_bridge":"IMPLEMENTED_VALIDATION_SUBSET","manmenmi_c_abi":[1,1,1,1],"provider_abi":manmenmi::MANMENMI_PROVIDER_ABI_VERSION,"reference_validated_handlers":0,"call_analysis":rows,"domain":"GAME_FUNCTION_REFERENCE_CONTEXT","execution_ready":0,"executed":0,"semantic_match":0,"mismatch":0,"inconclusive":0,"synthetic_domain":{"executed_functions":native["native_executed"],"matched_functions":native["synthetic_domain_functions_matched"],"cases":native["synthetic_domain_cases"]},"scope":"Reference process state remains unknown; explicit Cafe model uses the separately configured M10 adapter; compiled does not imply reference readiness"}),
        )
    }
    pub fn runtime_imports(&self, limit: usize, offset: usize) -> Result<Value> {
        self.refresh_module_hashes()?;
        if limit == 0 || limit > 1000 || offset > 10_000_000 {
            return Err(Error::Config("import query budget".into()));
        }
        let mut rows=self.rows("SELECT r.* FROM runtime_imports r JOIN functions f ON f.id=r.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' ORDER BY r.function,r.site LIMIT ?1 OFFSET ?2",&[&((limit+1) as i64),&(offset as i64)])?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        for row in &mut rows {
            let import = Import {
                library: row["library"].as_str().unwrap_or("").into(),
                symbol: row["symbol"].as_str().unwrap_or("").into(),
            };
            let contract = manmenmi::import_contract(&import);
            row["model_status"] = json!(if contract.is_some() {
                "MODEL_IMPLEMENTED"
            } else {
                "UNRESOLVED"
            });
            row["model_contract"] = serde_json::to_value(contract)?;
            row["reference_status"] = json!("NOT_REFERENCE_VALIDATED");
        }
        Ok(
            json!({"items":rows,"limit":limit,"offset":offset,"has_more":more,"scope":"Adapter enables two M10 services; other M10 capabilities are not enabled for game execution"}),
        )
    }
    pub fn native_status(&self, id: &str) -> Result<Value> {
        let ir = self.read_ir(id)?;
        let summary = self.function_summary(id)?;
        let operations: Vec<_> = ir
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .map(|i| &i.operation)
            .collect();
        let class = if operations.iter().any(|op| {
            matches!(
                op,
                Operation::Call {
                    target: woomy_ir::CallTarget::Imported { .. },
                    ..
                }
            )
        }) {
            "IMPORTS"
        } else if operations.iter().any(|op| {
            matches!(
                op,
                Operation::Call {
                    target: woomy_ir::CallTarget::Indirect { .. } | woomy_ir::CallTarget::Unknown,
                    ..
                } | Operation::UnknownInstruction { .. }
            )
        }) {
            "UNRESOLVED"
        } else if operations
            .iter()
            .any(|op| matches!(op, Operation::Call { .. }))
        {
            "INTERNAL_CALLS"
        } else if operations
            .iter()
            .any(|op| matches!(op, Operation::Load { .. } | Operation::Store { .. }))
        {
            "CONTROLLED_MEMORY_REQUIRED"
        } else {
            "REGISTER_STATE_ONLY"
        };
        let validation_runs = self.current_validation_runs(&ir)?;
        Ok(
            json!({"function":id,"classification":class,"native":self.codegen_status(id)?,"summary":summary,"execution_ready":false,"executed":!validation_runs.is_empty(),"validation_runs":validation_runs,"semantic_match":false,"real_game_context_match":false,"reason":"Initial register state and return target, guest memory permissions and dependency closure have not been validated for this game function"}),
        )
    }
}
