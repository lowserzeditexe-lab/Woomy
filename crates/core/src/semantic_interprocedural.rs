//! Bounded semantic reconstruction from current Typed IR, never from cached hypotheses.
use crate::{semantic_values::*, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{
    runtime::GuestMemory, Binary, CallTarget, Destination, Function, Operation, Terminator, ValueId,
};
use woomy_ppc::Register;

pub const INTERPROCEDURAL_VERSION: u32 = 2;
pub const PROOF_GATE_VERSION: u32 = 2;
const MAX_ITERATIONS: usize = 128;
const MAX_NODES: usize = 262144;
const MAX_EDGES: usize = 524288;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub module: String,
    pub function: String,
    pub block: u32,
    pub pc: u32,
    pub operation: Option<ValueId>,
    pub kind: String,
    pub value: Fact,
    pub inputs: BTreeSet<String>,
    pub evidence_class: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    pub node: String,
    pub address: Fact,
    pub location: Option<Location>,
    pub width: u8,
    pub write: bool,
    pub value: Fact,
    pub reaching_status: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub inputs: Vec<(Register, Fact)>,
    pub node: String,
    pub target: CallTarget,
    pub callee: Option<String>,
    pub arguments: Vec<(Register, Fact)>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Indirect {
    pub node: String,
    pub pc: u32,
    pub source: Register,
    pub value: Fact,
    pub classification: String,
    pub targets: Vec<u32>,
    pub failed_requirements: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub call_effects: Vec<crate::semantic_memory::CallEffect>,
    pub function: String,
    pub read_before_definition: BTreeSet<Register>,
    pub candidate_arguments: BTreeSet<Register>,
    pub definitely_defined: BTreeSet<Register>,
    pub possibly_modified: BTreeSet<Register>,
    pub candidate_return: Fact,
    pub source_return_type: String,
    pub memory_read: bool,
    pub memory_write: bool,
    pub memory_unknown: bool,
    pub calls: Vec<Call>,
    pub accesses: Vec<Access>,
    pub indirects: Vec<Indirect>,
    pub incomplete: BTreeSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constraint {
    pub caller: String,
    pub callee: String,
    pub site: String,
    pub register: Register,
    pub value: Fact,
    pub rule: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub memory_effects: BTreeMap<String, crate::semantic_memory::MemorySummary>,
    pub versions: BTreeMap<String, u32>,
    pub identity: String,
    pub image_hash: String,
    pub ir_hashes: BTreeMap<String, String>,
    pub iterations: usize,
    pub converged: bool,
    pub entry_assumption: String,
    pub call_sccs: Vec<Vec<String>>,
    pub summaries: BTreeMap<String, Summary>,
    pub nodes: Vec<Node>,
    pub constraints: Vec<Constraint>,
    pub limitations: Vec<String>,
}
#[derive(Clone, Default, PartialEq, Eq)]
struct State {
    regs: BTreeMap<Register, Fact>,
    storage: Storage,
    defined: BTreeSet<Register>,
}
struct Environment<'a> {
    memory_effects: &'a BTreeMap<String, crate::semantic_memory::MemorySummary>,
    memory: &'a GuestMemory,
    initial: &'a BTreeMap<u32, InitialWord>,
    functions: &'a BTreeMap<(String, u32), String>,
    summaries: &'a BTreeMap<String, Summary>,
    machine: &'a BTreeMap<String, woomy_ir::calls::Summary>,
    loader_entry: bool,
}
fn incoming(function: &str, reg: Register) -> Fact {
    Fact::atom(Atom::Incoming {
        function: function.into(),
        register: reg,
        offset: 0,
    })
    .at(&format!("{function}:input:{reg:?}"))
}
fn node_id(f: &Function, b: u32, id: ValueId) -> String {
    format!("{}:{b:08X}:{:08X}:{}", f.id, id.address, id.slot)
}
fn successors(t: &Terminator) -> Vec<u32> {
    match t {
        Terminator::ConditionalLrBranch {
            not_taken: Destination::Block(target),
            ..
        } => vec![*target],
        Terminator::Jump {
            target: Destination::Block(b),
        } => vec![*b],
        Terminator::Branch {
            taken, not_taken, ..
        } => [taken, not_taken]
            .into_iter()
            .filter_map(|d| {
                if let Destination::Block(b) = d {
                    Some(*b)
                } else {
                    None
                }
            })
            .collect(),
        _ => vec![],
    }
}
pub(crate) fn substitute(value: &Fact, callee: &str, regs: &BTreeMap<Register, Fact>) -> Fact {
    let mut out = Fact {
        unknown: value.unknown.clone(),
        definitions: value.definitions.clone(),
        ..Fact::default()
    };
    for atom in &value.alternatives {
        if let Atom::Incoming {
            function,
            register,
            offset,
        } = atom
        {
            if function == callee {
                let arg = regs
                    .get(register)
                    .cloned()
                    .unwrap_or_else(|| Fact::unknown("UNKNOWN_CALL_ARGUMENT"));
                let mut v = arg.add(&Fact::bits(*offset as u32));
                v.definitions = arg.definitions;
                out.merge(&v);
                continue;
            }
        }
        out.merge(&Fact::atom(atom.clone()));
    }
    out
}
fn gate(value: &Fact, env: &Environment<'_>, complete: bool) -> (String, Vec<u32>, Vec<String>) {
    let mut failures = Vec::new();
    let mut targets = BTreeSet::new();
    if !complete {
        failures.push("INCOMPLETE_CFG_OR_FIXED_POINT".into());
    }
    if !value.unknown.is_empty() || value.alternatives.is_empty() {
        failures.push("ALL_REACHING_DEFINITIONS_NOT_KNOWN".into());
    }
    for atom in &value.alternatives {
        match atom {
            Atom::Bits {
                bits,
                address_source: Some(origin),
            } if bits == origin && bits & 3 == 0 => {
                if env
                    .memory
                    .locate(*bits, 4, woomy_ir::runtime::EXECUTE)
                    .is_none()
                {
                    failures.push("NON_EXECUTABLE_TARGET".into());
                } else if !env.functions.keys().any(|(_, a)| a == bits) {
                    failures.push("NOT_KNOWN_FUNCTION_ENTRY".into());
                } else {
                    targets.insert(*bits);
                }
            }
            _ => failures.push("NO_FUNCTION_ADDRESS_PROVENANCE".into()),
        }
    }
    if targets.len() > 1 {
        failures.push("FINITE_SET_REQUIRES_MACHINE_DISPATCH_SUPPORT".into());
    }
    failures.sort();
    failures.dedup();
    let class = if failures.is_empty() && targets.len() == 1 {
        "MACHINE_PROVEN_SINGLE_TARGET"
    } else if value.unknown.iter().any(|s| s.contains("ALIAS")) {
        "UNKNOWN_ALIAS"
    } else if value.unknown.iter().any(|s| s.contains("CALL")) {
        "UNKNOWN_CALL_EFFECT"
    } else if value.unknown.iter().any(|s| s.contains("MUTABLE")) {
        "MUTABLE_STORAGE"
    } else if targets.len() > 1 {
        "FINITE_TARGET_SET"
    } else if !targets.is_empty() {
        "FUNCTION_POINTER_CANDIDATE"
    } else {
        "UNKNOWN_VALUE"
    };
    (class.into(), targets.into_iter().collect(), failures)
}
fn transfer(
    f: &Function,
    b: &woomy_ir::Block,
    mut state: State,
    env: &Environment<'_>,
    summary: &mut Summary,
    nodes: &mut BTreeMap<String, Node>,
) -> State {
    let mut values = BTreeMap::<ValueId, Fact>::new();
    for i in &b.instructions {
        let id = node_id(f, b.id, i.id);
        let get = |v: &ValueId| {
            values
                .get(v)
                .cloned()
                .unwrap_or_else(|| Fact::unknown("UNKNOWN_IR_VALUE"))
        };
        let mut inputs = BTreeSet::new();
        let mut kind = "Unknown";
        let mut value = match &i.operation {
            Operation::Constant { bits } => {
                kind = "Constant";
                if *bits <= u32::MAX as u64 {
                    Fact::bits(*bits as u32)
                } else {
                    Fact::unknown("NON_32_BIT_VALUE")
                }
            }
            Operation::RelocationImmediate {
                relocation,
                signed,
                shift,
            } => {
                kind = "Relocation";
                relocation
                    .immediate(*signed, *shift)
                    .map(|bits| {
                        Fact::atom(Atom::Bits {
                            bits,
                            address_source: relocation.local_address,
                        })
                    })
                    .unwrap_or_else(|| Fact::unknown("UNRESOLVED_RELOCATION"))
            }
            Operation::ReadRegister { register } => {
                kind = "Register";
                let value = state
                    .regs
                    .get(register)
                    .cloned()
                    .unwrap_or_else(|| Fact::unknown("UNKNOWN_REGISTER"));
                if value.definitions.len() > 1 {
                    kind = "PhiJoin";
                }
                if !state.defined.contains(register) {
                    summary.read_before_definition.insert(*register);
                }
                if matches!(register, Register::Gpr(3..=10))
                    && value
                        .alternatives
                        .iter()
                        .any(|a| matches!(a,Atom::Incoming {function,..} if function==&f.id))
                {
                    summary.candidate_arguments.insert(*register);
                }
                value
            }
            Operation::WriteRegister { register, value } => {
                kind = "Register";
                let value = match register {
                    Register::Cr(_) => get(value).truncate(15),
                    Register::Fpr(_) | Register::Reservation => {
                        Fact::unknown("NON_INTEGER_REGISTER_VALUE")
                    }
                    _ => get(value),
                };
                state.regs.insert(*register, value.clone().at(&id));
                state.defined.insert(*register);
                summary.possibly_modified.insert(*register);
                value
            }
            Operation::Address32 { value } => {
                kind = "AddressOf";
                get(value)
            }
            Operation::Binary {
                operator: Binary::AddWrapping,
                lhs,
                rhs,
            } => {
                kind = "AddWrapping";
                let a = get(lhs);
                let c = get(rhs);
                inputs.extend(a.definitions.clone());
                inputs.extend(c.definitions.clone());
                a.add(&c)
            }
            Operation::Load {
                address,
                width,
                endian,
                ..
            } => {
                kind = "Load";
                summary.memory_read = true;
                let a = get(address);
                inputs.extend(a.definitions.clone());
                let location = a.address();
                let value = if *endian == woomy_ir::Endian::Big {
                    state
                        .storage
                        .load_value(&a, *width, env.memory, env.initial, env.loader_entry)
                } else {
                    Fact::unknown("UNSUPPORTED_ENDIAN")
                };
                summary.accesses.push(Access {
                    node: id.clone(),
                    address: a.clone(),
                    location,
                    width: *width,
                    write: false,
                    reaching_status: value.status().into(),
                    value: value.clone(),
                });
                value
            }
            Operation::Store {
                address,
                value,
                width,
                endian,
                ..
            } => {
                kind = "Store";
                summary.memory_write = true;
                let a = get(address);
                inputs.extend(a.definitions.clone());
                let location = a.address();
                let value = get(value);
                summary.accesses.push(Access {
                    node: id.clone(),
                    address: a.clone(),
                    location: location.clone(),
                    width: *width,
                    write: true,
                    reaching_status: value.status().into(),
                    value: value.clone(),
                });
                if *endian == woomy_ir::Endian::Big {
                    let unknown_address = location.is_none();
                    state
                        .storage
                        .store(location, *width, value.clone().at(&id), env.memory);
                    if unknown_address {
                        state.storage.invalidate_at("UNKNOWN_ALIAS", &id);
                    }
                } else {
                    state.storage.invalidate("UNSUPPORTED_STORE_ENDIAN");
                }
                value
            }
            Operation::Call { target, .. } => {
                kind = "CallReturn";
                let callee = match target {
                    CallTarget::Static { module, address } => {
                        env.functions.get(&(module.clone(), *address)).cloned()
                    }
                    _ => None,
                };
                let known = callee.as_ref().and_then(|c| env.summaries.get(c));
                let args: Vec<(Register, Fact)> =
                    known
                        .map(|s| {
                            s.candidate_arguments
                                .iter()
                                .map(|r| {
                                    (
                                        *r,
                                        state.regs.get(r).cloned().unwrap_or_else(|| {
                                            Fact::unknown("UNKNOWN_CALL_ARGUMENT")
                                        }),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                for (_, arg) in &args {
                    for atom in &arg.alternatives {
                        if let Atom::Incoming {
                            function, register, ..
                        } = atom
                        {
                            if function == &f.id {
                                summary.read_before_definition.insert(*register);
                                if matches!(register, Register::Gpr(3..=10)) {
                                    summary.candidate_arguments.insert(*register);
                                }
                            }
                        }
                    }
                }
                summary.calls.push(Call {
                    inputs: state.regs.iter().map(|(r, v)| (*r, v.clone())).collect(),
                    node: id.clone(),
                    target: target.clone(),
                    callee: callee.clone(),
                    arguments: args,
                });
                if let CallTarget::Indirect { source } = target {
                    let v = state
                        .regs
                        .get(source)
                        .cloned()
                        .unwrap_or_else(|| Fact::unknown("UNKNOWN_TARGET_REGISTER"));
                    inputs.extend(v.definitions.clone());
                    let (classification, targets, failed_requirements) =
                        gate(&v, env, summary.incomplete.is_empty());
                    summary.indirects.push(Indirect {
                        node: id.clone(),
                        pc: i.source.address,
                        source: *source,
                        value: v,
                        classification,
                        targets,
                        failed_requirements,
                    });
                }
                let ret = known
                    .filter(|s| s.incomplete.is_empty())
                    .map(|s| substitute(&s.candidate_return, &s.function, &state.regs))
                    .unwrap_or_else(|| Fact::unknown("UNKNOWN_CALLEE_RETURN"));
                let effects = callee.as_ref().and_then(|c| env.machine.get(c));
                if let Some(e) = effects.filter(|e| {
                    e.fixed_point && e.effects.certainty == woomy_ir::calls::Certainty::Known
                }) {
                    for r in e
                        .effects
                        .registers_written
                        .union(&e.effects.registers_clobbered)
                    {
                        state
                            .regs
                            .insert(*r, Fact::unknown("CALL_REGISTER_WRITE").at(&id));
                    }
                } else {
                    for v in state.regs.values_mut() {
                        *v = Fact::unknown("UNKNOWN_CALL_EFFECT").at(&id);
                    }
                    summary.memory_unknown = true;
                }
                let call = summary.calls.last().expect("recorded call");
                let memory_effect = crate::semantic_memory::apply(
                    call,
                    callee.as_ref().and_then(|c| env.memory_effects.get(c)),
                    &mut state.storage,
                );
                for effect in &memory_effect.effects {
                    inputs.extend(effect.provenance.iter().cloned());
                    inputs.extend(effect.via_calls.iter().cloned());
                }
                summary.call_effects.push(memory_effect);
                if known.is_some_and(|s| s.definitely_defined.contains(&Register::Gpr(3))) {
                    state.defined.insert(Register::Gpr(3));
                }
                state.regs.insert(Register::Gpr(3), ret.clone().at(&id));
                ret
            }
            Operation::UnknownInstruction { .. }
            | Operation::UnknownSideEffect { .. }
            | Operation::OpaqueMemoryEffect { .. } => {
                state.storage.invalidate_at("UNKNOWN_WRITER", &id);
                for v in state.regs.values_mut() {
                    *v = Fact::unknown("UNKNOWN_INSTRUCTION_EFFECT").at(&id);
                }
                summary.memory_unknown = true;
                Fact::unknown("UNKNOWN_INSTRUCTION_EFFECT")
            }
            Operation::BranchCondition {
                decrement_ctr: true,
                ..
            } => {
                state
                    .regs
                    .insert(Register::Ctr, Fact::unknown("CTR_DECREMENT"));
                Fact::unknown("BRANCH_CONDITION")
            }
            _ => Fact::unknown("UNSUPPORTED_VALUE_OPERATION"),
        };
        inputs.extend(value.definitions.iter().cloned());
        inputs.remove(&id);
        nodes.insert(
            id.clone(),
            Node {
                id: id.clone(),
                module: f.module.clone(),
                function: f.id.clone(),
                block: b.id,
                pc: i.source.address,
                operation: Some(i.id),
                kind: kind.into(),
                value: value.clone(),
                inputs,
                evidence_class: "WOOMY_INFERRED_FROM_TYPED_IR".into(),
            },
        );
        value = value.at(&id);
        values.insert(i.id, value);
    }
    match b.terminator {
        Terminator::Return { .. } => {
            if state.defined.contains(&Register::Gpr(3)) {
                summary.candidate_return.merge(
                    &state
                        .regs
                        .get(&Register::Gpr(3))
                        .cloned()
                        .unwrap_or_else(|| Fact::unknown("UNKNOWN_RETURN")),
                );
            } else {
                summary
                    .candidate_return
                    .merge(&Fact::unknown("R3_NOT_DEFINITELY_DEFINED"));
            }
            summary.definitely_defined = summary
                .definitely_defined
                .intersection(&state.defined)
                .copied()
                .collect();
        }
        Terminator::UnresolvedIndirectBranch { .. } | Terminator::ConditionalLrBranch { .. } => {
            let source = match b.terminator {
                Terminator::UnresolvedIndirectBranch { source, .. } => source,
                _ => Register::Lr,
            };
            let value = state
                .regs
                .get(&source)
                .cloned()
                .unwrap_or_else(|| Fact::unknown("UNKNOWN_TARGET_REGISTER"));
            let (classification, targets, failed_requirements) =
                gate(&value, env, summary.incomplete.is_empty());
            summary.indirects.push(Indirect {
                node: format!("{}:{:08X}:terminator", f.id, b.id),
                pc: b.terminator_source.address,
                source,
                value,
                classification,
                targets,
                failed_requirements,
            });
        }
        _ => {}
    }
    state
}

fn analyze_function(f: &Function, env: &Environment<'_>) -> (Summary, BTreeMap<String, Node>) {
    let mut initial = State::default();
    for r in woomy_ppc::all_registers() {
        initial.regs.insert(r, incoming(&f.id, r));
    }
    let mut pred = BTreeMap::<u32, Vec<u32>>::new();
    for b in &f.blocks {
        for s in successors(&b.terminator) {
            pred.entry(s).or_default().push(b.id);
        }
    }
    let base = Summary {
        call_effects: vec![],
        function: f.id.clone(),
        read_before_definition: BTreeSet::new(),
        candidate_arguments: BTreeSet::new(),
        definitely_defined: woomy_ppc::all_registers(),
        possibly_modified: BTreeSet::new(),
        candidate_return: Fact::default(),
        source_return_type: "UNKNOWN".into(),
        memory_read: false,
        memory_write: false,
        memory_unknown: false,
        calls: vec![],
        accesses: vec![],
        indirects: vec![],
        incomplete: BTreeSet::new(),
    };
    let mut outputs = BTreeMap::<u32, State>::new();
    let mut final_summary = base.clone();
    let mut final_nodes = BTreeMap::new();
    let mut converged = false;
    for _ in 0..MAX_ITERATIONS {
        let previous = outputs.clone();
        let mut summary = base.clone();
        let mut nodes = BTreeMap::new();
        if f.blocks.iter().any(|b| {
            matches!(
                b.terminator,
                Terminator::OpaqueTerminator { .. }
                    | Terminator::UnresolvedIndirectBranch { .. }
                    | Terminator::ConditionalLrBranch { .. }
                    | Terminator::Jump {
                        target: Destination::Unknown | Destination::External(_)
                    }
            ) || matches!(&b.terminator,Terminator::Branch {taken,not_taken,..} if !matches!(taken,Destination::Block(_)) || !matches!(not_taken,Destination::Block(_)))
        }) {
            summary.incomplete.insert("UNSUPPORTED_CFG_BOUNDARY".into());
        }
        if env.memory.regions.iter().any(|r| {
            r.permissions & (woomy_ir::runtime::WRITE | woomy_ir::runtime::EXECUTE)
                == (woomy_ir::runtime::WRITE | woomy_ir::runtime::EXECUTE)
        }) {
            summary
                .incomplete
                .insert("WRITABLE_EXECUTABLE_MEMORY".into());
        }
        if f.blocks.iter().flat_map(|b| &b.instructions).any(|i| {
            matches!(
                &i.operation,
                Operation::UnknownInstruction { .. }
                    | Operation::UnknownSideEffect { .. }
                    | Operation::OpaqueMemoryEffect { .. }
                    | Operation::RelocationImmediate {
                        relocation: woomy_ir::Relocation {
                            local_address: None,
                            ..
                        },
                        ..
                    }
            )
        }) {
            summary
                .incomplete
                .insert("UNKNOWN_SOURCE_EFFECT_OR_RELOCATION".into());
        }
        for b in &f.blocks {
            let mut states: Vec<&State> = pred
                .get(&b.id)
                .into_iter()
                .flatten()
                .filter_map(|p| outputs.get(p))
                .collect();
            if b.id == f.entry {
                states.push(&initial);
            }
            if states.is_empty() {
                continue;
            }
            let mut state = State {
                defined: woomy_ppc::all_registers(),
                ..State::default()
            };
            for r in woomy_ppc::all_registers() {
                let mut v = Fact::default();
                for s in &states {
                    v.merge(
                        &s.regs
                            .get(&r)
                            .cloned()
                            .unwrap_or_else(|| Fact::unknown("UNKNOWN_PREDECESSOR")),
                    );
                }
                state.regs.insert(r, v);
            }
            for s in &states {
                state.defined = state.defined.intersection(&s.defined).copied().collect();
            }
            state.storage = Storage::join(
                &states.iter().map(|s| &s.storage).collect::<Vec<_>>(),
                env.memory,
                env.initial,
                env.loader_entry,
            );
            outputs.insert(b.id, transfer(f, b, state, env, &mut summary, &mut nodes));
        }
        final_summary = summary;
        final_nodes = nodes;
        if outputs == previous {
            converged = true;
            break;
        }
    }
    if !converged {
        final_summary
            .incomplete
            .insert("CFG_FIXED_POINT_BUDGET_EXHAUSTED".into());
        final_summary.candidate_return = Fact::unknown("CFG_FIXED_POINT_BUDGET_EXHAUSTED");
    }
    if final_summary.candidate_return.alternatives.is_empty()
        && final_summary.candidate_return.unknown.is_empty()
    {
        final_summary.candidate_return = Fact::unknown("NO_OBSERVED_RETURN");
        final_summary.definitely_defined.clear();
    }
    if let Some(machine) = env.machine.get(&f.id) {
        final_summary.possibly_modified.extend(
            machine
                .effects
                .registers_written
                .iter()
                .chain(&machine.effects.registers_clobbered)
                .copied(),
        );
        final_summary.memory_read |= machine.effects.memory_read;
        final_summary.memory_write |= machine.effects.memory_write;
        final_summary.memory_unknown |= machine.effects.memory_unknown;
    }
    for site in &mut final_summary.indirects {
        let (c, t, e) = gate(&site.value, env, final_summary.incomplete.is_empty());
        site.classification = c;
        site.targets = t;
        site.failed_requirements = e;
    }
    (final_summary, final_nodes)
}

/// `loader_entry` is an explicit synthetic/loader-entry assumption, never the
/// default for a function in a loaded image. It is part of the product identity.
pub fn analyze(
    functions: &[Function],
    memory: &GuestMemory,
    initial: &[InitialWord],
    image_hash: &str,
    loader_entry: bool,
) -> Result<Report> {
    if functions.len() > 4096
        || functions
            .iter()
            .flat_map(|f| &f.blocks)
            .map(|b| b.instructions.len())
            .sum::<usize>()
            > MAX_NODES
        || !memory.valid_with_limit(256 * 1024 * 1024)
        || initial.len() > 524288
    {
        return Err(Error::Analysis("SEMANTIC_V3_INPUT_BUDGET".into()));
    }
    let mut ordered: Vec<_> = functions.iter().collect();
    ordered.sort_by(|a, b| a.id.cmp(&b.id));
    let mut ir_hashes = BTreeMap::new();
    let mut lookup = BTreeMap::new();
    for f in &ordered {
        if !woomy_ir::verify(f).structural_valid
            || f.cfg_version != woomy_ppc::CFG_ANALYZER_VERSION
            || f.analysis_version != woomy_ppc::DATAFLOW_VERSION
        {
            return Err(Error::Analysis("SEMANTIC_V3_INVALID_IR".into()));
        }
        if ordered
            .first()
            .is_some_and(|first| first.module != f.module || first.module_hash != f.module_hash)
        {
            return Err(Error::Analysis(
                "SEMANTIC_V3_MODULE_IDENTITY_MISMATCH".into(),
            ));
        }
        if f.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter(|i| matches!(i.operation, Operation::Call { .. }))
            .count()
            > 4096
        {
            return Err(Error::Analysis(
                "SEMANTIC_V3_CALLEE_BUDGET_EXHAUSTED".into(),
            ));
        }
        if ir_hashes
            .insert(f.id.clone(), crate::hash_bytes(&serde_json::to_vec(f)?))
            .is_some()
            || lookup
                .insert((f.module.clone(), f.entry), f.id.clone())
                .is_some()
        {
            return Err(Error::Analysis("SEMANTIC_V3_DUPLICATE_FUNCTION".into()));
        }
    }
    let versions = BTreeMap::from([
        ("semantic".into(), 4),
        (
            "memory_effect".into(),
            crate::semantic_memory::MEMORY_EFFECT_VERSION,
        ),
        ("alias".into(), crate::semantic_memory::ALIAS_VERSION),
        (
            "effect_policy".into(),
            crate::semantic_memory::EFFECT_POLICY_VERSION,
        ),
        ("image".into(), woomy_ir::runtime::CAFE_IMAGE_VERSION),
        ("loader".into(), woomy_ir::runtime::CAFE_LOADER_VERSION),
        (
            "relocation".into(),
            woomy_ir::runtime::RELOCATION_APPLIER_VERSION,
        ),
        ("interprocedural".into(), INTERPROCEDURAL_VERSION),
        ("provenance".into(), VALUE_PROVENANCE_VERSION),
        ("proof_gate".into(), PROOF_GATE_VERSION),
        ("ir".into(), woomy_ir::IR_VERSION),
        ("cfg".into(), woomy_ppc::CFG_ANALYZER_VERSION),
        ("dataflow".into(), woomy_ppc::DATAFLOW_VERSION),
        (
            "machine_summary".into(),
            woomy_ir::runtime::FUNCTION_SUMMARY_VERSION,
        ),
    ]);
    let mut words = BTreeMap::new();
    for w in initial {
        if words.insert(w.address, w.clone()).is_some() {
            return Err(Error::Analysis(
                "SEMANTIC_V3_DUPLICATE_INITIALIZATION".into(),
            ));
        }
    }
    let identity = crate::hash_bytes(&serde_json::to_vec(&(
        &versions,
        &ir_hashes,
        image_hash,
        crate::hash_bytes(&serde_json::to_vec(memory)?),
        &words,
        loader_entry,
    ))?);
    let machine = woomy_ir::calls::summarize(functions);
    let mut summaries = BTreeMap::new();
    let mut memory_effects = BTreeMap::new();
    let mut nodes = BTreeMap::new();
    let mut converged = false;
    let mut iterations = 0;
    for round in 0..MAX_ITERATIONS {
        iterations = round + 1;
        let env = Environment {
            memory_effects: &memory_effects,
            memory,
            initial: &words,
            functions: &lookup,
            summaries: &summaries,
            machine: &machine,
            loader_entry,
        };
        let mut next = BTreeMap::new();
        nodes.clear();
        for f in &ordered {
            let (s, n) = analyze_function(f, &env);
            next.insert(f.id.clone(), s);
            nodes.extend(n);
        }
        if nodes.len() > MAX_NODES
            || nodes.values().map(|n| n.inputs.len()).sum::<usize>() > MAX_EDGES
        {
            return Err(Error::Analysis(
                "SEMANTIC_V3_PROVENANCE_BUDGET_EXHAUSTED".into(),
            ));
        }
        let next_effects = crate::semantic_memory::summarize(&next, &nodes, &ordered);
        if next == summaries && next_effects == memory_effects {
            converged = true;
            summaries = next;
            break;
        }
        summaries = next;
        memory_effects = next_effects;
    }
    if !converged {
        for summary in summaries.values_mut() {
            summary
                .incomplete
                .insert("INTERPROCEDURAL_BUDGET_EXHAUSTED".into());
            for s in &mut summary.indirects {
                s.classification = "UNKNOWN_VALUE".into();
                s.failed_requirements
                    .push("INTERPROCEDURAL_BUDGET_EXHAUSTED".into());
            }
        }
    }
    let mut constraints = vec![];
    for summary in summaries.values() {
        for call in &summary.calls {
            if let Some(callee) = &call.callee {
                for (register, value) in &call.arguments {
                    constraints.push(Constraint {
                        caller: summary.function.clone(),
                        callee: callee.clone(),
                        site: call.node.clone(),
                        register: *register,
                        value: value.clone(),
                        rule: "CALLER_VALUE_TO_OBSERVED_CALLEE_INPUT; NOT_ABI_PROOF".into(),
                    });
                }
            }
        }
    }
    let call_sccs = call_sccs(&summaries);
    for constraint in &constraints {
        if let Some(site) = nodes.get(&constraint.site).cloned() {
            let id = format!("{}:argument:{:?}", constraint.site, constraint.register);
            nodes.insert(
                id.clone(),
                Node {
                    id,
                    module: site.module,
                    function: site.function,
                    block: site.block,
                    pc: site.pc,
                    operation: site.operation,
                    kind: "CallArgument".into(),
                    value: constraint.value.clone(),
                    inputs: constraint.value.definitions.clone(),
                    evidence_class: "WOOMY_INFERRED_CALL_CONSTRAINT_NOT_ABI_PROOF".into(),
                },
            );
        }
    }
    // Materialize external graph roots, rather than leaving dangling IDs.
    let missing: BTreeSet<_> = nodes
        .values()
        .flat_map(|n| n.inputs.iter())
        .filter(|id| !nodes.contains_key(*id))
        .cloned()
        .collect();
    for id in missing {
        let owner = ordered
            .iter()
            .find(|f| id.starts_with(&format!("{}:input:", f.id)));
        let (module, function, block, pc, kind) = if let Some(f) = owner {
            (
                f.module.clone(),
                f.id.clone(),
                f.entry,
                f.entry,
                "IncomingRegister",
            )
        } else {
            (
                ordered
                    .first()
                    .map(|f| f.module.clone())
                    .unwrap_or_default(),
                String::new(),
                0,
                0,
                if words.values().any(|w| w.evidence == id) {
                    "RelocationInitialization"
                } else {
                    "Unknown"
                },
            )
        };
        let value = if let Some(f) = owner {
            woomy_ppc::all_registers()
                .into_iter()
                .find(|r| id == format!("{}:input:{r:?}", f.id))
                .map(|r| incoming(&f.id, r))
                .unwrap_or_else(|| Fact::unknown("UNRECOGNIZED_INPUT_ROOT"))
        } else if let Some(word) = words.values().find(|w| w.evidence == id) {
            Fact::atom(Atom::Bits {
                bits: word.target,
                address_source: Some(word.target),
            })
        } else {
            Fact::unknown("UNKNOWN_PROVENANCE_ROOT")
        };
        nodes.insert(
            id.clone(),
            Node {
                id,
                module,
                function,
                block,
                pc,
                operation: None,
                kind: kind.into(),
                value,
                inputs: BTreeSet::new(),
                evidence_class: "FACT_INPUT_IDENTITY".into(),
            },
        );
    }
    if nodes.len() > MAX_NODES {
        return Err(Error::Analysis("SEMANTIC_V3_ROOT_BUDGET_EXHAUSTED".into()));
    }
    Ok(Report {memory_effects,versions,identity,image_hash:image_hash.into(),ir_hashes,iterations,converged,entry_assumption:if loader_entry{"EXPLICIT_LOADER_ENTRY"}else{"UNKNOWN_RUNTIME_ENTRY"}.into(),call_sccs,summaries,nodes:nodes.into_values().collect(),constraints,limitations:vec!["Source signatures and object identities remain hypotheses; imports have unknown effects".into(),"Known CFG paths only; loader_entry=false does not assume mutable loader bytes survive until function entry".into(),"Finite target sets retained but not promoted: Typed IR has no guarded finite-target call form".into()]})
}

fn call_sccs(summaries: &BTreeMap<String, Summary>) -> Vec<Vec<String>> {
    let edges: BTreeMap<String, BTreeSet<String>> = summaries
        .iter()
        .map(|(id, s)| {
            (
                id.clone(),
                s.calls
                    .iter()
                    .filter_map(|c| c.callee.clone())
                    .filter(|c| summaries.contains_key(c))
                    .collect(),
            )
        })
        .collect();
    let mut reverse = BTreeMap::<String, BTreeSet<String>>::new();
    for (a, bs) in &edges {
        for b in bs {
            reverse.entry(b.clone()).or_default().insert(a.clone());
        }
    }
    let mut seen = BTreeSet::new();
    let mut order = vec![];
    for root in edges.keys() {
        let mut stack = vec![(root.clone(), false)];
        while let Some((node, done)) = stack.pop() {
            if done {
                order.push(node);
                continue;
            }
            if !seen.insert(node.clone()) {
                continue;
            }
            stack.push((node.clone(), true));
            for dst in &edges[&node] {
                if !seen.contains(dst) {
                    stack.push((dst.clone(), false));
                }
            }
        }
    }
    seen.clear();
    let mut result = vec![];
    for root in order.into_iter().rev() {
        if seen.contains(&root) {
            continue;
        }
        let mut component = vec![];
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            if !seen.insert(node.clone()) {
                continue;
            }
            component.push(node.clone());
            if let Some(es) = reverse.get(&node) {
                stack.extend(es.iter().cloned());
            }
        }
        component.sort();
        result.push(component);
    }
    result.sort();
    result
}
