//! Versioned, block-structured machine IR. Structural verification is not
//! semantic equivalence: unmodeled PPC is represented explicitly and preserved.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ppc::{Cfg, DecodedInstruction, Flow, Register, Semantics};

pub mod calls;
pub mod execution;
mod lift;
pub mod runtime;
mod verify;
pub use lift::{lift, lift_with_relocations};
pub use verify::verify;
pub const IR_VERSION: u32 = 4;
pub const MAX_OPERATIONS: usize = 262_144;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Type {
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Ptr,
    Unknown,
    Opaque,
}
pub fn register_type(r: Register) -> Type {
    match r {
        Register::Fpr(_) => Type::F64,
        Register::Reservation => Type::Opaque,
        _ => Type::I32,
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ValueId {
    pub address: u32,
    pub slot: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub id: ValueId,
    pub ty: Type,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Endian {
    Big,
    Little,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binary {
    AddWrapping,
    SubWrapping,
    And,
    Or,
    Xor,
    MultiplyLow,
    MultiplyHighSigned,
    MultiplyHighUnsigned,
    ShiftLeft,
    ShiftRight,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryEffect {
    None,
    Read,
    Write,
    ReadWrite,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effects {
    pub uses: BTreeSet<Register>,
    pub defs: BTreeSet<Register>,
    pub memory: MemoryEffect,
    pub details: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub module: String,
    pub address: u32,
    pub raw: u32,
    pub decoder_version: u32,
    pub analysis_version: u32,
    pub kind: String,
}
/// A proved target does not change the original machine transfer's trace kind.
pub fn static_call_event_kind(source: &Provenance) -> u32 {
    u32::from(matches!(
        woomy_ppc::decode(source.address, source.raw).control_flow,
        Flow::Indirect {
            link: true,
            conditional: false,
            ..
        }
    ))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub address: u32,
    pub raw: u32,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallTarget {
    Static { module: String, address: u32 },
    Imported { library: String, symbol: String },
    Indirect { source: Register },
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    /// Logical halfword relocation; the original PPC bytes are never patched.
    RelocationImmediate {
        relocation: Relocation,
        signed: bool,
        shift: u8,
    },
    /// Produces a CR field nibble (LT/GT/EQ/SO), not host condition flags.
    Compare32 {
        lhs: ValueId,
        rhs: ValueId,
        xer: ValueId,
        signed: bool,
    },
    /// Selective CA/OV/sticky SO updates, preserving all other XER bits.
    IntegerFlags32 {
        lhs: ValueId,
        rhs: ValueId,
        result: ValueId,
        xer: ValueId,
        subtract: bool,
        carry: bool,
        overflow: bool,
    },
    /// Tests one CR bit and optionally decrements CTR *before* testing it.
    BranchCondition {
        cr_bit: Option<u8>,
        cr_expected: bool,
        decrement_ctr: bool,
        ctr_expected_zero: bool,
    },
    Constant {
        bits: u64,
    },
    ReadRegister {
        register: Register,
    },
    WriteRegister {
        register: Register,
        value: ValueId,
    },
    Binary {
        operator: Binary,
        lhs: ValueId,
        rhs: ValueId,
    },
    RotateMask32 {
        value: ValueId,
        shift: u8,
        begin: u8,
        end: u8,
    },
    Address32 {
        value: ValueId,
    },
    Load {
        address: ValueId,
        width: u8,
        endian: Endian,
        may_trap: bool,
    },
    Store {
        address: ValueId,
        value: ValueId,
        width: u8,
        endian: Endian,
        may_trap: bool,
    },
    /// A call keeps complete guest state implicit. No argument/return ABI is assumed.
    Call {
        target: CallTarget,
        link_value: u32,
        effects: Effects,
        signature: Type,
    },
    UnknownInstruction {
        opcode: String,
        reason: String,
        effects: Effects,
    },
    UnknownValue {
        reason: String,
    },
    UnknownSideEffect {
        reason: String,
        effects: Effects,
    },
    OpaqueMemoryEffect {
        reason: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relocation {
    pub module: String,
    pub symbol: String,
    pub symbol_table: u32,
    pub symbol_index: u32,
    pub kind: u8,
    pub addend: i32,
    pub place: u32,
    pub target: woomy_ppc::dataflow::Target,
    pub local_address: Option<u32>,
}
impl Relocation {
    pub fn immediate(&self, signed: bool, shift: u8) -> Option<u32> {
        let address = self.local_address?;
        let field = match self.kind {
            4 => address as u16,
            5 => (address >> 16) as u16,
            6 => (address.wrapping_add(0x8000) >> 16) as u16,
            _ => return None,
        };
        let bits = if signed {
            field as i16 as i32 as u32
        } else {
            field as u32
        };
        bits.checked_shl(shift as u32)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instruction {
    pub id: ValueId,
    pub result: Option<Value>,
    pub operation: Operation,
    pub source: Provenance,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Destination {
    Block(u32),
    External(u32),
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Successor {
    pub destination: Destination,
    pub kind: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Terminator {
    /// Non-linking bclr: taken target is LR & !3; the other path falls through.
    ConditionalLrBranch {
        condition: ValueId,
        not_taken: Destination,
    },
    Branch {
        condition: ValueId,
        taken: Destination,
        not_taken: Destination,
    },
    Jump {
        target: Destination,
    },
    Return {
        source: Register,
        clear_low_bits: u8,
    },
    Trap,
    UnresolvedIndirectBranch {
        source: Register,
        clear_low_bits: u8,
    },
    OpaqueTerminator {
        reason: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub id: u32,
    pub sources: Vec<Source>,
    pub instructions: Vec<Instruction>,
    pub terminator: Terminator,
    pub terminator_source: Provenance,
    pub successors: Vec<Successor>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Function {
    pub id: String,
    pub module: String,
    pub module_hash: String,
    pub entry: u32,
    pub ir_version: u32,
    pub decoder_version: u32,
    pub cfg_version: u32,
    pub analysis_version: u32,
    pub status: String,
    pub blocks: Vec<Block>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub id: String,
    pub hash: String,
    pub ir_version: u32,
    pub functions: Vec<Function>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub block: Option<u32>,
    pub instruction: Option<ValueId>,
    pub message: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub status: String,
    pub ir_version: u32,
    pub structural_valid: bool,
    pub executable_semantics_validated: bool,
    pub unknown_operations: usize,
    pub diagnostics: Vec<Diagnostic>,
}

fn provenance(module: &str, d: &DecodedInstruction) -> Provenance {
    Provenance {
        module: module.into(),
        address: d.address,
        raw: d.raw,
        decoder_version: woomy_ppc::PPC_DECODER_VERSION,
        analysis_version: woomy_ppc::DATAFLOW_VERSION,
        kind: if d.status == "KNOWN" {
            "REAL"
        } else {
            "PARTIAL"
        }
        .into(),
    }
}
fn effects(d: &DecodedInstruction) -> Effects {
    match &d.semantics {
        Semantics::Known {
            uses,
            defs,
            memory_read,
            memory_write,
            side_effects,
        } => Effects {
            uses: uses.clone(),
            defs: defs.clone(),
            memory: match (memory_read, memory_write) {
                (false, false) => MemoryEffect::None,
                (true, false) => MemoryEffect::Read,
                (false, true) => MemoryEffect::Write,
                _ => MemoryEffect::ReadWrite,
            },
            details: side_effects.clone(),
        },
        Semantics::Unknown => unknown_effects(),
    }
}
fn unknown_effects() -> Effects {
    Effects {
        uses: woomy_ppc::all_registers(),
        defs: woomy_ppc::all_registers(),
        memory: MemoryEffect::ReadWrite,
        details: vec!["Unknown guest state and memory effects".into()],
    }
}
