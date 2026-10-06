//! Conservative entry requirements; unknown input is not a zero-initialized register.
use crate::cafe::CafeGuestImage;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{
    runtime::ImageContextVersions, Binary, Destination, Function, Operation, Terminator, ValueId,
};
use woomy_ppc::Register;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputProvenance {
    LoaderDerived,
    ZeroInitializedByLoader,
    RelocationDerived,
    ExplicitTestInput,
    ReferenceSnapshot,
    RuntimeProvided,
    Unknown,
}
#[derive(Debug, Clone, Serialize)]
pub struct Requirement {
    pub reason: String,
    pub function: String,
    pub block: u32,
    pub pc: u32,
    pub operation: Option<ValueId>,
    pub register: Option<Register>,
    pub address: Option<u32>,
    pub width: Option<u8>,
    pub expression: String,
    pub provenance: InputProvenance,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionContextAnalysis {
    pub versions: ImageContextVersions,
    pub function: String,
    pub image_hash: String,
    pub state: String,
    pub compiled: bool,
    pub requirements: Vec<Requirement>,
    pub required_registers: BTreeSet<Register>,
    pub memory_accesses: usize,
    pub calls: usize,
    pub blockers: BTreeSet<String>,
    pub register_only_synthetic_eligible: bool,
    pub actual_game_execution_ready: bool,
    pub control_assumptions: Vec<String>,
}
#[derive(Clone, Debug)]
enum Expr {
    Constant(u32),
    Input(Register, i64),
    Unknown,
}
impl Expr {
    fn text(&self) -> String {
        match self {
            Self::Constant(n) => format!("0x{n:08X}"),
            Self::Input(r, o) => format!("entry.{r:?} {o:+}"),
            Self::Unknown => "UNKNOWN_ADDRESS_OR_ALIAS".into(),
        }
    }
}
fn effects(op: &Operation) -> (BTreeSet<Register>, BTreeSet<Register>) {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    match op {
        Operation::ReadRegister { register } => {
            reads.insert(*register);
        }
        Operation::WriteRegister { register, .. } => {
            writes.insert(*register);
        }
        Operation::BranchCondition {
            cr_bit,
            decrement_ctr,
            ..
        } => {
            if let Some(b) = cr_bit {
                reads.insert(Register::Cr(b / 4));
            }
            if *decrement_ctr {
                reads.insert(Register::Ctr);
            }
            if *decrement_ctr {
                writes.insert(Register::Ctr);
            }
        }
        Operation::Call { effects, .. }
        | Operation::UnknownInstruction { effects, .. }
        | Operation::UnknownSideEffect { effects, .. } => {
            reads.extend(&effects.uses); /* Unknown writes cannot prove an input defined. */
        }
        _ => {}
    }
    (reads, writes)
}
pub struct ExecutionContextBuilder<'a> {
    image: &'a CafeGuestImage,
}
impl<'a> ExecutionContextBuilder<'a> {
    pub fn new(image: &'a CafeGuestImage) -> Self {
        Self { image }
    }
    pub fn analyze(&self, f: &Function, compiled: bool) -> ExecutionContextAnalysis {
        let mut result = ExecutionContextAnalysis {
            versions: ImageContextVersions::CURRENT,
            function: f.id.clone(),
            image_hash: self.image.hash().into(),
            state: "CONTEXT_INCOMPLETE".into(),
            compiled,
            requirements: vec![],
            required_registers: BTreeSet::new(),
            memory_accesses: 0,
            calls: 0,
            blockers: BTreeSet::new(),
            register_only_synthetic_eligible: false,
            actual_game_execution_ready: false,
            control_assumptions: vec![
                "Function boundaries and explored CFG are inferred".into(),
                "Termination is not statically proven; step/call/depth limits are required".into(),
                "LR return target requires explicit outer-return harness or reference evidence"
                    .into(),
                "Linked image placement is not a captured Cafe process".into(),
            ],
        };
        if self
            .image
            .relocations()
            .iter()
            .any(|r| ["INVALID", "AMBIGUOUS"].contains(&r.state.as_str()))
        {
            result.blockers.insert("INVALID_IMAGE_RELOCATION".into());
        }
        if !woomy_ir::verify(f).structural_valid
            || f.module != self.image.module()
            || f.module_hash != self.image.module_hash()
        {
            result
                .blockers
                .insert("INVALID_IR_OR_IMAGE_IDENTITY".into());
            return result;
        }
        let universe: BTreeSet<_> = (0..32)
            .map(Register::Gpr)
            .chain((0..32).map(Register::Fpr))
            .chain((0..8).map(Register::Cr))
            .chain([
                Register::Lr,
                Register::Ctr,
                Register::Xer,
                Register::Fpscr,
                Register::Reservation,
            ])
            .collect();
        let mut predecessors: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for b in &f.blocks {
            for s in &b.successors {
                if let Destination::Block(target) = s.destination {
                    predecessors.entry(target).or_default().push(b.id);
                }
            }
        }
        let mut outs: BTreeMap<_, _> = f.blocks.iter().map(|b| (b.id, universe.clone())).collect();
        let entry_defs =
            |id: u32, outs: &BTreeMap<u32, BTreeSet<Register>>| -> BTreeSet<Register> {
                if id == f.entry {
                    return BTreeSet::new();
                }
                let Some(pred) = predecessors.get(&id) else {
                    return BTreeSet::new();
                };
                let mut defs = universe.clone();
                for p in pred {
                    defs = defs.intersection(&outs[p]).copied().collect();
                }
                defs
            };
        let mut converged = false;
        for _ in 0..=f.blocks.len().saturating_add(1).min(4097) {
            let mut changed = false;
            for b in &f.blocks {
                let mut defs = entry_defs(b.id, &outs);
                for op in &b.instructions {
                    let (_, writes) = effects(&op.operation);
                    defs.extend(writes);
                    if matches!(
                        op.operation,
                        Operation::Call { .. }
                            | Operation::UnknownInstruction { .. }
                            | Operation::UnknownSideEffect { .. }
                    ) {
                        defs.clear();
                    }
                }
                if outs[&b.id] != defs {
                    outs.insert(b.id, defs);
                    changed = true;
                }
            }
            if !changed {
                converged = true;
                break;
            }
        }
        if !converged {
            result
                .blockers
                .insert("ENTRY_REQUIREMENT_FIXED_POINT_LIMIT".into());
        }
        for b in &f.blocks {
            let mut defs = if converged {
                entry_defs(b.id, &outs)
            } else {
                BTreeSet::new()
            };
            let mut values: BTreeMap<ValueId, Expr> = BTreeMap::new();
            let mut regs: BTreeMap<Register, Expr> = BTreeMap::new();
            for op in &b.instructions {
                let pc = op.source.address;
                let make = |reason: &str,
                            register,
                            address,
                            width,
                            expression: String,
                            provenance,
                            evidence: &str| Requirement {
                    reason: reason.into(),
                    function: f.id.clone(),
                    block: b.id,
                    pc,
                    operation: Some(op.id),
                    register,
                    address,
                    width,
                    expression,
                    provenance,
                    evidence: evidence.into(),
                };
                let (reads, writes) = effects(&op.operation);
                for r in reads.difference(&defs) {
                    result.required_registers.insert(*r);
                    result.requirements.push(make(
                        "REGISTER_READ",
                        Some(*r),
                        None,
                        None,
                        format!("{r:?}: UNKNOWN_INPUT"),
                        InputProvenance::Unknown,
                        "Read before a definition on at least one known CFG path",
                    ));
                }
                let get = |id: &ValueId| values.get(id).cloned().unwrap_or(Expr::Unknown);
                let expr = match &op.operation {
                    Operation::Constant { bits } => Expr::Constant(*bits as u32),
                    Operation::ReadRegister { register } => {
                        regs.get(register).cloned().unwrap_or({
                            if b.id == f.entry {
                                Expr::Input(*register, 0)
                            } else {
                                Expr::Unknown
                            }
                        })
                    }
                    Operation::WriteRegister { register, value } => {
                        regs.insert(*register, get(value));
                        Expr::Unknown
                    }
                    Operation::Address32 { value } => get(value),
                    Operation::RelocationImmediate {
                        relocation,
                        signed,
                        shift,
                    } => {
                        result.requirements.push(make(
                            "RELOCATION_TARGET",
                            None,
                            relocation.local_address,
                            None,
                            relocation.symbol.clone(),
                            if relocation.local_address.is_some() {
                                InputProvenance::RelocationDerived
                            } else {
                                InputProvenance::Unknown
                            },
                            "IR relocation provenance; image must resolve this source range",
                        ));
                        relocation
                            .immediate(*signed, *shift)
                            .map(Expr::Constant)
                            .unwrap_or(Expr::Unknown)
                    }
                    Operation::Binary { operator, lhs, rhs } => {
                        match (operator, get(lhs), get(rhs)) {
                            (Binary::AddWrapping, Expr::Constant(a), Expr::Constant(b)) => {
                                Expr::Constant(a.wrapping_add(b))
                            }
                            (Binary::SubWrapping, Expr::Constant(a), Expr::Constant(b)) => {
                                Expr::Constant(a.wrapping_sub(b))
                            }
                            (Binary::Or, Expr::Constant(a), Expr::Constant(b)) => {
                                Expr::Constant(a | b)
                            }
                            (Binary::AddWrapping, Expr::Input(r, o), Expr::Constant(c))
                            | (Binary::AddWrapping, Expr::Constant(c), Expr::Input(r, o)) => {
                                Expr::Input(r, o + c as i32 as i64)
                            }
                            (Binary::SubWrapping, Expr::Input(r, o), Expr::Constant(c)) => {
                                Expr::Input(r, o - c as i32 as i64)
                            }
                            _ => Expr::Unknown,
                        }
                    }
                    Operation::Load { address, width, .. }
                    | Operation::Store { address, width, .. } => {
                        result.memory_accesses += 1;
                        let write = matches!(op.operation, Operation::Store { .. });
                        let e = get(address);
                        let mut provenance = InputProvenance::Unknown;
                        let class = match e {
                            Expr::Constant(a) => {
                                if let Some((i, _)) = self.image.memory().locate(
                                    a,
                                    *width as usize,
                                    if write { 2 } else { 1 },
                                ) {
                                    let r = &self.image.regions()[i];
                                    if !self.image.range_resolved(a, *width as usize) {
                                        "UNRESOLVED_RELOCATION"
                                    } else if r.permissions & 2 == 0 {
                                        provenance = InputProvenance::LoaderDerived;
                                        "LOADER_IMMUTABLE"
                                    } else if r.section_type == 8 {
                                        "LOADER_ZERO_INITIALIZED_BUT_PROCESS_VALUE_UNKNOWN"
                                    } else {
                                        "LOADER_MUTABLE_PROCESS_VALUE_UNKNOWN"
                                    }
                                } else {
                                    "UNKNOWN_OR_PERMISSION_CONFLICT"
                                }
                            }
                            Expr::Input(Register::Gpr(1), _) => "STACK_REQUIRES_CALLER_PROVENANCE",
                            Expr::Input(_, _) => "CALLER_PROVIDED_POINTER",
                            Expr::Unknown => "UNKNOWN_ADDRESS_OR_ALIAS",
                        };
                        result.requirements.push(make(if matches!(e,Expr::Input(Register::Gpr(1),_)){"STACK_ACCESS"}else if write {"GLOBAL_WRITE"}else{"GLOBAL_READ"},None,if let Expr::Constant(a)=e {Some(a)}else{None},Some(*width),format!("{}: {class}",e.text()),provenance,"Block-local symbolic IR expression; no object/type or allocation inferred"));
                        result.blockers.insert(format!("MEMORY: {class}"));
                        Expr::Unknown
                    }
                    Operation::Call { target, .. } => {
                        result.calls += 1;
                        result.requirements.push(make(
                            match target {
                                woomy_ir::CallTarget::Imported { .. } => "IMPORT_CALL",
                                woomy_ir::CallTarget::Static { .. } => "INTERNAL_CALL",
                                _ => "INDIRECT_CALL",
                            },
                            None,
                            None,
                            None,
                            format!("{target:?}"),
                            InputProvenance::Unknown,
                            "Call contract and closure required; no Cafe handler inferred",
                        ));
                        result
                            .blockers
                            .insert("CALL_CLOSURE_AND_RUNTIME_REQUIRED".into());
                        regs.clear();
                        Expr::Unknown
                    }
                    Operation::UnknownInstruction { .. }
                    | Operation::UnknownSideEffect { .. }
                    | Operation::UnknownValue { .. }
                    | Operation::OpaqueMemoryEffect { .. } => {
                        result.requirements.push(make(
                            "UNKNOWN_EFFECT",
                            None,
                            None,
                            None,
                            format!("{:?}", op.operation),
                            InputProvenance::Unknown,
                            "Unsupported IR is a blocker",
                        ));
                        result.blockers.insert("UNSUPPORTED_IR".into());
                        regs.clear();
                        Expr::Unknown
                    }
                    _ => Expr::Unknown,
                };
                if let Some(value) = &op.result {
                    values.insert(value.id, expr);
                }
                defs.extend(writes);
                if matches!(
                    op.operation,
                    Operation::Call { .. }
                        | Operation::UnknownInstruction { .. }
                        | Operation::UnknownSideEffect { .. }
                ) {
                    defs.clear();
                }
            }
            let control_source = match &b.terminator {
                Terminator::Return { source, .. }
                | Terminator::UnresolvedIndirectBranch { source, .. } => Some(*source),
                Terminator::ConditionalLrBranch { .. } => Some(Register::Lr),
                _ => None,
            };
            if let Some(source) = control_source.as_ref() {
                if !defs.contains(source) {
                    result.required_registers.insert(*source);
                    result.requirements.push(Requirement {
                        reason: "REGISTER_READ".into(),
                        function: f.id.clone(),
                        block: b.id,
                        pc: b.terminator_source.address,
                        operation: None,
                        register: Some(*source),
                        address: None,
                        width: None,
                        expression: format!("{source:?}: control target input"),
                        provenance: InputProvenance::Unknown,
                        evidence: "Terminator reads actual guest register".into(),
                    });
                }
            }
            if matches!(
                b.terminator,
                Terminator::UnresolvedIndirectBranch { .. }
                    | Terminator::ConditionalLrBranch { .. }
                    | Terminator::OpaqueTerminator { .. }
            ) || b
                .successors
                .iter()
                .any(|s| !matches!(s.destination, Destination::Block(_)))
            {
                result.blockers.insert("UNRESOLVED_CONTROL_FLOW".into());
            }
            for source in &b.sources {
                if !self.image.range_resolved(source.address, 4) {
                    result
                        .blockers
                        .insert("EXECUTABLE_SOURCE_RELOCATION_UNRESOLVED".into());
                }
            }
        }
        result.register_only_synthetic_eligible = result.memory_accesses == 0
            && result.calls == 0
            && result.blockers.is_empty()
            && woomy_codegen::eligibility(f).status == "ELIGIBLE";
        for r in &result.required_registers {
            result
                .blockers
                .insert(format!("REGISTER_INPUT_PROVENANCE: {r:?}"));
        }
        result
            .blockers
            .insert("NO_REFERENCE_PROCESS_CONTEXT".into());
        result
    }
}
