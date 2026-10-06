//! Bounded forward may-analysis. Unreached blocks are bottom; Unknown is top.
//! Values are 32-bit bit patterns, not host pointers. No calling convention is assumed.
use crate::{all_registers, Cfg, DecodedInstruction, Flow, Register, Semantics};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_ALTERNATIVES: usize = 4;
pub const MAX_DEFINITIONS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Target {
    Address {
        module: String,
        section: u32,
        offset: u32,
    },
    ImportedSymbol {
        library: String,
        symbol: String,
        addend: i32,
        is_code: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Atom {
    Constant(u32),
    Target(Target),
    Upper { target: Target, adjusted: bool },
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    #[default]
    Unknown,
    Known(Atom),
    Conflicting(BTreeSet<Atom>),
}
impl Value {
    pub fn constant(n: u32) -> Self {
        Self::Known(Atom::Constant(n))
    }
    pub fn as_constant(&self) -> Option<u32> {
        match self {
            Self::Known(Atom::Constant(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn atoms(&self) -> Option<BTreeSet<Atom>> {
        match self {
            Self::Unknown => None,
            Self::Known(a) => Some(BTreeSet::from([a.clone()])),
            Self::Conflicting(a) => Some(a.clone()),
        }
    }
    fn from_atoms(atoms: BTreeSet<Atom>) -> Self {
        if atoms.is_empty() || atoms.len() > MAX_ALTERNATIVES {
            return Self::Unknown;
        }
        if atoms.len() == 1 {
            return atoms
                .into_iter()
                .next()
                .map(Self::Known)
                .unwrap_or_default();
        }
        Self::Conflicting(atoms)
    }
    pub fn join(&self, other: &Self) -> Self {
        let (Some(mut a), Some(b)) = (self.atoms(), other.atoms()) else {
            return Self::Unknown;
        };
        a.extend(b);
        Self::from_atoms(a)
    }
    pub fn map(&self, mut f: impl FnMut(&Atom) -> Option<Atom>) -> Self {
        let Some(atoms) = self.atoms() else {
            return Self::Unknown;
        };
        let mut out = BTreeSet::new();
        for a in atoms {
            let Some(b) = f(&a) else {
                return Self::Unknown;
            };
            out.insert(b);
        }
        Self::from_atoms(out)
    }
    pub fn binary(&self, other: &Self, mut f: impl FnMut(&Atom, &Atom) -> Option<Atom>) -> Self {
        let (Some(a), Some(b)) = (self.atoms(), other.atoms()) else {
            return Self::Unknown;
        };
        let mut out = BTreeSet::new();
        for x in &a {
            for y in &b {
                let Some(z) = f(x, y) else {
                    return Self::Unknown;
                };
                out.insert(z);
                if out.len() > MAX_ALTERNATIVES {
                    return Self::Unknown;
                }
            }
        }
        Self::from_atoms(out)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Definition {
    Entry,
    Instruction(u32),
    Call(u32),
    Unknown(u32),
    Many,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub value: Value,
    pub definitions: BTreeSet<Definition>,
}
impl Fact {
    pub fn entry() -> Self {
        Self {
            value: Value::Unknown,
            definitions: BTreeSet::from([Definition::Entry]),
        }
    }
    fn join(&self, other: &Self) -> Self {
        let mut definitions = self.definitions.clone();
        definitions.extend(&other.definitions);
        if definitions.len() > MAX_DEFINITIONS || definitions.contains(&Definition::Many) {
            definitions = BTreeSet::from([Definition::Many]);
        }
        Self {
            value: self.value.join(&other.value),
            definitions,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub registers: BTreeMap<Register, Fact>,
    pub memory_tainted: bool,
    /// Affine offsets from symbolic entry r1. Never concrete runtime addresses.
    pub stack_offsets: BTreeMap<Register, i32>,
    pub stack_slots: BTreeMap<i32, Fact>,
}
impl State {
    fn entry() -> Self {
        Self {
            registers: all_registers()
                .into_iter()
                .map(|r| (r, Fact::entry()))
                .collect(),
            memory_tainted: false,
            stack_offsets: BTreeMap::from([(Register::Gpr(1), 0)]),
            stack_slots: BTreeMap::new(),
        }
    }
    pub fn fact(&self, r: Register) -> Fact {
        self.registers.get(&r).cloned().unwrap_or_else(Fact::entry)
    }
    pub fn value(&self, r: Register) -> Value {
        self.fact(r).value
    }
    fn join(&self, other: &Self) -> Self {
        Self {
            registers: all_registers()
                .into_iter()
                .map(|r| (r, self.fact(r).join(&other.fact(r))))
                .collect(),
            memory_tainted: self.memory_tainted || other.memory_tainted,
            stack_offsets: self
                .stack_offsets
                .iter()
                .filter(|(r, n)| other.stack_offsets.get(r) == Some(n))
                .map(|(r, n)| (*r, *n))
                .collect(),
            stack_slots: self
                .stack_slots
                .iter()
                .filter_map(|(n, f)| other.stack_slots.get(n).map(|g| (*n, f.join(g))))
                .collect(),
        }
    }
}
/// The core layer may evaluate validated relocations and immutable memory.
/// Returning None invokes the scalar evaluator only for unrelocated instructions.
pub trait ValueModel {
    /// Only a supplied RAM/stack contract permits memory facts to resolve control.
    /// Real dump analysis leaves this false: r1 convention alone is insufficient.
    fn controlled_stack(&self) -> bool {
        false
    }
    fn evaluate(
        &self,
        _d: &DecodedInstruction,
        _input: &State,
    ) -> Option<BTreeMap<Register, Value>> {
        None
    }
}
impl ValueModel for () {}

#[derive(Debug, Clone, Serialize)]
pub struct RegisterFact {
    pub address: u32,
    pub register: Register,
    pub fact: Fact,
}
#[derive(Debug, Default)]
pub struct Dataflow {
    pub inputs: BTreeMap<u32, State>,
    pub outputs: BTreeMap<u32, State>,
    pub uses: Vec<RegisterFact>,
    pub writes: Vec<RegisterFact>,
    pub converged: bool,
    pub truncated: bool,
    pub steps: usize,
}
impl Dataflow {
    pub fn used(&self, pc: u32, r: Register) -> Option<&Fact> {
        self.uses
            .binary_search_by_key(&(pc, r), |f| (f.address, f.register))
            .ok()
            .map(|i| &self.uses[i].fact)
    }
}

/// No unstable intermediate facts escape on cancellation or a resource limit.
pub fn analyze(
    entry: u32,
    cfg: &Cfg,
    code: &BTreeMap<u32, DecodedInstruction>,
    model: &impl ValueModel,
    mut cancelled: impl FnMut() -> bool,
) -> Dataflow {
    let mut result = Dataflow::default();
    let blocks: BTreeMap<_, _> = cfg.blocks.iter().map(|b| (b.start, b)).collect();
    if blocks.len() > 4096 || !blocks.contains_key(&entry) || cfg.truncated {
        result.truncated = true;
        return result;
    }
    let mut incoming: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    let mut outgoing: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for e in &cfg.edges {
        if let Some(t) = e.to.filter(|t| blocks.contains_key(t)) {
            if e.kind != "Unknown" && blocks.contains_key(&e.from) {
                incoming.entry(t).or_default().insert(e.from);
                outgoing.entry(e.from).or_default().insert(t);
            }
        }
    }
    let mut work = BTreeSet::from([entry]);
    while let Some(start) = work.pop_first() {
        if cancelled() {
            return Dataflow {
                truncated: true,
                ..Dataflow::default()
            };
        }
        let mut input = (start == entry).then(State::entry);
        if let Some(ps) = incoming.get(&start) {
            for p in ps {
                if let Some(state) = result.outputs.get(p) {
                    input = Some(input.map_or_else(|| state.clone(), |s| s.join(state)));
                }
            }
        }
        let Some(input) = input else {
            continue;
        };
        if result.inputs.get(&start) == Some(&input) {
            continue;
        }
        let mut state = input.clone();
        for pc in &blocks[&start].instructions {
            result.steps += 1;
            if result.steps > 2_000_000 || cancelled() {
                return Dataflow {
                    truncated: true,
                    ..Dataflow::default()
                };
            }
            if let Some(d) = code.get(pc) {
                transfer(d, &mut state, model);
            } else {
                kill(&mut state, Definition::Unknown(*pc));
            }
        }
        result.inputs.insert(start, input);
        if result.outputs.get(&start) != Some(&state) {
            result.outputs.insert(start, state);
            if let Some(ts) = outgoing.get(&start) {
                work.extend(ts);
            }
        }
    }
    // Re-evaluate once using final block inputs, recording reaching uses and writes.
    for b in &cfg.blocks {
        let Some(mut state) = result.inputs.get(&b.start).cloned() else {
            continue;
        };
        for pc in &b.instructions {
            if cancelled() {
                return Dataflow {
                    truncated: true,
                    ..Dataflow::default()
                };
            }
            let Some(d) = code.get(pc) else {
                continue;
            };
            let uses = match &d.semantics {
                Semantics::Known { uses, .. } if !is_call(d) => uses.clone(),
                _ => all_registers(),
            };
            for r in uses {
                result.uses.push(RegisterFact {
                    address: *pc,
                    register: r,
                    fact: state.fact(r),
                });
            }
            let changed = transfer(d, &mut state, model);
            for r in changed {
                result.writes.push(RegisterFact {
                    address: *pc,
                    register: r,
                    fact: state.fact(r),
                });
            }
            if result.uses.len() + result.writes.len() > 262_144 {
                return Dataflow {
                    truncated: true,
                    ..Dataflow::default()
                };
            }
        }
    }
    result.uses.sort_by_key(|f| (f.address, f.register));
    result.writes.sort_by_key(|f| (f.address, f.register));
    result.converged = true;
    result
}
fn is_call(d: &DecodedInstruction) -> bool {
    match d.control_flow {
        // bl .+4 materializes PC; execution simply continues at the next instruction.
        Flow::Call {
            target,
            conditional: false,
        } if d.address.checked_add(4) == Some(target) => false,
        Flow::Call { .. } | Flow::ExternalCall | Flow::Indirect { link: true, .. } => true,
        _ => false,
    }
}
fn kill(state: &mut State, definition: Definition) -> BTreeSet<Register> {
    let regs = all_registers();
    state.registers = regs
        .iter()
        .map(|r| {
            (
                *r,
                Fact {
                    value: Value::Unknown,
                    definitions: BTreeSet::from([definition]),
                },
            )
        })
        .collect();
    state.memory_tainted = true;
    state.stack_offsets.clear();
    state.stack_slots.clear();
    regs
}
fn transfer(
    d: &DecodedInstruction,
    state: &mut State,
    model: &impl ValueModel,
) -> BTreeSet<Register> {
    if is_call(d) {
        return kill(state, Definition::Call(d.address));
    }
    let Semantics::Known {
        defs, memory_write, ..
    } = &d.semantics
    else {
        return kill(state, Definition::Unknown(d.address));
    };
    let mut computed = model.evaluate(d, state).unwrap_or_else(|| {
        if d.status == "KNOWN" {
            scalar(d, state)
        } else {
            BTreeMap::new()
        }
    });
    let input = state.clone();
    for r in defs {
        state.stack_offsets.remove(r);
    }
    if d.status == "KNOWN" {
        let t = Register::Gpr(((d.raw >> 21) & 31) as u8);
        let a = Register::Gpr(((d.raw >> 16) & 31) as u8);
        let b = Register::Gpr(((d.raw >> 11) & 31) as u8);
        let immediate = d.raw as u16 as i16 as i32;
        if d.opcode == "addi" && a != Register::Gpr(0) {
            if let Some(offset) = input
                .stack_offsets
                .get(&a)
                .and_then(|n| n.checked_add(immediate))
                .filter(|n| n.abs_diff(0) <= 1048576)
            {
                state.stack_offsets.insert(t, offset);
            }
        } else if d.opcode == "or" && t == b {
            if let Some(offset) = input.stack_offsets.get(&t) {
                state.stack_offsets.insert(a, *offset);
            }
        }
        let slot = input
            .stack_offsets
            .get(&a)
            .and_then(|n| n.checked_add(immediate))
            .filter(|n| n.abs_diff(0) <= 1048576);
        if ["lwz", "lwzu"].contains(&d.opcode.as_str()) {
            if let Some(value) = slot
                .and_then(|n| input.stack_slots.get(&n))
                .filter(|_| model.controlled_stack())
            {
                computed.insert(t, value.value.clone());
            }
            if d.opcode == "lwzu" {
                if let Some(n) = slot {
                    state.stack_offsets.insert(a, n);
                }
            }
        }
        if *memory_write {
            // Only exact aligned word slots are modeled. All other writes may alias.
            if ["stw", "stwu"].contains(&d.opcode.as_str()) && slot.is_some_and(|n| n % 4 == 0) {
                let n = slot.unwrap();
                let mut fact = input.fact(t);
                fact.definitions.insert(Definition::Instruction(d.address));
                state.stack_slots.insert(n, fact);
                if d.opcode == "stwu" {
                    state.stack_offsets.insert(a, n);
                }
                if state.stack_slots.len() > 1024 {
                    state.stack_slots.clear();
                }
            } else {
                state.stack_slots.clear();
            }
        }
    } else if *memory_write {
        state.stack_slots.clear();
    }
    for r in defs {
        state.registers.insert(
            *r,
            Fact {
                value: computed.get(r).cloned().unwrap_or_default(),
                definitions: BTreeSet::from([Definition::Instruction(d.address)]),
            },
        );
    }
    if model.controlled_stack()
        && d.status == "KNOWN"
        && ["lwz", "lwzu"].contains(&d.opcode.as_str())
    {
        let a = Register::Gpr(((d.raw >> 16) & 31) as u8);
        let t = Register::Gpr(((d.raw >> 21) & 31) as u8);
        if let Some(fact) = input
            .stack_offsets
            .get(&a)
            .and_then(|n| n.checked_add(d.raw as u16 as i16 as i32))
            .and_then(|n| input.stack_slots.get(&n))
        {
            if let Some(written) = state.registers.get_mut(&t) {
                written.definitions.extend(&fact.definitions);
                if written.definitions.len() > MAX_DEFINITIONS {
                    written.definitions = BTreeSet::from([Definition::Many]);
                }
            }
        }
    }
    state.memory_tainted |= *memory_write;
    defs.clone()
}
pub fn mask32(begin: u32, end: u32) -> u32 {
    let begin = begin & 31;
    let end = end & 31;
    (0..32)
        .filter(|bit| {
            if begin <= end {
                *bit >= begin && *bit <= end
            } else {
                *bit >= begin || *bit <= end
            }
        })
        .fold(0, |m, bit| m | (1 << (31 - bit)))
}
fn arithmetic(a: &Value, b: &Value, f: impl Fn(u32, u32) -> u32) -> Value {
    a.binary(b, |x, y| match (x, y) {
        (Atom::Constant(x), Atom::Constant(y)) => Some(Atom::Constant(f(*x, *y))),
        _ => None,
    })
}
pub fn add(a: &Value, b: &Value) -> Value {
    a.binary(b, |x, y| match (x, y) {
        (Atom::Constant(x), Atom::Constant(y)) => Some(Atom::Constant(x.wrapping_add(*y))),
        (
            Atom::Target(Target::Address {
                module,
                section,
                offset,
            }),
            Atom::Constant(n),
        )
        | (
            Atom::Constant(n),
            Atom::Target(Target::Address {
                module,
                section,
                offset,
            }),
        ) => Some(Atom::Target(Target::Address {
            module: module.clone(),
            section: *section,
            offset: offset.checked_add_signed(*n as i32)?,
        })),
        _ => None,
    })
}
fn scalar(d: &DecodedInstruction, s: &State) -> BTreeMap<Register, Value> {
    let raw = d.raw;
    let rt = Register::Gpr(((raw >> 21) & 31) as u8);
    let ra = Register::Gpr(((raw >> 16) & 31) as u8);
    let rb = Register::Gpr(((raw >> 11) & 31) as u8);
    let a = s.value(ra);
    let b = s.value(rb);
    let src = s.value(rt);
    let imm = raw as u16 as i16 as i32 as u32;
    let immediate = Value::constant(imm);
    let mut out = BTreeMap::new();
    let mut set = |r, v| {
        out.insert(r, v);
    };
    match d.opcode.as_str() {
        "addi" | "addis" => {
            let base = if (raw >> 16) & 31 == 0 {
                Value::constant(0)
            } else {
                a.clone()
            };
            set(
                rt,
                add(
                    &base,
                    &Value::constant(if d.opcode == "addis" { imm << 16 } else { imm }),
                ),
            );
        }
        "addic" | "addic." => set(rt, arithmetic(&a, &immediate, u32::wrapping_add)),
        "subfic" => set(rt, arithmetic(&a, &immediate, |a, b| b.wrapping_sub(a))),
        "mulli" => set(rt, arithmetic(&a, &immediate, u32::wrapping_mul)),
        "ori" | "oris" | "xori" | "xoris" | "andi." | "andis." => {
            let n = if d.opcode.contains('s') {
                (raw & 0xffff) << 16
            } else {
                raw & 0xffff
            };
            set(
                ra,
                arithmetic(&src, &Value::constant(n), |a, b| match d.opcode.as_str() {
                    "ori" | "oris" => a | b,
                    "xori" | "xoris" => a ^ b,
                    _ => a & b,
                }),
            );
        }
        "or" if rt == rb => set(ra, src.clone()),
        "or" | "and" | "andc" | "orc" | "xor" | "nor" | "nand" | "eqv" => set(
            ra,
            arithmetic(&src, &b, |x, y| match d.opcode.as_str() {
                "or" => x | y,
                "and" => x & y,
                "andc" => x & !y,
                "orc" => x | !y,
                "xor" => x ^ y,
                "nor" => !(x | y),
                "nand" => !(x & y),
                _ => !(x ^ y),
            }),
        ),
        "add" | "addc" => set(rt, add(&a, &b)),
        "subf" | "subfc" => set(rt, arithmetic(&a, &b, |a, b| b.wrapping_sub(a))),
        "neg" => set(rt, arithmetic(&Value::constant(0), &a, u32::wrapping_sub)),
        "mullw" => set(rt, arithmetic(&a, &b, u32::wrapping_mul)),
        "mulhwu" => set(
            rt,
            arithmetic(&a, &b, |a, b| ((a as u64 * b as u64) >> 32) as u32),
        ),
        "mulhw" => set(
            rt,
            arithmetic(&a, &b, |a, b| {
                (((a as i32 as i64) * (b as i32 as i64)) >> 32) as u32
            }),
        ),
        "rlwinm" | "rlwimi" | "rlwnm" => {
            let shift = if d.opcode == "rlwnm" {
                b.clone()
            } else {
                Value::constant((raw >> 11) & 31)
            };
            let mask = mask32(raw >> 6, raw >> 1);
            let rotated = arithmetic(&src, &shift, |v, n| v.rotate_left(n & 31) & mask);
            set(
                ra,
                if d.opcode == "rlwimi" {
                    arithmetic(&a, &rotated, |old, n| (old & !mask) | n)
                } else {
                    rotated
                },
            );
        }
        "slw" => set(
            ra,
            arithmetic(&src, &b, |v, n| if n & 32 != 0 { 0 } else { v << (n & 31) }),
        ),
        "srw" => set(
            ra,
            arithmetic(&src, &b, |v, n| if n & 32 != 0 { 0 } else { v >> (n & 31) }),
        ),
        "sraw" | "srawi" => {
            let n = if d.opcode == "srawi" {
                Value::constant((raw >> 11) & 31)
            } else {
                b.clone()
            };
            set(
                ra,
                arithmetic(&src, &n, |v, n| ((v as i32) >> (n & 63).min(31)) as u32),
            );
        }
        "cntlzw" => set(
            ra,
            src.map(|v| {
                if let Atom::Constant(n) = v {
                    Some(Atom::Constant(n.leading_zeros()))
                } else {
                    None
                }
            }),
        ),
        "extsh" | "extsb" => set(
            ra,
            src.map(|v| {
                if let Atom::Constant(n) = v {
                    Some(Atom::Constant(if d.opcode == "extsh" {
                        *n as i16 as i32 as u32
                    } else {
                        *n as i8 as i32 as u32
                    }))
                } else {
                    None
                }
            }),
        ),
        "mfspr" | "mtspr" => {
            let spr = ((raw >> 16) & 31) | ((raw >> 6) & 0x3e0);
            let reg = match spr {
                1 => Some(Register::Xer),
                8 => Some(Register::Lr),
                9 => Some(Register::Ctr),
                _ => None,
            };
            if let Some(reg) = reg {
                if d.opcode == "mfspr" {
                    set(rt, s.value(reg));
                } else {
                    set(reg, src);
                }
            }
        }
        _ => {}
    }
    if raw & 1 != 0 && matches!(d.control_flow, Flow::Call { .. } | Flow::Indirect { .. }) {
        out.insert(Register::Lr, Value::constant(d.address.wrapping_add(4)));
    }
    // Branches may decrement CTR. Never preserve its old value as a new fact.
    if matches!(
        d.control_flow,
        Flow::Branch { .. } | Flow::Indirect { .. } | Flow::Return { .. }
    ) && ((raw >> 26) == 16 || (raw >> 26) == 19)
        && raw & (4 << 21) == 0
    {
        out.insert(
            Register::Ctr,
            arithmetic(
                &s.value(Register::Ctr),
                &Value::constant(1),
                u32::wrapping_sub,
            ),
        );
    }
    out
}
