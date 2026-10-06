//! Interpretation above machine IR. Never changes lifting, execution or call targets.
use crate::{Error, ProjectService, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{Binary, CallTarget, Function, Operation, ValueId};
use woomy_ppc::Register;
pub const SEMANTIC_VERSION: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceClass {
    Documented,
    PublicImplementationObservation,
    EmpiricallyObservedInLocalDump,
    ReferenceCapture,
    WoomyInferred,
    ManmenmiModel,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateStatus {
    Hypothesis,
    Conflicting,
    Rejected,
    Superseded,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticType {
    Unknown,
    Integer,
    SignedInteger,
    UnsignedInteger,
    Float,
    Double,
    GuestPointer {
        pointee: Option<String>,
    },
    FunctionPointer {
        signature: Option<String>,
    },
    Array {
        element: Option<String>,
        count: Option<u32>,
    },
    StructPointer {
        aggregate: String,
    },
    ClassPointer {
        class: String,
    },
    Enum,
    Opaque,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeEvidence {
    pub id: u32,
    pub block: u32,
    pub pc: u32,
    pub operation: ValueId,
    pub class: EvidenceClass,
    pub rule: String,
    pub detail: String,
    pub predecessor_facts: Vec<usize>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub status: CandidateStatus,
    pub confidence_basis_points: Option<u16>,
    pub evidence: Vec<u32>,
    pub supersedes: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeCandidate {
    pub candidate: Candidate,
    pub subject: String,
    pub alternatives: Vec<SemanticType>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionSignatureCandidate {
    pub candidate: Candidate,
    pub entry_register_inputs: Vec<Register>,
    pub return_type: SemanticType,
    pub abi: String,
    pub scope: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldCandidate {
    pub candidate: Candidate,
    pub offset: i64,
    pub width_bytes: u8,
    pub ty: SemanticType,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateCandidate {
    pub candidate: Candidate,
    pub base: String,
    pub fields: Vec<FieldCandidate>,
    pub extent: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectCandidate {
    pub candidate: Candidate,
    pub aggregate: Option<String>,
    pub storage: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VtableCandidate {
    pub candidate: Candidate,
    pub guest_address: Option<u32>,
    pub target_evidence: Vec<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassCandidate {
    pub candidate: Candidate,
    pub aggregates: Vec<String>,
    pub vtables: Vec<String>,
    pub methods: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PlatformCategory {
    GameLogic,
    Graphics,
    Audio,
    Input,
    Filesystem,
    Save,
    Network,
    Threading,
    Memory,
    System,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticKind {
    LoadField {
        base: String,
        offset: i64,
        width_bytes: u8,
    },
    StoreField {
        base: String,
        offset: i64,
        width_bytes: u8,
    },
    DirectCall {
        target: CallTarget,
    },
    IndirectCall {
        target: CallTarget,
    },
    PlatformCall {
        target: CallTarget,
        category: PlatformCategory,
    },
    // Reserved interpretations, not emitted by the v1 pass.
    ArrayAccess,
    VirtualCall,
    ObjectCast,
    AddressOf,
    ConstructorCandidate,
    DestructorCandidate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticOperation {
    pub candidate: Candidate,
    pub kind: SemanticKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticReport {
    pub version: u32,
    pub ir_version: u32,
    pub function: String,
    pub module: String,
    pub module_hash: String,
    pub ir_hash: String,
    pub domain: String,
    pub evidence: Vec<TypeEvidence>,
    pub types: Vec<TypeCandidate>,
    pub signatures: Vec<FunctionSignatureCandidate>,
    pub aggregates: Vec<AggregateCandidate>,
    pub objects: Vec<ObjectCandidate>,
    pub vtables: Vec<VtableCandidate>,
    pub classes: Vec<ClassCandidate>,
    pub operations: Vec<SemanticOperation>,
    pub inter_block_facts: Vec<crate::semantic_flow::FlowFact>,
    pub typed_arguments: Vec<TypeCandidate>,
    pub return_observations: Vec<Value>,
    pub inferred_type_kinds: Vec<String>,
    pub representable_type_kinds: Vec<String>,
    pub metrics: Value,
    pub limitations: Vec<String>,
}
fn candidate(id: String, evidence: Vec<u32>) -> Candidate {
    Candidate {
        id,
        status: CandidateStatus::Hypothesis,
        confidence_basis_points: None,
        evidence,
        supersedes: vec![],
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Expr {
    Conflict(Vec<Expr>),
    Constant(u32),
    Base(String, i64),
    Unknown,
}
fn category(library: &str, symbol: &str) -> PlatformCategory {
    match (library, symbol) {
        ("coreinit", "OSGetCurrentThread") => PlatformCategory::Threading,
        ("coreinit", "OSIsDebuggerInitialized") => PlatformCategory::System,
        ("coreinit", "memcpy" | "memset" | "memmove" | "OSBlockMove" | "OSBlockSet") => {
            PlatformCategory::Memory
        }
        ("gx2", _) => PlatformCategory::Graphics,
        ("snd_core" | "sndcore2", _) => PlatformCategory::Audio,
        ("vpad", _) => PlatformCategory::Input,
        ("nsysnet" | "nlibcurl", _) => PlatformCategory::Network,
        ("nn_save", _) => PlatformCategory::Save,
        _ => PlatformCategory::Unknown,
    }
}
pub fn analyze(f: &Function) -> Result<SemanticReport> {
    if !woomy_ir::verify(f).structural_valid {
        return Err(Error::Analysis("SEMANTIC_INVALID_MACHINE_IR".into()));
    }
    let total = f.blocks.iter().map(|b| b.instructions.len()).sum::<usize>();
    if total > 65536 {
        return Err(Error::Analysis("SEMANTIC_OPERATION_LIMIT".into()));
    }
    let flow = crate::semantic_flow::analyze(f)?;
    let mut r=SemanticReport { version:SEMANTIC_VERSION,ir_version:woomy_ir::IR_VERSION,function:f.id.clone(),module:f.module.clone(),module_hash:f.module_hash.clone(),ir_hash:crate::hash_bytes(&serde_json::to_vec(f)?),domain:"STATIC_MACHINE_IR_HYPOTHESES".into(),evidence:vec![],types:vec![],signatures:vec![],aggregates:vec![],objects:vec![],vtables:vec![],classes:vec![],operations:vec![],inter_block_facts:flow.facts.clone(),typed_arguments:vec![],return_observations:vec![],inferred_type_kinds:vec![],representable_type_kinds:["Unknown","Integer","SignedInteger","UnsignedInteger","Float","Double","GuestPointer","FunctionPointer","Array","StructPointer","ClassPointer","Enum","Opaque"].iter().map(|s|s.to_string()).collect(),metrics:Value::Null,limitations:vec!["Bounded CFG fixed point, unknown dominates joins; calls kill facts; no memory value caching or heap identity".into(),"Storage width does not establish signedness, source field names or object identity".into(),"Entry-register observations are not a recovered Cafe parameter ABI; return type UNKNOWN".into(),"No virtual calls/classes resolved; no executable semantics or indirect targets changed".into()] };
    let mut fields = BTreeMap::<String, BTreeMap<(i64, u8), Vec<u32>>>::new();
    let mut interpretations = BTreeMap::<String, Vec<(SemanticType, u32)>>::new();
    let mut inputs = BTreeMap::<Register, u32>::new();
    let mut memory_ops = 0usize;
    let mut calls = 0usize;
    let mut platform = 0usize;
    let mut categorized = 0usize;
    for block in &f.blocks {
        let mut values = BTreeMap::<ValueId, Expr>::new();
        let mut regs = flow.incoming.get(&block.id).cloned().unwrap_or_default();
        let mut barrier = true;
        for op in &block.instructions {
            let get = |id: &ValueId| values.get(id).cloned().unwrap_or(Expr::Unknown);
            let eid = r.evidence.len() as u32;
            let mut note: Option<(&str, String)> = None;
            let expr = match &op.operation {
                Operation::Constant { bits } => Expr::Constant(*bits as u32),
                Operation::ReadRegister { register } => {
                    if matches!(regs.get(register),Some(Expr::Base(base,0)) if base == &format!("incoming.{register:?}"))
                    {
                        inputs.entry(*register).or_insert(eid);
                        note=Some(("CFG_INCOMING_REGISTER_READ","Incoming register value observed through conservative CFG flow; parameter role remains a hypothesis".into()));
                    }
                    regs.get(register).cloned().unwrap_or_else(|| {
                        if barrier {
                            Expr::Unknown
                        } else {
                            Expr::Base(format!("block_{:08X}.{register:?}", block.id), 0)
                        }
                    })
                }
                Operation::WriteRegister { register, value } => {
                    if *register == Register::Gpr(3) {
                        r.return_observations.push(json!({"block":block.id,"pc":op.source.address,"operation":op.id,"value":get(value),"rule":"OBSERVED_R3_WRITE; not a proven return value or source type","status":"OBSERVATION"}));
                    }
                    regs.insert(*register, get(value));
                    Expr::Unknown
                }
                Operation::Address32 { value } => get(value),
                Operation::Binary {
                    operator: Binary::AddWrapping,
                    lhs,
                    rhs,
                } => match (get(lhs), get(rhs)) {
                    (Expr::Base(b, o), Expr::Constant(c))
                    | (Expr::Constant(c), Expr::Base(b, o)) => o
                        .checked_add(c as i32 as i64)
                        .filter(|n| i32::try_from(*n).is_ok())
                        .map(|n| Expr::Base(b, n))
                        .unwrap_or(Expr::Unknown),
                    (Expr::Constant(a), Expr::Constant(b)) => Expr::Constant(a.wrapping_add(b)),
                    _ => Expr::Unknown,
                },
                Operation::Load { address, width, .. }
                | Operation::Store { address, width, .. } => {
                    memory_ops += 1;
                    if let Expr::Base(base, offset) = get(address) {
                        note=Some(("BASE_OFFSET_ACCESS",format!("Observed {width}-byte memory access from {base} at {offset:+}; source aggregate identity unproven")));
                        interpretations
                            .entry(base.clone())
                            .or_default()
                            .push((SemanticType::GuestPointer { pointee: None }, eid));
                        // Stack accesses remain stack evidence, not structure candidates.
                        if !base.ends_with("Gpr(1)") {
                            fields
                                .entry(base.clone())
                                .or_default()
                                .entry((offset, *width))
                                .or_default()
                                .push(eid);
                            let kind = if matches!(op.operation, Operation::Load { .. }) {
                                SemanticKind::LoadField {
                                    base,
                                    offset,
                                    width_bytes: *width,
                                }
                            } else {
                                SemanticKind::StoreField {
                                    base,
                                    offset,
                                    width_bytes: *width,
                                }
                            };
                            r.operations.push(SemanticOperation {
                                candidate: candidate(
                                    format!("op.{:08X}.{}", op.id.address, op.id.slot),
                                    vec![eid],
                                ),
                                kind,
                            });
                        }
                    }
                    Expr::Unknown
                }
                Operation::Compare32 {
                    lhs, rhs, signed, ..
                } => {
                    for value in [get(lhs), get(rhs)] {
                        if let Expr::Base(base, 0) = value {
                            interpretations.entry(base).or_default().push((
                                if *signed {
                                    SemanticType::SignedInteger
                                } else {
                                    SemanticType::UnsignedInteger
                                },
                                eid,
                            ));
                            note=Some(("COMPARISON_INTERPRETATION","Signedness belongs to this comparison; it does not prove the source type".into()));
                        }
                    }
                    Expr::Unknown
                }
                Operation::BranchCondition {
                    decrement_ctr: true,
                    ..
                } => {
                    regs.insert(Register::Ctr, Expr::Unknown);
                    Expr::Unknown
                }
                Operation::Call { target, .. } => {
                    calls += 1;
                    note=Some(("MACHINE_CALL_IDENTITY","Target from typed IR; category is an inference from explicit import identity, not behavior or ABI proof".into()));
                    let kind = match target {
                        CallTarget::Imported { library, symbol } => {
                            platform += 1;
                            let c = category(library, symbol);
                            if c != PlatformCategory::Unknown {
                                categorized += 1;
                            }
                            SemanticKind::PlatformCall {
                                target: target.clone(),
                                category: c,
                            }
                        }
                        CallTarget::Static { .. } => SemanticKind::DirectCall {
                            target: target.clone(),
                        },
                        _ => SemanticKind::IndirectCall {
                            target: target.clone(),
                        },
                    };
                    r.operations.push(SemanticOperation {
                        candidate: candidate(
                            format!("op.{:08X}.{}", op.id.address, op.id.slot),
                            vec![eid],
                        ),
                        kind,
                    });
                    regs.clear();
                    values.clear();
                    barrier = true;
                    Expr::Unknown
                }
                Operation::UnknownInstruction { .. }
                | Operation::UnknownSideEffect { .. }
                | Operation::OpaqueMemoryEffect { .. } => {
                    regs.clear();
                    values.clear();
                    barrier = true;
                    Expr::Unknown
                }
                _ => Expr::Unknown,
            };
            if let Some((rule, detail)) = note {
                r.evidence.push(TypeEvidence {
                    id: eid,
                    block: block.id,
                    pc: op.source.address,
                    operation: op.id,
                    class: EvidenceClass::WoomyInferred,
                    rule: rule.into(),
                    detail,
                    predecessor_facts: r
                        .inter_block_facts
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| f.block == block.id)
                        .map(|(index, _)| index)
                        .collect(),
                });
            }
            if let Some(result) = &op.result {
                values.insert(result.id, expr);
            }
        }
    }
    for block in &f.blocks {
        if matches!(block.terminator, woomy_ir::Terminator::Return { .. }) {
            let observations: Vec<_> = r
                .return_observations
                .iter()
                .filter(|v| v["block"].as_u64() == Some(block.id as u64))
                .cloned()
                .collect();
            if let Some(last) = observations.last() {
                let last_pc = last["pc"].as_u64().unwrap() as u32;
                if !block.instructions.iter().any(|op| {
                    op.source.address > last_pc
                        && matches!(
                            op.operation,
                            Operation::Call { .. }
                                | Operation::UnknownInstruction { .. }
                                | Operation::UnknownSideEffect { .. }
                                | Operation::OpaqueMemoryEffect { .. }
                        )
                }) {
                    let mut candidate = last.clone();
                    candidate["status"] = json!("RETURN_VALUE_CANDIDATE");
                    candidate["rule"]=json!("R3_WRITE_REACHES_KNOWN_RETURN_WITHOUT_OPAQUE_EFFECT; Cafe/source return type UNKNOWN");
                    r.return_observations.push(candidate);
                }
            }
        }
    }
    if !inputs.is_empty() {
        r.signatures.push(FunctionSignatureCandidate{candidate:candidate("signature.entry-inputs".into(),inputs.values().copied().collect()),entry_register_inputs:inputs.keys().copied().collect(),return_type:SemanticType::Unknown,abi:"UNKNOWN".into(),scope:"Known reachable CFG; incoming register observations include control/stack state; no proven Cafe ABI parameters".into()});
    }
    for (subject, observations) in interpretations {
        let mut alternatives = vec![];
        let mut evidence = vec![];
        for (ty, eid) in observations {
            if !alternatives.contains(&ty) {
                alternatives.push(ty);
            }
            if !evidence.contains(&eid) {
                evidence.push(eid);
            }
        }
        let mut c = candidate(format!("type.{subject}"), evidence);
        if alternatives.len() > 1 {
            c.status = CandidateStatus::Conflicting;
        }
        r.types.push(TypeCandidate {
            candidate: c,
            subject,
            alternatives,
        });
    }
    for (base, observations) in fields {
        if observations
            .keys()
            .map(|(offset, _)| offset)
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        {
            continue;
        }
        let mut fs = vec![];
        let all: Vec<u32> = observations.values().flatten().copied().collect();
        for ((offset, width), evidence) in &observations {
            let mut c = candidate(format!("field.{base}.{offset}.{width}"), evidence.clone());
            if observations.keys().any(|(o, w)| {
                (*o, *w) != (*offset, *width)
                    && *o < *offset + *width as i64
                    && *offset < *o + *w as i64
            }) {
                c.status = CandidateStatus::Conflicting;
            }
            fs.push(FieldCandidate {
                candidate: c,
                offset: *offset,
                width_bytes: *width,
                ty: SemanticType::Unknown,
            });
        }
        let mut c = candidate(format!("aggregate.{base}"), all);
        if fs
            .iter()
            .any(|f| f.candidate.status == CandidateStatus::Conflicting)
        {
            c.status = CandidateStatus::Conflicting;
        }
        r.aggregates.push(AggregateCandidate {
            candidate: c,
            base,
            fields: fs,
            extent: None,
        });
    }
    r.typed_arguments = r
        .types
        .iter()
        .filter(|t| (3..=10).any(|n| t.subject == format!("incoming.Gpr({n})")))
        .cloned()
        .collect();
    r.inferred_type_kinds = r
        .types
        .iter()
        .flat_map(|t| &t.alternatives)
        .map(|t| {
            match t {
                SemanticType::GuestPointer { .. } => "GuestPointer",
                SemanticType::SignedInteger => "SignedInteger",
                SemanticType::UnsignedInteger => "UnsignedInteger",
                _ => "Unknown",
            }
            .to_string()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let interpreted: usize = r.operations.len();
    r.metrics = json!({"return_value_candidates":r.return_observations.iter().filter(|v|v["status"]=="RETURN_VALUE_CANDIDATE").count(),"observed_r3_writes":r.return_observations.iter().filter(|v|v["status"]=="OBSERVATION").count(),"virtual_call_candidates":0,"inter_block_propagated_facts":r.inter_block_facts.len(),"inter_block_conflicts":r.inter_block_facts.iter().filter(|f|matches!(f.value,Expr::Conflict(_))).count(),"cfg_iterations":flow.iterations,"typed_signature_candidates":usize::from(!r.typed_arguments.is_empty()),"typed_argument_candidates":r.typed_arguments.len(),"object_candidates":0,"functions_analyzed":1,"function_denominator":1,"signature_candidates":r.signatures.len(),"signatures_proven":0,"functions_with_type_candidates":usize::from(!r.types.is_empty()),"type_candidates":r.types.len(),"type_conflicts":r.types.iter().filter(|t|t.candidate.status==CandidateStatus::Conflicting).count(),"aggregate_candidates":r.aggregates.len(),"field_candidates":r.aggregates.iter().map(|a|a.fields.len()).sum::<usize>(),"vtable_candidates":0,"virtual_calls_resolved":0,"class_candidates":0,"platform_import_sites":platform,"platform_sites_categorized":categorized,"call_operations":calls,"memory_operations":memory_ops,"semantic_operations":interpreted,"machine_operation_denominator":total,"domain":"STATIC_MACHINE_IR_HYPOTHESES"});
    Ok(r)
}
impl ProjectService {
    /// Always rederive from current verified IR; persistence never promotes a hypothesis.
    pub fn semantic_analysis(&self, function: Option<&str>) -> Result<Value> {
        self.semantic_analysis_round(function, 0)
    }
    fn semantic_analysis_round(&self, function: Option<&str>, round: usize) -> Result<Value> {
        if round >= 16 {
            return Err(Error::Analysis(
                "SEMANTIC_MACHINE_FEEDBACK_BUDGET_EXHAUSTED".into(),
            ));
        }
        self.refresh_module_hashes()?;
        let ids: Vec<String> = if let Some(id) = function {
            vec![id.into()]
        } else {
            self.db.prepare("SELECT i.function FROM ir_functions i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' ORDER BY i.function LIMIT 4097")?.query_map([],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?
        };
        if ids.len() > 4096 {
            return Err(Error::Analysis("SEMANTIC_FUNCTION_LIMIT".into()));
        }
        let mut items = vec![];
        let mut modules = BTreeSet::new();
        let mut totals = BTreeMap::<String, u64>::new();
        for id in &ids {
            let ir = self.read_ir(id)?;
            modules.insert(ir.module.clone());
            let report = analyze(&ir)?;
            let value = serde_json::to_value(&report)?;
            let bytes = serde_json::to_vec(&report)?;
            if bytes.len() > 16 * 1024 * 1024 {
                return Err(Error::Analysis("SEMANTIC_ARTIFACT_LIMIT".into()));
            }
            let key = crate::hash_bytes(&bytes);
            self.persist_product("SEMANTIC", id, &ir.module, &key, &value)?;
            for (k, v) in report.metrics.as_object().expect("metrics object") {
                if let Some(n) = v.as_u64() {
                    *totals.entry(k.clone()).or_default() += n;
                }
            }
            items.push(json!({"function":id,"ir_hash":report.ir_hash,"metrics":report.metrics}));
            if function.is_some() {
                return Ok(value);
            }
        }
        let mut table_reports = vec![];
        let mut interprocedural_reports = vec![];
        let mut v3_totals = BTreeMap::<String, u64>::new();
        let mut v4_totals = BTreeMap::<String, u64>::new();
        let mut historical_audit = vec![];
        for module in modules {
            let image = self.guest_image(Some(&module))?;
            let functions: BTreeMap<u32, String> = self
                .db
                .prepare("SELECT address,id FROM functions WHERE module=?1 ORDER BY address")?
                .query_map([image.module()], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            let relocations = image
                .relocations()
                .iter()
                .filter(|r| r.kind == 1)
                .map(|r| crate::semantic_tables::EntryEvidence {
                    address: r.place,
                    relocation_index: r.index,
                    relocation_target: r.target,
                    applied_local_addr32: r.state == "APPLIED_PROVEN_LOCAL",
                })
                .collect::<Vec<_>>();
            let mut tables = crate::semantic_tables::scan(image.memory(), &relocations, &functions)
                .map_err(|e| Error::Analysis(e.into()))?;
            let ir_functions: Vec<_> = ids
                .iter()
                .map(|id| self.read_ir(id))
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .filter(|f| f.module == module)
                .collect();
            let initial: Vec<_> = relocations
                .iter()
                .filter(|r| r.applied_local_addr32)
                .filter_map(|r| {
                    r.relocation_target
                        .map(|target| crate::semantic_values::InitialWord {
                            address: r.address,
                            target,
                            evidence: format!("{module}:relocation:{}", r.relocation_index),
                        })
                })
                .collect();
            let interprocedural = crate::semantic_interprocedural::analyze(
                &ir_functions,
                image.memory(),
                &initial,
                image.hash(),
                false,
            )?;
            let reconstruction = crate::semantic_reconstruction::reconstruct(
                &interprocedural,
                &ir_functions,
                image.memory(),
                &tables,
            )?;
            if self.persist_semantic_promotions(&ir_functions, &interprocedural)? > 0 {
                return self.semantic_analysis_round(function, round + 1);
            }
            let historical:Vec<(String,u32,String,String)>=self.db.prepare("SELECT s.function,s.address,s.status,s.evidence FROM indirect_sites s JOIN functions f ON f.id=s.function WHERE f.module=?1 AND s.is_return=0 ORDER BY s.function,s.address")?.query_map([&module],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?.collect::<std::result::Result<_,_>>()?;
            for (fid, pc, status, evidence) in historical {
                let local = interprocedural.summaries.get(&fid);
                let site = local.and_then(|s| s.indirects.iter().find(|s| s.pc == pc));
                let evidence = serde_json::from_str::<Value>(&evidence).unwrap_or(json!(evidence));
                // A promoted call is static in the new IR and therefore no longer
                // occurs in this pass's indirect observations. Retain its proof
                // classification in the audit, without reusing it as proof input.
                let classification = site.map(|s| s.classification.as_str()).unwrap_or_else(|| {
                    if status == "RESOLVED" {
                        if evidence["gate"]["classification"] == "MACHINE_PROVEN_SINGLE_TARGET" {
                            "MACHINE_PROVEN_SINGLE_TARGET"
                        } else {
                            "MACHINE_RESOLVED_HISTORICAL_SITE"
                        }
                    } else {
                        "UNSUPPORTED_PATTERN"
                    }
                });
                historical_audit.push(json!({"function":fid,"pc":pc,"historical_status":status,"classification":classification,"analysis":site,"effect_audit":site.map(|site|crate::semantic_memory_reporting::audit(&interprocedural,site)),"missing_evidence":if site.is_none() && status != "RESOLVED"{Some("No reachable supported Typed IR indirect transfer at this historical PPC site; opaque/unreached CFG paths remain unknown")}else{None},"function_incomplete":local.map(|s|&s.incomplete),"historical_evidence":evidence}));
            }
            let metrics = crate::semantic_reporting::metrics(&interprocedural, &reconstruction);
            for (key, value) in metrics.as_object().expect("metrics") {
                if let Some(n) = value.as_u64() {
                    *v3_totals.entry(key.clone()).or_default() += n;
                }
            }
            let memory_metrics = crate::semantic_memory_reporting::metrics(&interprocedural);
            for (key, value) in memory_metrics.as_object().expect("metrics") {
                if let Some(n) = value.as_u64() {
                    *v4_totals.entry(key.clone()).or_default() += n;
                }
            }
            let storage_addresses: BTreeSet<u32> = tables
                .iter()
                .flat_map(|t| &t.entries)
                .map(|e| e.evidence.address)
                .collect();
            let storage_audits: Vec<_> = storage_addresses
                .into_iter()
                .map(|a| crate::semantic_memory_reporting::storage_audit(&interprocedural, a, 4))
                .collect();
            let inter_value = json!({"analysis":interprocedural,"reconstruction":reconstruction,"metrics":metrics,"memory_metrics":memory_metrics,"storage_audits":storage_audits});
            let inter_bytes = serde_json::to_vec(&inter_value)?;
            if inter_bytes.len() > 64 * 1024 * 1024 {
                return Err(Error::Analysis("SEMANTIC_V3_ARTIFACT_LIMIT".into()));
            }
            self.persist_product(
                "SEMANTIC_INTERPROCEDURAL",
                &module,
                &module,
                &crate::hash_bytes(&inter_bytes),
                &inter_value,
            )?;
            interprocedural_reports.push(inter_value);
            for id in &ids {
                let ir = self.read_ir(id)?;
                if ir.module != module {
                    continue;
                }
                for block in &ir.blocks {
                    for op in &block.instructions {
                        if let Operation::RelocationImmediate { relocation, .. } = &op.operation {
                            if let Some(address) = relocation.local_address {
                                for table in &mut tables {
                                    if address >= table.address
                                        && (address as u64)
                                            < table.address as u64 + table.byte_length as u64
                                    {
                                        table.consumers.push(json!({"function":id,"block":block.id,"pc":op.source.address,"operation":op.id,"rule":"RELOCATION_MATERIALIZATION_REFERENCES_CANDIDATE_RANGE; load/call flow not proven"}));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            *totals
                .entry("function_pointer_candidates".into())
                .or_default() += tables
                .iter()
                .flat_map(|t| &t.entries)
                .filter(|e| e.conflicts.is_empty())
                .count() as u64;
            *totals.entry("table_candidates".into()).or_default() += tables
                .iter()
                .filter(|t| t.status == "HYPOTHESIS" && t.entries.len() >= 2)
                .count()
                as u64;
            *totals
                .entry("table_storage_runs_examined".into())
                .or_default() += tables.len() as u64;
            totals.insert("semantic_indirect_candidates".into(), 0);
            totals.insert("machine_proven_indirect_targets_from_semantics".into(), 0);
            let table_report = json!({"version":SEMANTIC_VERSION,"module":image.module(),"module_hash":image.module_hash(),"image_hash":image.hash(),"tables":tables,"machine_target_promotion":"Separate freshly derived machine proof gate; table candidacy is insufficient"});
            let value = serde_json::to_vec(&table_report)?;
            if value.len() > 16 * 1024 * 1024 {
                return Err(Error::Analysis("SEMANTIC_TABLE_ARTIFACT_LIMIT".into()));
            }
            self.persist_product(
                "SEMANTIC_TABLES",
                &module,
                &module,
                &crate::hash_bytes(&value),
                &table_report,
            )?;
            table_reports.push(table_report);
        }
        self.output.write_atomic(
            "reports/semantic-tables.json",
            &serde_json::to_vec_pretty(
                &json!({"version":SEMANTIC_VERSION,"modules":table_reports}),
            )?,
        )?;
        self.output.write_atomic(
            "reports/semantic-interprocedural.json",
            &serde_json::to_vec(
                &json!({"version":SEMANTIC_VERSION,"modules":interprocedural_reports}),
            )?,
        )?;
        self.output.write_atomic("reports/semantic-indirect-audit.json",&serde_json::to_vec_pretty(&json!({"version":SEMANTIC_VERSION,"sites":historical_audit,"scope":"Every persisted non-return site; no unsupported site silently omitted"}))?)?;
        let promoted: usize = self.db.query_row(
            "SELECT count(*) FROM analysis_products WHERE kind='MACHINE_PROOF'",
            [],
            |r| r.get(0),
        )?;
        let summary = json!({"version":SEMANTIC_VERSION,"domain":"STATIC_MACHINE_IR_HYPOTHESES","metrics":totals,"v3_metrics":v3_totals,"v4_metrics":v4_totals,"machine_promoted_targets":promoted,"items":items,"interprocedural_report":"reports/semantic-interprocedural.json","indirect_audit":"reports/semantic-indirect-audit.json","effects_on_machine_analysis":"Only freshly derived complete single-target proofs can promote; hypotheses never authorize execution","constructor_destructor_candidates":"NOT_IMPLEMENTED"});
        self.output.write_atomic(
            "reports/semantic-summary.json",
            &serde_json::to_vec_pretty(&summary)?,
        )?;
        Ok(summary)
    }
}
