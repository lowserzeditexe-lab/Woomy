//! Indirect sites are resolved from converged register facts, never by scanning code.
use crate::module_model::ModuleModel;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use woomy_ppc::dataflow::{Atom, Dataflow, Target, Value};
use woomy_ppc::{Cfg, DecodedInstruction, Flow, Register, DATAFLOW_VERSION};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum ControlTarget {
    Static { module: String, address: u32 },
    Imported { library: String, symbol: String },
}
#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub address: u32,
    pub source: Register,
    pub is_return: bool,
    pub status: &'static str,
    pub targets: Vec<ControlTarget>,
    pub evidence: String,
}
pub fn resolve(
    cfg: &Cfg,
    code: &BTreeMap<u32, DecodedInstruction>,
    facts: &Dataflow,
    model: &ModuleModel<'_>,
) -> Vec<Resolution> {
    let mut result = Vec::new();
    for block in &cfg.blocks {
        for pc in &block.instructions {
            let Some(d) = code.get(pc) else {
                continue;
            };
            if !matches!(d.control_flow, Flow::Indirect { .. } | Flow::Return { .. }) {
                continue;
            }
            let source = if (d.raw >> 1) & 1023 == 528 {
                Register::Ctr
            } else {
                Register::Lr
            };
            let fact = facts.used(*pc, source);
            let value = fact.map(|f| &f.value).unwrap_or(&Value::Unknown);
            let mut targets = Vec::new();
            let mut rejected = 0;
            if facts.converged {
                if let Some(atoms) = value.atoms() {
                    for atom in atoms {
                        if let Atom::Target(Target::ImportedSymbol {
                            library,
                            symbol,
                            addend: 0,
                            is_code: true,
                        }) = atom
                        {
                            targets.push(ControlTarget::Imported { library, symbol });
                        } else if let Some(address) = model.executable_target(&atom) {
                            targets.push(ControlTarget::Static {
                                module: model.id.into(),
                                address,
                            });
                        } else {
                            rejected += 1;
                        }
                    }
                }
            }
            targets.sort();
            targets.dedup();
            let status = if targets.is_empty() {
                "UNRESOLVED"
            } else if rejected != 0 {
                "PARTIAL"
            } else if targets.len() > 1 {
                "AMBIGUOUS"
            } else {
                "RESOLVED"
            };
            result.push(Resolution {address:*pc,source,is_return:matches!(d.control_flow,Flow::Return{..}),status,targets,evidence:json!({"rule":"CONVERGED_REGISTER_VALUE","source_register":source,"value":value,"reaching_definitions":fact.map(|f|&f.definitions),"analysis_version":DATAFLOW_VERSION,"converged":facts.converged,"rejected_alternatives":rejected,"scope":"Known CFG paths; function boundaries remain inferred; imported runtime addresses unknown"}).to_string()});
        }
    }
    result.sort_by_key(|r| r.address);
    if let Some(hash) = model.loader_identity() {
        for resolution in &mut result {
            let mut evidence: serde_json::Value =
                serde_json::from_str(&resolution.evidence).expect("owned evidence JSON");
            evidence["loader_image_hash"] = json!(hash);
            evidence["loader_versions"] = json!(woomy_ir::runtime::ImageContextVersions::CURRENT);
            evidence["loader_read_rule"]=json!("Immutable resolved regions only; mutable globals unknown; memory alias taint invalidates loads");
            resolution.evidence = evidence.to_string();
        }
    }
    result
}
