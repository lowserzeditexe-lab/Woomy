//! Conservative subset of the 32-bit PowerPC ISA, CFG and register liveness.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub mod dataflow;

pub const PPC_DECODER_VERSION: u32 = 6;
pub const CFG_ANALYZER_VERSION: u32 = 4;
pub const DATAFLOW_VERSION: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Register {
    Gpr(u8),
    Fpr(u8),
    Cr(u8),
    Lr,
    Ctr,
    Xer,
    Fpscr,
    /// Opaque reservation state/address; asynchronous loss is never predicted.
    Reservation,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operand {
    IndexedMemory {
        base: Option<Register>,
        index: Register,
        width: u8,
    },
    ImportSymbol {
        library: String,
        name: String,
    },
    Register(Register),
    Immediate(u32),
    SignedImmediate(i32),
    Memory {
        base: Option<Register>,
        displacement: i32,
        width: u8,
    },
    BranchTarget(u32),
    ConditionRegister(u8),
    ConditionBit(u8),
    SpecialRegister(u16),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Semantics {
    Known {
        uses: BTreeSet<Register>,
        defs: BTreeSet<Register>,
        memory_read: bool,
        memory_write: bool,
        side_effects: Vec<String>,
    },
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flow {
    ExternalCall,
    Next,
    Branch { target: u32, conditional: bool },
    Call { target: u32, conditional: bool },
    Return { conditional: bool },
    Indirect { link: bool, conditional: bool },
    Trap,
    ConditionalTrap,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodedInstruction {
    pub address: u32,
    pub raw: u32,
    pub opcode: String,
    pub operands: Vec<Operand>,
    pub classification: String,
    pub control_flow: Flow,
    pub semantics: Semantics,
    pub status: String,
}
fn gpr(n: u32) -> Register {
    Register::Gpr((n & 31) as u8)
}
fn signed(value: u32, bits: u32) -> i32 {
    ((value << (32 - bits)) as i32) >> (32 - bits)
}
fn target(address: u32, displacement: i32, absolute: bool) -> u32 {
    if absolute {
        displacement as u32
    } else {
        address.wrapping_add(displacement as u32)
    }
}

pub fn decode(address: u32, raw: u32) -> DecodedInstruction {
    let mut d = DecodedInstruction {
        address,
        raw,
        opcode: ".long".into(),
        operands: vec![Operand::Immediate(raw)],
        classification: "Unknown".into(),
        control_flow: Flow::Unknown,
        semantics: Semantics::Unknown,
        status: "UNKNOWN".into(),
    };
    let op = raw >> 26;
    let rt = (raw >> 21) & 31;
    let ra = (raw >> 16) & 31;
    let rb = (raw >> 11) & 31;
    let imm = signed(raw & 0xffff, 16);
    let xo = (raw >> 1) & 1023;
    let mut uses = BTreeSet::new();
    let mut defs = BTreeSet::new();
    let mut mr = false;
    let mut mw = false;
    let mut operands = vec![];
    let mut effects = vec![];
    let mut flow = Flow::Next;
    let name: &str;
    let class: &str;
    match op {
        7 | 8 | 12 | 13 => {
            name = match op {
                7 => "mulli",
                8 => "subfic",
                12 => "addic",
                _ => "addic.",
            };
            class = "Integer";
            uses.insert(gpr(ra));
            defs.insert(gpr(rt));
            if op != 7 {
                uses.insert(Register::Xer);
                defs.insert(Register::Xer);
            }
            if op == 13 {
                defs.insert(Register::Cr(0));
            }
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::Register(gpr(ra)),
                Operand::SignedImmediate(imm),
            ]);
        }
        3 | 31 if op == 3 || (xo == 4 && raw & 1 == 0) => {
            name = if op == 3 { "twi" } else { "tw" };
            class = "ControlFlow";
            uses.insert(gpr(ra));
            operands.extend([Operand::Immediate(rt), Operand::Register(gpr(ra))]);
            if op == 3 {
                operands.push(Operand::SignedImmediate(imm));
            } else {
                uses.insert(gpr(rb));
                operands.push(Operand::Register(gpr(rb)));
            }
            flow = if rt == 0 {
                Flow::Next
            } else if rt & 28 == 28 || rt & 7 == 7 {
                Flow::Trap
            } else {
                Flow::ConditionalTrap
            };
            if rt != 0 {
                effects.push("MayTrap".into());
            }
        }
        14 | 15 => {
            name = if op == 14 { "addi" } else { "addis" };
            class = "Integer";
            if ra != 0 {
                uses.insert(gpr(ra));
            }
            defs.insert(gpr(rt));
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::Register(gpr(ra)),
                Operand::SignedImmediate(imm),
            ]);
        }
        24..=29 => {
            name = ["ori", "oris", "xori", "xoris", "andi.", "andis."][(op - 24) as usize];
            class = "Integer";
            uses.insert(gpr(rt));
            defs.insert(gpr(ra));
            if op >= 28 {
                defs.insert(Register::Cr(0));
                uses.insert(Register::Xer);
            }
            operands.extend([
                Operand::Register(gpr(ra)),
                Operand::Register(gpr(rt)),
                Operand::Immediate(raw & 0xffff),
            ]);
        }
        10 | 11 if raw & 0x00600000 == 0 => {
            name = if op == 10 { "cmpli" } else { "cmpi" };
            class = "Compare";
            let cr = ((raw >> 23) & 7) as u8;
            uses.extend([gpr(ra), Register::Xer]);
            defs.insert(Register::Cr(cr));
            operands.extend([
                Operand::ConditionRegister(cr),
                Operand::Register(gpr(ra)),
                if op == 10 {
                    Operand::Immediate(raw & 0xffff)
                } else {
                    Operand::SignedImmediate(imm)
                },
            ]);
        }
        46 | 47 => {
            if op == 46 && ra >= rt {
                d.status = "INVALID".into();
                return d;
            }
            name = if op == 46 { "lmw" } else { "stmw" };
            class = "Memory";
            if ra != 0 {
                uses.insert(gpr(ra));
            }
            for n in rt..32 {
                if op == 46 {
                    defs.insert(gpr(n));
                } else {
                    uses.insert(gpr(n));
                }
            }
            mr = op == 46;
            mw = op == 47;
            effects.push("MayTrap; multiple accesses; partial completion possible".into());
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::Memory {
                    base: if ra == 0 { None } else { Some(gpr(ra)) },
                    displacement: imm,
                    width: ((32 - rt) * 4) as u8,
                },
            ]);
        }
        31 if (xo == 0 || xo == 32) && raw & 0x00600001 == 0 => {
            name = if xo == 0 { "cmp" } else { "cmpl" };
            class = "Compare";
            let cr = ((raw >> 23) & 7) as u8;
            uses.extend([gpr(ra), gpr(rb), Register::Xer]);
            defs.insert(Register::Cr(cr));
            operands.extend([
                Operand::ConditionRegister(cr),
                Operand::Register(gpr(ra)),
                Operand::Register(gpr(rb)),
            ]);
        }
        31 if [
            23, 55, 87, 119, 151, 183, 215, 247, 279, 311, 343, 375, 407, 439,
        ]
        .contains(&xo)
            && raw & 1 == 0 =>
        {
            let (mnemonic, width, load, update) = match xo {
                23 => ("lwzx", 4, true, false),
                55 => ("lwzux", 4, true, true),
                87 => ("lbzx", 1, true, false),
                119 => ("lbzux", 1, true, true),
                151 => ("stwx", 4, false, false),
                183 => ("stwux", 4, false, true),
                215 => ("stbx", 1, false, false),
                247 => ("stbux", 1, false, true),
                279 => ("lhzx", 2, true, false),
                311 => ("lhzux", 2, true, true),
                343 => ("lhax", 2, true, false),
                375 => ("lhaux", 2, true, true),
                407 => ("sthx", 2, false, false),
                _ => ("sthux", 2, false, true),
            };
            if update && (ra == 0 || (load && ra == rt)) {
                d.status = "INVALID".into();
                return d;
            }
            name = mnemonic;
            class = "Memory";
            uses.insert(gpr(rb));
            if ra != 0 {
                uses.insert(gpr(ra));
            }
            if update {
                defs.insert(gpr(ra));
            }
            if load {
                defs.insert(gpr(rt));
                mr = true;
            } else {
                uses.insert(gpr(rt));
                mw = true;
            }
            effects.push("MayTrap".into());
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::IndexedMemory {
                    base: if ra == 0 { None } else { Some(gpr(ra)) },
                    index: gpr(rb),
                    width,
                },
            ]);
        }
        31 if xo == 20 || xo == 150 => {
            let load = xo == 20;
            if (raw & 1 != 0) == load {
                d.status = "INVALID".into();
                return d;
            }
            name = if load { "lwarx" } else { "stwcx." };
            class = "AtomicMemory";
            if ra != 0 {
                uses.insert(gpr(ra));
            }
            uses.insert(gpr(rb));
            defs.insert(Register::Reservation);
            if load {
                defs.insert(gpr(rt));
                mr = true;
                effects.push("EstablishReservation; reservation may be lost asynchronously".into());
            } else {
                uses.extend([gpr(rt), Register::Reservation, Register::Xer]);
                defs.insert(Register::Cr(0));
                mw = true;
                effects.push("ConditionalStore; clear reservation; CR0 reflects store and XER.SO; mismatched reservation address has undefined success".into());
            }
            effects.push("MayTrap; requires word alignment".into());
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::IndexedMemory {
                    base: if ra == 0 { None } else { Some(gpr(ra)) },
                    index: gpr(rb),
                    width: 4,
                },
            ]);
        }
        50 | 51 | 54 | 55 => {
            let load = op < 54;
            let update = op & 1 != 0;
            if update && ra == 0 {
                d.status = "INVALID".into();
                return d;
            }
            name = match op {
                50 => "lfd",
                51 => "lfdu",
                54 => "stfd",
                _ => "stfdu",
            };
            class = "FloatingMemory";
            if ra != 0 {
                uses.insert(gpr(ra));
            }
            if update {
                defs.insert(gpr(ra));
            }
            let fpr = Register::Fpr(rt as u8);
            if load {
                defs.insert(fpr);
                mr = true;
            } else {
                uses.insert(fpr);
                mw = true;
            }
            effects.push(
                "MayTrap; big-endian 64-bit transfer; no numeric conversion or FPSCR update".into(),
            );
            operands.extend([
                Operand::Register(fpr),
                Operand::Memory {
                    base: if ra == 0 { None } else { Some(gpr(ra)) },
                    displacement: imm,
                    width: 8,
                },
            ]);
        }
        32..=45 => {
            let (mnemonic, width, load, update) = match op {
                32 => ("lwz", 4, true, false),
                33 => ("lwzu", 4, true, true),
                34 => ("lbz", 1, true, false),
                35 => ("lbzu", 1, true, true),
                36 => ("stw", 4, false, false),
                37 => ("stwu", 4, false, true),
                38 => ("stb", 1, false, false),
                39 => ("stbu", 1, false, true),
                40 => ("lhz", 2, true, false),
                41 => ("lhzu", 2, true, true),
                42 => ("lha", 2, true, false),
                43 => ("lhau", 2, true, true),
                44 => ("sth", 2, false, false),
                _ => ("sthu", 2, false, true),
            };
            if update && (ra == 0 || (load && ra == rt)) {
                d.status = "INVALID".into();
                return d;
            }
            name = mnemonic;
            class = "Memory";
            if ra != 0 {
                uses.insert(gpr(ra));
            }
            if update {
                defs.insert(gpr(ra));
            }
            if load {
                defs.insert(gpr(rt));
                mr = true;
            } else {
                uses.insert(gpr(rt));
                mw = true;
            }
            effects.push("MayTrap".into());
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::Memory {
                    base: if ra == 0 { None } else { Some(gpr(ra)) },
                    displacement: imm,
                    width,
                },
            ]);
        }
        18 => {
            let t = target(address, signed(raw & 0x03fffffc, 26), raw & 2 != 0);
            let link = raw & 1 != 0;
            name = if link { "bl" } else { "b" };
            class = "ControlFlow";
            flow = if link {
                defs.insert(Register::Lr);
                Flow::Call {
                    target: t,
                    conditional: false,
                }
            } else {
                Flow::Branch {
                    target: t,
                    conditional: false,
                }
            };
            operands.push(Operand::BranchTarget(t));
        }
        16 | 19 if op == 16 || xo == 16 || xo == 528 => {
            if op == 19 && raw & 0x0000f800 != 0 {
                d.status = "INVALID".into();
                return d;
            }
            let bo = rt;
            let bi = ra;
            let link = raw & 1 != 0;
            let conditional = bo & 20 != 20;
            if bo & 16 == 0 {
                uses.insert(Register::Cr((bi / 4) as u8));
            }
            if bo & 4 == 0 {
                uses.insert(Register::Ctr);
                defs.insert(Register::Ctr);
            }
            if op == 19 && xo == 528 && bo & 4 == 0 {
                d.status = "INVALID".into();
                return d;
            }
            if link {
                defs.insert(Register::Lr);
            }
            operands.extend([Operand::Immediate(bo), Operand::Immediate(bi)]);
            class = "ControlFlow";
            if op == 16 {
                let t = target(address, signed(raw & 0xfffc, 16), raw & 2 != 0);
                name = if link { "bcl" } else { "bc" };
                operands.push(Operand::BranchTarget(t));
                flow = if link {
                    Flow::Call {
                        target: t,
                        conditional,
                    }
                } else {
                    Flow::Branch {
                        target: t,
                        conditional,
                    }
                };
            } else if xo == 16 {
                uses.insert(Register::Lr);
                name = if link {
                    "bclrl"
                } else if bo == 20 && bi == 0 {
                    "blr"
                } else {
                    "bclr"
                };
                // Only the canonical blr pattern is classified as a return.
                flow = if !link && bo == 20 && bi == 0 {
                    Flow::Return { conditional: false }
                } else {
                    Flow::Indirect { link, conditional }
                };
            } else {
                uses.insert(Register::Ctr);
                name = if link { "bctrl" } else { "bctr" };
                flow = Flow::Indirect { link, conditional };
            }
        }
        20 | 21 | 23 => {
            name = match op {
                20 => "rlwimi",
                21 => "rlwinm",
                _ => "rlwnm",
            };
            class = "Integer";
            uses.insert(gpr(rt));
            if op == 20 {
                uses.insert(gpr(ra));
            }
            if op == 23 {
                uses.insert(gpr(rb));
            }
            defs.insert(gpr(ra));
            if raw & 1 != 0 {
                defs.insert(Register::Cr(0));
                uses.insert(Register::Xer);
            }
            operands.extend([
                Operand::Register(gpr(ra)),
                Operand::Register(gpr(rt)),
                if op == 23 {
                    Operand::Register(gpr(rb))
                } else {
                    Operand::Immediate(rb)
                },
                Operand::Immediate((raw >> 6) & 31),
                Operand::Immediate((raw >> 1) & 31),
            ]);
        }
        31 if xo == 339 || xo == 467 => {
            let spr = (((raw >> 16) & 31) | ((raw >> 6) & 0x3e0)) as u16;
            let reg = match spr {
                1 => Register::Xer,
                8 => Register::Lr,
                9 => Register::Ctr,
                _ => return d,
            };
            name = if xo == 339 { "mfspr" } else { "mtspr" };
            class = "Register";
            if raw & 1 != 0 {
                d.status = "INVALID".into();
                return d;
            }
            if xo == 339 {
                uses.insert(reg);
                defs.insert(gpr(rt));
            } else {
                uses.insert(gpr(rt));
                defs.insert(reg);
            }
            operands.extend([Operand::Register(gpr(rt)), Operand::SpecialRegister(spr)]);
        }
        19 if [33, 129, 193, 225, 257, 289, 417, 449].contains(&xo) && raw & 1 == 0 => {
            name = match xo {
                33 => "crnor",
                129 => "crandc",
                193 => "crxor",
                225 => "crnand",
                257 => "crand",
                289 => "creqv",
                417 => "crorc",
                _ => "cror",
            };
            class = "Condition";
            // A bit update preserves the other three bits of its modeled CR field.
            uses.extend([
                Register::Cr((rt / 4) as u8),
                Register::Cr((ra / 4) as u8),
                Register::Cr((rb / 4) as u8),
            ]);
            defs.insert(Register::Cr((rt / 4) as u8));
            operands.extend([
                Operand::ConditionBit(rt as u8),
                Operand::ConditionBit(ra as u8),
                Operand::ConditionBit(rb as u8),
            ]);
        }
        31 if [
            8, 10, 40, 104, 136, 138, 200, 202, 232, 234, 235, 266, 459, 491,
        ]
        .contains(&(xo & 511)) =>
        {
            let base = xo & 511;
            let unary = [104, 200, 202, 232, 234].contains(&base);
            if unary && rb != 0 {
                d.status = "INVALID".into();
                return d;
            }
            name = match base {
                8 => "subfc",
                10 => "addc",
                40 => "subf",
                104 => "neg",
                136 => "subfe",
                138 => "adde",
                200 => "subfze",
                202 => "addze",
                232 => "subfme",
                234 => "addme",
                235 => "mullw",
                266 => "add",
                459 => "divwu",
                _ => "divw",
            };
            class = "Integer";
            uses.insert(gpr(ra));
            defs.insert(gpr(rt));
            operands.extend([Operand::Register(gpr(rt)), Operand::Register(gpr(ra))]);
            if !unary {
                uses.insert(gpr(rb));
                operands.push(Operand::Register(gpr(rb)));
            }
            let carry = [8, 10, 136, 138, 200, 202, 232, 234].contains(&base);
            if carry || raw & 0x400 != 0 {
                uses.insert(Register::Xer);
                defs.insert(Register::Xer);
            }
            if raw & 0x400 != 0 {
                effects.push("OverflowEnabled".into());
            }
            if raw & 1 != 0 {
                uses.insert(Register::Xer);
                defs.insert(Register::Cr(0));
            }
            if base == 459 || base == 491 {
                effects.push("UndefinedResultOnInvalidDivision".into());
            }
        }
        31 if xo == 11 || xo == 75 => {
            name = if xo == 11 { "mulhwu" } else { "mulhw" };
            class = "Integer";
            uses.extend([gpr(ra), gpr(rb)]);
            defs.insert(gpr(rt));
            operands.extend([
                Operand::Register(gpr(rt)),
                Operand::Register(gpr(ra)),
                Operand::Register(gpr(rb)),
            ]);
            if raw & 1 != 0 {
                uses.insert(Register::Xer);
                defs.insert(Register::Cr(0));
            }
        }
        31 if [24, 26, 536, 792, 824, 922, 954].contains(&xo) => {
            let unary = [26, 922, 954].contains(&xo);
            if unary && rb != 0 {
                d.status = "INVALID".into();
                return d;
            }
            name = match xo {
                24 => "slw",
                26 => "cntlzw",
                536 => "srw",
                792 => "sraw",
                824 => "srawi",
                922 => "extsh",
                _ => "extsb",
            };
            class = "Integer";
            uses.insert(gpr(rt));
            defs.insert(gpr(ra));
            operands.extend([Operand::Register(gpr(ra)), Operand::Register(gpr(rt))]);
            if !unary {
                if xo == 824 {
                    operands.push(Operand::Immediate(rb));
                } else {
                    uses.insert(gpr(rb));
                    operands.push(Operand::Register(gpr(rb)));
                }
            }
            if xo == 792 || xo == 824 {
                uses.insert(Register::Xer);
                defs.insert(Register::Xer);
            }
            if raw & 1 != 0 {
                uses.insert(Register::Xer);
                defs.insert(Register::Cr(0));
            }
        }
        31 if [28, 60, 124, 284, 316, 412, 444, 476].contains(&xo) => {
            class = "Integer";
            name = match xo {
                28 => "and",
                60 => "andc",
                124 => "nor",
                284 => "eqv",
                316 => "xor",
                412 => "orc",
                444 => "or",
                _ => "nand",
            };
            uses.extend([gpr(rt), gpr(rb)]);
            defs.insert(gpr(ra));
            operands.extend([
                Operand::Register(gpr(ra)),
                Operand::Register(gpr(rt)),
                Operand::Register(gpr(rb)),
            ]);
            if raw & 1 != 0 {
                uses.insert(Register::Xer);
                defs.insert(Register::Cr(0));
            }
        }
        _ => return d,
    }
    d.opcode = name.into();
    d.operands = operands;
    d.classification = class.into();
    d.control_flow = flow;
    d.semantics = Semantics::Known {
        uses,
        defs,
        memory_read: mr,
        memory_write: mw,
        side_effects: effects,
    };
    d.status = "KNOWN".into();
    d
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub from: u32,
    pub to: Option<u32>,
    pub kind: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub start: u32,
    pub end: u32,
    pub instructions: Vec<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CallSite {
    pub address: u32,
    pub target: Option<u32>,
    pub kind: String,
}
#[derive(Debug, Default, Clone, Serialize)]
pub struct Cfg {
    pub blocks: Vec<Block>,
    pub edges: Vec<Edge>,
    pub calls: Vec<CallSite>,
    pub truncated: bool,
}
fn successors(d: &DecodedInstruction) -> Vec<(Option<u32>, &'static str)> {
    let next = d.address.checked_add(4);
    match d.control_flow {
        Flow::Next | Flow::Call { .. } | Flow::ExternalCall => vec![(next, "Fallthrough")],
        Flow::Branch {
            target,
            conditional: false,
        } => vec![(Some(target), "Branch")],
        Flow::Branch {
            target,
            conditional: true,
        } => vec![
            (Some(target), "ConditionalTrue"),
            (next, "ConditionalFalse"),
        ],
        Flow::Return { conditional: false } | Flow::Trap => vec![],
        Flow::Return { conditional: true } => vec![(next, "ConditionalFalse")],
        Flow::Indirect { link, conditional } => {
            let mut v = vec![(None, "Indirect")];
            if link || conditional {
                v.push((next, "Fallthrough"));
            }
            v
        }
        Flow::Unknown => vec![(None, "Unknown")],
        Flow::ConditionalTrap => vec![(None, "Unknown"), (next, "ConditionalFalse")],
    }
}

/// Follows only proven successors; other function entries are boundaries.
pub fn build_cfg(
    entry: u32,
    code: &BTreeMap<u32, DecodedInstruction>,
    entries: &BTreeSet<u32>,
    limit: usize,
) -> Cfg {
    build_cfg_with_targets(entry, code, entries, limit, &BTreeMap::new())
}

/// Additional edges retain the decoded indirect instruction and its unknown exit.
/// The caller may remove that exit only after validating a final complete proof.
pub fn build_cfg_with_targets(
    entry: u32,
    code: &BTreeMap<u32, DecodedInstruction>,
    entries: &BTreeSet<u32>,
    limit: usize,
    indirect_targets: &BTreeMap<u32, BTreeSet<u32>>,
) -> Cfg {
    let successors = |d: &DecodedInstruction| {
        let mut edges = successors(d);
        if matches!(
            d.control_flow,
            Flow::Indirect { link: false, .. } | Flow::Return { .. }
        ) {
            if let Some(targets) = indirect_targets.get(&d.address) {
                edges.extend(targets.iter().map(|t| (Some(*t), "IndirectProven")));
            }
        }
        edges
    };
    let mut cfg = Cfg::default();
    let mut visited = BTreeSet::new();
    let mut pending = VecDeque::from([entry]);
    while let Some(pc) = pending.pop_front() {
        if visited.contains(&pc) || (pc != entry && entries.contains(&pc)) {
            continue;
        }
        if visited.len() >= limit {
            cfg.truncated = true;
            break;
        }
        let Some(d) = code.get(&pc) else {
            continue;
        };
        visited.insert(pc);
        for (t, _) in successors(d) {
            if let Some(t) = t {
                if !visited.contains(&t) {
                    pending.push_back(t);
                }
            }
        }
    }
    let mut leaders = BTreeSet::from([entry]);
    for pc in &visited {
        let d = &code[pc];
        if d.control_flow != Flow::Next {
            for (t, _) in successors(d) {
                if let Some(t) = t {
                    if visited.contains(&t) {
                        leaders.insert(t);
                    }
                }
            }
        }
    }
    let mut current: Option<Block> = None;
    for pc in &visited {
        if current.as_ref().is_some_and(|b| {
            leaders.contains(pc)
                || b.end.checked_add(4) != Some(*pc)
                || code[&b.end].control_flow != Flow::Next
        }) {
            if let Some(b) = current.take() {
                cfg.blocks.push(b);
            }
        }
        let b = current.get_or_insert_with(|| Block {
            start: *pc,
            end: *pc,
            instructions: vec![],
        });
        b.end = *pc;
        b.instructions.push(*pc);
        match code[pc].control_flow {
            Flow::ExternalCall => cfg.calls.push(CallSite {
                address: *pc,
                target: None,
                kind: "Import".into(),
            }),
            Flow::Call { target, .. } => cfg.calls.push(CallSite {
                address: *pc,
                target: Some(target),
                kind: "Direct".into(),
            }),
            Flow::Indirect { link: true, .. } => cfg.calls.push(CallSite {
                address: *pc,
                target: None,
                kind: "Indirect".into(),
            }),
            _ => {}
        }
    }
    if let Some(b) = current {
        cfg.blocks.push(b);
    }
    for b in &cfg.blocks {
        for (t, kind) in successors(&code[&b.end]) {
            cfg.edges.push(Edge {
                from: b.start,
                to: t,
                kind: if t.is_some_and(|t| !visited.contains(&t)) {
                    "Unknown".into()
                } else {
                    kind.into()
                },
            });
        }
    }
    cfg
}

#[derive(Debug, Serialize)]
pub struct LiveSet {
    pub block: u32,
    pub live_in: BTreeSet<Register>,
    pub live_out: BTreeSet<Register>,
}
pub fn all_registers() -> BTreeSet<Register> {
    (0..32)
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
        .collect()
}
/// Monotone finite lattice, deterministic worklist. Unknown exits use all registers.
pub fn liveness(cfg: &Cfg, code: &BTreeMap<u32, DecodedInstruction>) -> Vec<LiveSet> {
    let mut live: BTreeMap<u32, LiveSet> = cfg
        .blocks
        .iter()
        .map(|b| {
            (
                b.start,
                LiveSet {
                    block: b.start,
                    live_in: BTreeSet::new(),
                    live_out: BTreeSet::new(),
                },
            )
        })
        .collect();
    let blocks: BTreeMap<_, _> = cfg.blocks.iter().map(|b| (b.start, b)).collect();
    let mut work: BTreeSet<u32> = blocks.keys().copied().collect();
    let mut edges: BTreeMap<u32, Vec<&Edge>> = BTreeMap::new();
    let mut predecessors: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    for e in &cfg.edges {
        edges.entry(e.from).or_default().push(e);
        if let Some(t) = e.to {
            predecessors.entry(t).or_default().insert(e.from);
        }
    }
    while let Some(start) = work.pop_first() {
        let Some(b) = blocks.get(&start) else {
            continue;
        };
        let mut out = BTreeSet::new();
        if let Some(es) = edges.get(&start) {
            for e in es {
                match e.to.and_then(|t| live.get(&t)) {
                    Some(s) if e.kind != "Unknown" => out.extend(s.live_in.iter().copied()),
                    _ => out.extend(all_registers()),
                }
            }
        }
        // No ABI return convention has been proven: preserve observable register state.
        if edges.get(&start).is_none_or(|es| es.is_empty()) {
            out.extend(all_registers());
        }
        let mut input = out.clone();
        for pc in b.instructions.iter().rev() {
            let Some(d) = code.get(pc) else {
                input.extend(all_registers());
                continue;
            };
            if matches!(
                d.control_flow,
                Flow::Call { .. } | Flow::ExternalCall | Flow::Indirect { link: true, .. }
            ) {
                input.extend(all_registers());
                continue;
            }
            match &d.semantics {
                Semantics::Unknown => input.extend(all_registers()),
                Semantics::Known { uses, defs, .. } => {
                    input.retain(|r| !defs.contains(r));
                    input.extend(uses);
                }
            }
        }
        if let Some(s) = live.get_mut(&start) {
            if s.live_in != input || s.live_out != out {
                s.live_in = input;
                s.live_out = out;
                if let Some(ps) = predecessors.get(&start) {
                    work.extend(ps);
                }
            }
        }
    }
    live.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn branch_offsets_and_lr() {
        assert_eq!(
            decode(0x1000, 0x4bfffffc).control_flow,
            Flow::Branch {
                target: 0xffc,
                conditional: false
            }
        );
        assert_eq!(
            decode(0x1000, 0x48000007).control_flow,
            Flow::Call {
                target: 4,
                conditional: false
            }
        );
        assert_eq!(
            decode(0, 0x4e800020).control_flow,
            Flow::Return { conditional: false }
        );
        assert_eq!(
            decode(0, 0x4e800420).control_flow,
            Flow::Indirect {
                link: false,
                conditional: false
            }
        );
        assert_eq!(
            decode(0, 0x4e800421).control_flow,
            Flow::Indirect {
                link: true,
                conditional: false
            }
        );
        assert_eq!(
            decode(0x1000, 0x4182fffc).control_flow,
            Flow::Branch {
                target: 0xffc,
                conditional: true
            }
        );
    }
    #[test]
    fn unknown_is_barrier() {
        let d = decode(0, 0);
        assert!(matches!(d.semantics, Semantics::Unknown));
        assert_eq!(d.control_flow, Flow::Unknown);
    }
    #[test]
    fn carrying_immediate_and_traps_have_conservative_effects() {
        let d = decode(0x1000, 0x3529ffff);
        assert_eq!(d.opcode, "addic.");
        let Semantics::Known { uses, defs, .. } = d.semantics else {
            panic!("expected known semantics")
        };
        assert!(uses.contains(&Register::Gpr(9)));
        assert!(uses.contains(&Register::Xer));
        assert!(defs.contains(&Register::Xer));
        assert!(defs.contains(&Register::Cr(0)));
        assert_eq!(decode(0, 0x7ffe0008).control_flow, Flow::Trap);
        let conditional = (3u32 << 26) | (4 << 21) | (3 << 16);
        assert_eq!(decode(0, conditional).control_flow, Flow::ConditionalTrap);
        let code = BTreeMap::from([(0, decode(0, conditional)), (4, decode(4, 0x4e800020))]);
        let cfg = build_cfg(0, &code, &BTreeSet::from([0]), 10);
        assert_eq!(cfg.blocks.len(), 2);
        assert!(cfg.edges.iter().any(|e| e.kind == "Unknown"));
        assert!(cfg.edges.iter().any(|e| e.to == Some(4)));
    }
    #[test]
    fn multiple_and_indexed_memory_and_register_compare() {
        let load = decode(0, 0xbbc10008);
        assert_eq!(load.opcode, "lmw");
        let Semantics::Known {
            uses,
            defs,
            memory_read,
            ..
        } = load.semantics
        else {
            panic!("known load")
        };
        assert!(memory_read);
        assert!(uses.contains(&Register::Gpr(1)));
        assert_eq!(defs, BTreeSet::from([Register::Gpr(30), Register::Gpr(31)]));
        let store = decode(0, 0xbfc10008);
        assert_eq!(store.opcode, "stmw");
        assert_eq!(decode(0, 0x7d06002e).opcode, "lwzx");
        assert_eq!(decode(0, 0x7c1e0040).opcode, "cmpl");
        assert_eq!(
            decode(0, (46 << 26) | (30 << 21) | (31 << 16)).status,
            "INVALID"
        );
    }
    #[test]
    fn integer_carry_overflow_cr_bits_and_indexed_store() {
        let make =
            |xo: u32, rc: u32| (31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (xo << 1) | rc;
        for (xo, opcode) in [
            (8, "subfc"),
            (235, "mullw"),
            (11, "mulhwu"),
            (459, "divwu"),
            (491, "divw"),
            (24, "slw"),
            (824, "srawi"),
            (151, "stwx"),
        ] {
            assert_eq!(decode(0, make(xo, 0)).opcode, opcode);
        }
        let Semantics::Known { uses, defs, .. } = decode(0, make(522, 1)).semantics else {
            panic!("addco.")
        };
        assert!(uses.contains(&Register::Xer));
        assert!(defs.contains(&Register::Xer));
        assert!(defs.contains(&Register::Cr(0)));
        let Semantics::Known {
            uses,
            defs,
            memory_write,
            ..
        } = decode(0, make(151, 0)).semantics
        else {
            panic!("stwx")
        };
        assert!(memory_write);
        assert!(defs.is_empty());
        assert!(uses.contains(&Register::Gpr(3)));
        let crxor = (19 << 26) | (6 << 21) | (6 << 16) | (6 << 11) | (193 << 1);
        let d = decode(0, crxor);
        assert_eq!(d.opcode, "crxor");
        let Semantics::Known { uses, defs, .. } = d.semantics else {
            panic!("crxor")
        };
        assert!(uses.contains(&Register::Cr(1)));
        assert_eq!(defs, BTreeSet::from([Register::Cr(1)]));
        assert_eq!(
            decode(0x1000, 0x4bffffff).control_flow,
            Flow::Call {
                target: 0xfffffffc,
                conditional: false
            }
        );
    }
    #[test]
    fn cfg_if_else_loop_call_and_liveness() {
        let code: BTreeMap<_, _> = [0x4182000c, 0x38630001, 0x4bfffff8, 0x48000011, 0x4e800020]
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let a = 0x1000 + i as u32 * 4;
                (a, decode(a, *w))
            })
            .collect();
        let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
        assert_eq!(cfg.blocks.len(), 4);
        assert_eq!(cfg.calls[0].target, Some(0x101c));
        assert!(cfg.edges.iter().any(|e| e.to == Some(0x1000)));
        assert!(cfg.edges.iter().any(|e| e.kind == "ConditionalFalse"));
        let a = liveness(&cfg, &code);
        let b = liveness(&cfg, &code);
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert!(a[0].live_in.contains(&Register::Gpr(3)));
    }
    proptest! { #[test] fn every_word_is_safe(word in any::<u32>(), address in any::<u32>()) { let _=decode(address,word); } }
}
