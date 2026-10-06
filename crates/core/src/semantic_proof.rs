//! The only semantic-to-machine bridge. Re-derivation, not confidence or cache,
//! authorizes lowering an unconditional indirect call to a proved static call.
use crate::{
    semantic_interprocedural::{self, Report},
    semantic_values::InitialWord,
    Error, ProjectService, Result,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{runtime::GuestMemory, CallTarget, Function, Operation};

pub fn promote(
    functions: &[Function],
    memory: &GuestMemory,
    initial: &[InitialWord],
    image_hash: &str,
) -> Result<(Vec<Function>, Vec<Value>)> {
    let report = semantic_interprocedural::analyze(functions, memory, initial, image_hash, false)?;
    lower(functions, &report)
}
pub(crate) fn lower(
    functions: &[Function],
    report: &Report,
) -> Result<(Vec<Function>, Vec<Value>)> {
    let mut lowered = functions.to_vec();
    let mut proofs = vec![];
    if !report.converged || report.entry_assumption != "UNKNOWN_RUNTIME_ENTRY" {
        return Ok((lowered, proofs));
    }
    let nodes: BTreeMap<_, _> = report.nodes.iter().map(|n| (n.id.clone(), n)).collect();
    for f in &mut lowered {
        if report.ir_hashes.get(&f.id) != Some(&crate::hash_bytes(&serde_json::to_vec(f)?)) {
            return Err(Error::Analysis("STALE_SEMANTIC_PROOF_INPUT".into()));
        }
        let Some(summary) = report.summaries.get(&f.id) else {
            continue;
        };
        for site in summary.indirects.iter().filter(|s| {
            s.classification == "MACHINE_PROVEN_SINGLE_TARGET"
                && s.failed_requirements.is_empty()
                && s.targets.len() == 1
        }) {
            let target = site.targets[0];
            let Some(callee) = functions
                .iter()
                .find(|c| c.module == f.module && c.entry == target)
            else {
                continue;
            };
            let mut chain = BTreeSet::new();
            let mut pending = site.value.definitions.iter().cloned().collect::<Vec<_>>();
            while let Some(id) = pending.pop() {
                if !chain.insert(id.clone()) {
                    continue;
                }
                if chain.len() > 4096 {
                    return Err(Error::Analysis(
                        "MACHINE_PROOF_CHAIN_BUDGET_EXHAUSTED".into(),
                    ));
                }
                let n = nodes
                    .get(&id)
                    .ok_or_else(|| Error::Analysis("MACHINE_PROOF_MISSING_NODE".into()))?;
                pending.extend(n.inputs.iter().cloned());
            }
            let Some(op)=f.blocks.iter_mut().flat_map(|b|&mut b.instructions).find(|i|i.source.address==site.pc && matches!(i.operation,Operation::Call {target:CallTarget::Indirect {source},..} if source==site.source)) else{continue;};
            if !matches!(
                woomy_ppc::decode(op.source.address, op.source.raw).control_flow,
                woomy_ppc::Flow::Indirect {
                    link: true,
                    conditional: false,
                    ..
                }
            ) {
                continue;
            }
            if let Operation::Call {
                target: destination,
                ..
            } = &mut op.operation
            {
                *destination = CallTarget::Static {
                    module: f.module.clone(),
                    address: target,
                };
            }
            proofs.push(json!({"version":semantic_interprocedural::PROOF_GATE_VERSION,"report_identity":report.identity,"image_hash":report.image_hash,"function":f.id,"module":f.module,"module_hash":f.module_hash,"input_ir_hash":report.ir_hashes[&f.id],"pc":site.pc,"source_register":site.source,"target":target,"callee":callee.id,"gate":site,"memory_effects":report.memory_effects,"call_effects":summary.call_effects,"chain":chain.iter().map(|id|nodes[id]).collect::<Vec<_>>(),"rule":"ALL_REACHING_DEFINITIONS_EXACT_EXECUTABLE_ENTRY; UNCONDITIONAL_CALL; TARGET_CAPTURE_AND_LINK_VALUE_PRESERVED","scope":"Known IR CFG paths; no source-level type or Cafe ABI claim"}));
        }
        if !woomy_ir::verify(f).structural_valid {
            return Err(Error::Analysis("MACHINE_PROOF_LOWERING_INVALID".into()));
        }
    }
    Ok((lowered, proofs))
}
impl ProjectService {
    pub(crate) fn persist_semantic_promotions(
        &self,
        functions: &[Function],
        report: &Report,
    ) -> Result<usize> {
        let (lowered, proofs) = lower(functions, report)?;
        if proofs.is_empty() {
            return Ok(0);
        }
        let module = &functions[0].module;
        if functions.iter().any(|f| &f.module != module) {
            return Err(Error::Analysis("MACHINE_PROOF_CROSS_MODULE_BATCH".into()));
        }
        let tx = self.db.unchecked_transaction()?;
        let changed: BTreeSet<_> = proofs
            .iter()
            .filter_map(|p| p["function"].as_str())
            .collect();
        for f in lowered.iter().filter(|f| changed.contains(f.id.as_str())) {
            let old: String = tx.query_row(
                "SELECT artifact FROM ir_functions WHERE function=?1",
                [&f.id],
                |r| r.get(0),
            )?;
            if crate::hash_bytes(old.as_bytes()) != report.ir_hashes[&f.id] {
                return Err(Error::Analysis("STALE_MACHINE_PROOF_DATABASE".into()));
            }
            tx.execute(
                "UPDATE ir_functions SET artifact=?2 WHERE function=?1",
                rusqlite::params![f.id, serde_json::to_string(f)?],
            )?;
            let verification = woomy_ir::verify(f);
            tx.execute("UPDATE ir_verifications SET status=?2,unknown_operations=?3,report=?4 WHERE function=?1",rusqlite::params![f.id,verification.status,verification.unknown_operations,serde_json::to_string(&verification)?])?;
            tx.execute("DELETE FROM codegen_artifacts WHERE function=?1", [&f.id])?;
            tx.execute(
                "UPDATE native_builds SET status='STALE' WHERE function=?1",
                [&f.id],
            )?;
        }
        for proof in &proofs {
            let function = proof["function"].as_str().expect("owned proof");
            let pc = proof["pc"].as_u64().expect("owned proof");
            let target = proof["target"].as_u64().expect("owned proof");
            let evidence = serde_json::to_string(proof)?;
            let affected=tx.execute("UPDATE indirect_sites SET status='RESOLVED',target=?3,evidence=?4 WHERE function=?1 AND address=?2 AND is_return=0",rusqlite::params![function,pc,target,evidence])?;
            if affected != 1 {
                return Err(Error::Analysis(
                    "MACHINE_PROOF_MISSING_HISTORICAL_SITE".into(),
                ));
            }
            tx.execute(
                "DELETE FROM indirect_targets WHERE function=?1 AND site=?2",
                rusqlite::params![function, pc],
            )?;
            tx.execute(
                "INSERT INTO indirect_targets VALUES(?1,?2,0,'STATIC',?3,?4,NULL,NULL,?5)",
                rusqlite::params![function, pc, module, target, evidence],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO call_edges VALUES(?1,?2,?3)",
                rusqlite::params![function, proof["callee"].as_str(), pc],
            )?;
            tx.execute(
                "UPDATE call_sites SET target=?3,evidence=?4 WHERE function=?1 AND address=?2",
                rusqlite::params![function, pc, target, evidence],
            )?;
            tx.execute(
                "INSERT INTO cross_references VALUES(?1,?2,?3,'IndirectTarget','MACHINE_PROVEN')",
                rusqlite::params![module, pc, target],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO analysis_products VALUES('MACHINE_PROOF',?1,?2,?3,?4,?5)",
                rusqlite::params![
                    format!("{function}:{pc:08X}"),
                    module,
                    report.identity,
                    serde_json::to_string(&report.versions)?,
                    evidence
                ],
            )?;
        }
        tx.execute("DELETE FROM function_summaries WHERE function IN (SELECT id FROM functions WHERE module=?1)",[module])?;
        crate::calls::persist(&tx, module)?;
        tx.execute("DELETE FROM analysis_products WHERE module=?1 AND kind NOT IN ('MACHINE_PROOF','IMAGE')",[module])?;
        tx.commit()?;
        Ok(proofs.len())
    }
}
