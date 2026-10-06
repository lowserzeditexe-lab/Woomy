//! Deterministic may-effect summaries. Preserved means never written in the
//! analyzed closure, not an assumed Cafe callee-save convention.
use crate::{runtime::*, *};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Certainty {
    Known,
    Partial,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    InternalFunction {
        module: String,
        address: u32,
        function: Option<String>,
    },
    ImportedFunction {
        library: String,
        symbol: String,
    },
    IndirectFunction {
        source: Register,
    },
    RuntimeFunction {
        provider: String,
        symbol: String,
    },
    Unresolved,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEffects {
    pub certainty: Certainty,
    pub registers_read: BTreeSet<Register>,
    pub registers_written: BTreeSet<Register>,
    pub registers_clobbered: BTreeSet<Register>,
    pub memory_read: bool,
    pub memory_write: bool,
    pub memory_unknown: bool,
    pub stack: String,
    pub return_behavior: String,
    pub may_trap: bool,
    pub provenance: String,
}
impl CallEffects {
    fn empty() -> Self {
        Self {
            certainty: Certainty::Known,
            registers_read: BTreeSet::new(),
            registers_written: BTreeSet::new(),
            registers_clobbered: BTreeSet::new(),
            memory_read: false,
            memory_write: false,
            memory_unknown: false,
            stack: "UNKNOWN".into(),
            return_behavior: "UNKNOWN (termination not proved)".into(),
            may_trap: false,
            provenance: "IR may-effects; conditional on inferred CFG/function coverage".into(),
        }
    }
    fn unknown(&mut self) {
        self.certainty = Certainty::Unknown;
        self.registers_clobbered = woomy_ppc::all_registers();
        self.memory_unknown = true;
        self.may_trap = true;
    }
    fn merge(&mut self, other: &Self) {
        self.registers_read.extend(&other.registers_read);
        self.registers_written.extend(&other.registers_written);
        self.registers_clobbered.extend(&other.registers_clobbered);
        self.memory_read |= other.memory_read;
        self.memory_write |= other.memory_write;
        self.memory_unknown |= other.memory_unknown;
        self.may_trap |= other.may_trap;
        if other.certainty != Certainty::Known {
            self.certainty = Certainty::Partial;
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    pub version: u32,
    pub caller: String,
    pub site: u32,
    pub target: Target,
    pub target_evidence: String,
    pub link_value: u32,
    pub link_behavior: String,
    pub effects: CallEffects,
    pub source: Provenance,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackObservation {
    pub site: u32,
    pub operation: String,
    pub entry_sp_offset: Option<i32>,
    pub width: Option<u8>,
    pub register: Option<Register>,
    pub provenance: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub version: u32,
    pub function: String,
    pub effects: CallEffects,
    pub registers_preserved: BTreeSet<Register>,
    pub direct_callees: BTreeSet<String>,
    pub unresolved_sites: BTreeSet<u32>,
    pub return_sites: BTreeSet<u32>,
    pub stack_observations: Vec<StackObservation>,
    pub fixed_point: bool,
    pub iterations: usize,
}
pub fn contracts(
    function: &Function,
    functions: &[Function],
    summaries: &BTreeMap<String, Summary>,
) -> Vec<Contract> {
    function.blocks.iter().flat_map(|b| &b.instructions).filter_map(|i| {
        let Operation::Call { target, link_value, .. } = &i.operation else { return None; };
        let mut effects = CallEffects::empty();
        let target = match target {
            CallTarget::Static { module,address } => {
                let callee = functions.iter().find(|f| f.module == *module && f.entry == *address);
                if let Some(summary) = callee.and_then(|f| summaries.get(&f.id)) { effects.merge(&summary.effects); }
                else { effects.unknown(); }
                Target::InternalFunction { module: module.clone(), address: *address, function: callee.map(|f| f.id.clone()) }
            }
            CallTarget::Imported { library,symbol } => { effects.unknown(); Target::ImportedFunction { library: library.clone(), symbol: symbol.clone() } }
            CallTarget::Indirect { source } => { effects.unknown(); Target::IndirectFunction { source: *source } }
            CallTarget::Unknown => { effects.unknown(); Target::Unresolved }
        };
        effects.registers_written.insert(Register::Lr);
        Some(Contract { version: CALL_MODEL_VERSION, caller: function.id.clone(), site: i.source.address, target,
            target_evidence: "Verified IR identity and source provenance; target identity does not prove effects".into(),
            link_value: *link_value, link_behavior: "Capture branch target before LR := CIA+4; return uses actual LR & ~3".into(), effects, source: i.source.clone() })
    }).collect()
}
pub fn summarize(functions: &[Function]) -> BTreeMap<String, Summary> {
    let mut local = BTreeMap::new();
    for f in functions {
        let mut summary = Summary {
            version: FUNCTION_SUMMARY_VERSION,
            function: f.id.clone(),
            effects: CallEffects::empty(),
            registers_preserved: BTreeSet::new(),
            direct_callees: BTreeSet::new(),
            unresolved_sites: BTreeSet::new(),
            return_sites: BTreeSet::new(),
            stack_observations: stack_observations(f),
            fixed_point: false,
            iterations: 0,
        };
        if !verify(f).structural_valid || f.limitations.iter().any(|s| s == "Source CFG truncated")
        {
            summary.effects.unknown();
        }
        for b in &f.blocks {
            for i in &b.instructions {
                match &i.operation {
                    Operation::ReadRegister { register } => {
                        summary.effects.registers_read.insert(*register);
                    }
                    Operation::WriteRegister { register, .. } => {
                        summary.effects.registers_written.insert(*register);
                    }
                    Operation::Load { .. } => {
                        summary.effects.memory_read = true;
                        summary.effects.may_trap = true;
                    }
                    Operation::Store { .. } => {
                        summary.effects.memory_write = true;
                        summary.effects.may_trap = true;
                    }
                    Operation::BranchCondition {
                        cr_bit,
                        decrement_ctr,
                        ..
                    } => {
                        if let Some(bit) = cr_bit {
                            summary.effects.registers_read.insert(Register::Cr(bit / 4));
                        }
                        if *decrement_ctr {
                            summary.effects.registers_read.insert(Register::Ctr);
                            summary.effects.registers_written.insert(Register::Ctr);
                        }
                    }
                    Operation::Call { target, .. } => {
                        summary.effects.registers_written.insert(Register::Lr);
                        if let CallTarget::Static { module, address } = target {
                            if let Some(callee) = functions
                                .iter()
                                .find(|f| f.module == *module && f.entry == *address)
                            {
                                summary.direct_callees.insert(callee.id.clone());
                                continue;
                            }
                        }
                        summary.unresolved_sites.insert(i.source.address);
                        summary.effects.unknown();
                    }
                    Operation::UnknownInstruction { .. }
                    | Operation::UnknownSideEffect { .. }
                    | Operation::UnknownValue { .. }
                    | Operation::OpaqueMemoryEffect { .. } => summary.effects.unknown(),
                    Operation::RelocationImmediate {
                        relocation:
                            Relocation {
                                local_address: None,
                                ..
                            },
                        ..
                    } => summary.effects.unknown(),
                    _ => {}
                }
            }
            match b.terminator {
                Terminator::Return { .. } => {
                    summary.effects.registers_read.insert(Register::Lr);
                    summary.return_sites.insert(b.terminator_source.address);
                }
                Terminator::ConditionalLrBranch { .. } => {
                    summary.effects.registers_read.insert(Register::Lr);
                    summary.effects.unknown();
                }
                Terminator::Trap => summary.effects.may_trap = true,
                Terminator::OpaqueTerminator { .. }
                | Terminator::UnresolvedIndirectBranch { .. } => summary.effects.unknown(),
                _ => {}
            }
            if b.successors
                .iter()
                .any(|s| !matches!(s.destination, Destination::Block(_)))
            {
                summary.effects.unknown();
            }
        }
        if summary.effects.certainty != Certainty::Known
            && (!summary.effects.registers_written.is_empty()
                || !summary.effects.registers_read.is_empty())
        {
            summary.effects.certainty = Certainty::Partial;
        }
        local.insert(f.id.clone(), summary);
    }
    let mut result = local.clone();
    // Monotonic finite unions; cycles require no speculative recursive expansion.
    for iteration in 1..=functions.len().saturating_add(1).min(4097) {
        let mut next = local.clone();
        for summary in next.values_mut() {
            for callee in &summary.direct_callees {
                if let Some(callee) = result.get(callee) {
                    summary.effects.merge(&callee.effects);
                    summary.unresolved_sites.extend(&callee.unresolved_sites);
                } else {
                    summary.effects.unknown();
                }
            }
        }
        let stable = next.iter().all(|(id, s)| {
            s.effects == result[id].effects && s.unresolved_sites == result[id].unresolved_sites
        });
        result = next;
        if stable {
            for s in result.values_mut() {
                s.fixed_point = true;
                s.iterations = iteration;
                s.registers_preserved = woomy_ppc::all_registers()
                    .difference(&s.effects.registers_written)
                    .copied()
                    .filter(|r| !s.effects.registers_clobbered.contains(r))
                    .collect();
            }
            return result;
        }
    }
    for s in result.values_mut() {
        s.effects.unknown();
        s.registers_preserved.clear();
    }
    result
}

/// Local affine observations, not a recovered high-level frame or variable list.
/// Entry r1 is a symbolic anchor. Other block entries/calls remain unknown.
fn stack_observations(f: &Function) -> Vec<StackObservation> {
    let mut observations = Vec::new();
    for b in &f.blocks {
        let mut sp = (b.id == f.entry).then_some(0i32);
        let mut offsets = BTreeMap::<ValueId, i32>::new();
        let mut constants = BTreeMap::<ValueId, u32>::new();
        let mut registers = BTreeMap::<ValueId, Register>::new();
        let mut origins = BTreeMap::<Register, Register>::new();
        let mut stack_values = BTreeSet::new();
        let mut loaded_values = BTreeMap::<ValueId, u32>::new();
        let mut loaded_registers = BTreeMap::<Register, u32>::new();
        for i in &b.instructions {
            match &i.operation {
                Operation::Constant { bits } => {
                    constants.insert(i.id, *bits as u32);
                }
                Operation::ReadRegister { register } => {
                    registers.insert(i.id, origins.get(register).copied().unwrap_or(*register));
                    if let Some(pc) = loaded_registers.get(register) {
                        loaded_values.insert(i.id, *pc);
                    }
                    if *register == Register::Gpr(1) {
                        stack_values.insert(i.id);
                        if let Some(n) = sp {
                            offsets.insert(i.id, n);
                        }
                    }
                }
                Operation::Binary {
                    operator: Binary::AddWrapping,
                    lhs,
                    rhs,
                } => {
                    if stack_values.contains(lhs) && constants.contains_key(rhs) {
                        stack_values.insert(i.id);
                    }
                    if let (Some(a), Some(b)) = (offsets.get(lhs), constants.get(rhs)) {
                        if let Some(n) = a.checked_add(*b as i32) {
                            offsets.insert(i.id, n);
                        }
                    }
                }
                Operation::Address32 { value } => {
                    if stack_values.contains(value) {
                        stack_values.insert(i.id);
                    }
                    if let Some(n) = offsets.get(value).copied() {
                        offsets.insert(i.id, n);
                    }
                }
                Operation::WriteRegister { register, value } => {
                    origins.remove(register);
                    loaded_registers.remove(register);
                    if let Some(origin) = registers.get(value) {
                        origins.insert(*register, *origin);
                    }
                    if let Some(pc) = loaded_values.get(value) {
                        loaded_registers.insert(*register, *pc);
                        observations.push(StackObservation {site:i.source.address,operation:"REGISTER_FROM_STACK".into(),entry_sp_offset:None,width:None,register:Some(*register),provenance:format!("IR value from stack load at {pc:08X}; restoration candidate, no equality with entry state asserted")});
                    }
                    if *register == Register::Gpr(1) {
                        sp = offsets.get(value).copied();
                        observations.push(StackObservation {site:i.source.address,operation:"SP_WRITE".into(),entry_sp_offset:sp,width:None,register:Some(*register),provenance:"IR affine observation; symbolic entry r1, no Cafe frame ABI assumed".into()});
                    }
                }
                Operation::Load { address, width, .. }
                | Operation::Store { address, width, .. }
                    if stack_values.contains(address) =>
                {
                    if matches!(i.operation, Operation::Load { .. }) {
                        loaded_values.insert(i.id, i.source.address);
                    }
                    observations.push(StackObservation {site:i.source.address,operation:if matches!(i.operation,Operation::Load {..}) {"STACK_LOAD"} else {"STACK_STORE"}.into(),entry_sp_offset:offsets.get(address).copied(),width:Some(*width),register:if let Operation::Store {value,..}=i.operation {registers.get(&value).copied()} else {None},provenance:"IR r1-relative access; entry offset UNKNOWN across blocks/calls, register origin only within this block".into()});
                }
                Operation::Call { .. } => {
                    sp = None;
                    origins.clear();
                    loaded_registers.clear();
                }
                _ => {}
            }
        }
    }
    observations
}
