//! Bounded semantic reference for the implemented scalar subset. Unsupported
//! effects stop explicitly; no platform calls, FPU or atomics are simulated.
use super::*;
use crate::runtime::{CallEvent, GuestMemory, Program, WoomyRuntime, READ, WRITE};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryWrite {
    pub address: u32,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    pub gpr: [u32; 32],
    pub fpr: [u64; 32],
    pub cr: [u8; 8],
    pub lr: u32,
    pub ctr: u32,
    pub xer: u32,
    pub fpscr: u32,
    pub memory_base: u32,
    pub memory: Vec<u8>,
    pub writes: Vec<MemoryWrite>,
    #[serde(default)]
    pub guest_memory: GuestMemory,
    #[serde(default)]
    pub calls: Vec<CallEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_context: Option<crate::runtime::RuntimeBinding>,
}
impl Machine {
    pub fn new(memory_base: u32, memory: Vec<u8>) -> Self {
        Self {
            gpr: [0; 32],
            fpr: [0; 32],
            cr: [0; 8],
            lr: 0,
            ctr: 0,
            xer: 0,
            fpscr: 0,
            memory_base,
            memory,
            writes: vec![],
            guest_memory: GuestMemory::default(),
            calls: vec![],
            runtime_context: None,
        }
    }
    fn read(&self, r: Register) -> Option<u32> {
        match r {
            Register::Gpr(n) => self.gpr.get(n as usize).copied(),
            Register::Lr => Some(self.lr),
            Register::Ctr => Some(self.ctr),
            Register::Xer => Some(self.xer),
            Register::Cr(n) => self.cr.get(n as usize).map(|v| *v as u32),
            _ => None,
        }
    }
    fn write(&mut self, r: Register, value: u32) -> bool {
        match r {
            Register::Gpr(n) if n < 32 => self.gpr[n as usize] = value,
            Register::Lr => self.lr = value,
            Register::Ctr => self.ctr = value,
            Register::Xer => self.xer = value,
            Register::Cr(n) if n < 8 => self.cr[n as usize] = (value & 15) as u8,
            _ => return false,
        }
        true
    }
    pub fn can_access(&self, address: u32, width: usize, permission: u32) -> bool {
        if !self.guest_memory.regions.is_empty() {
            return self
                .guest_memory
                .locate(address, width, permission)
                .is_some();
        }
        address.checked_sub(self.memory_base).is_some_and(|n| {
            (n as usize)
                .checked_add(width)
                .is_some_and(|end| end <= self.memory.len())
        })
    }
    pub fn load_width(&self, address: u32, width: u8) -> Option<u64> {
        if ![1, 2, 4, 8].contains(&width) || !self.can_access(address, width as usize, READ) {
            return None;
        }
        let bytes = if self.guest_memory.regions.is_empty() {
            let n = (address - self.memory_base) as usize;
            &self.memory[n..n + width as usize]
        } else {
            let (i, n) = self.guest_memory.locate(address, width as usize, READ)?;
            &self.guest_memory.regions[i].bytes[n..n + width as usize]
        };
        Some(bytes.iter().fold(0, |v, b| (v << 8) | *b as u64))
    }
    pub fn store_width(&mut self, address: u32, value: u64, width: u8) -> Option<()> {
        if ![1, 2, 4, 8].contains(&width)
            || !self.can_access(address, width as usize, WRITE)
            || self.writes.len() >= 131072
        {
            return None;
        }
        let bytes = value.to_be_bytes()[8 - width as usize..].to_vec();
        if self.guest_memory.regions.is_empty() {
            let n = (address - self.memory_base) as usize;
            self.memory[n..n + width as usize].copy_from_slice(&bytes);
        } else {
            let (i, n) = self.guest_memory.locate(address, width as usize, WRITE)?;
            self.guest_memory.regions[i].bytes[n..n + width as usize].copy_from_slice(&bytes);
        }
        self.writes.push(MemoryWrite { address, bytes });
        Some(())
    }
    fn bounded(&self) -> bool {
        self.guest_memory.valid()
            && self.calls.len() <= 65536
            && self.memory.len() <= 1024 * 1024
            && self.memory_base as u64 + self.memory.len() as u64 <= 0x1_0000_0000
            && self.writes.len() <= 65_536
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Exit {
    Returned { target: u32 },
    Trapped { address: u32 },
    Unsupported { address: u32, reason: String },
    Limit,
    InvalidIr,
    GuestFault { address: u32, reason: String },
    UnresolvedCall { address: u32 },
    UnresolvedImport { address: u32 },
    RuntimeFailure { address: u32, reason: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub state: Machine,
    pub trace: Vec<u32>,
    pub exit: Exit,
}
fn finish(state: Machine, trace: Vec<u32>, exit: Exit) -> Execution {
    Execution { state, trace, exit }
}
fn unsupported(state: Machine, trace: Vec<u32>, address: u32, reason: &str) -> Execution {
    finish(
        state,
        trace,
        Exit::Unsupported {
            address,
            reason: reason.into(),
        },
    )
}

/// Independent word-based evaluator: it does not invoke the lifter or evaluate
/// its operations. Tests compare full state, ordered memory writes and PPC trace.
pub fn execute_ppc(
    code: &BTreeMap<u32, u32>,
    entry: u32,
    state: Machine,
    budget: usize,
) -> Execution {
    execute_ppc_inner(code, entry, state, budget, None)
}
pub fn execute_ppc_program(
    program: &Program,
    code: &BTreeMap<u32, u32>,
    state: Machine,
    budget: usize,
) -> Execution {
    execute_ppc_program_with_provider(program, code, state, budget, program.provider.provider())
}
pub fn execute_ppc_program_with_provider(
    program: &Program,
    code: &BTreeMap<u32, u32>,
    state: Machine,
    budget: usize,
    provider: &dyn crate::runtime::RuntimeProvider,
) -> Execution {
    if program.validate().is_err() {
        return finish(state, vec![], Exit::InvalidIr);
    }
    if state.runtime_context != program.runtime_context
        || provider.binding() != program.runtime_context
        || (program.provider != crate::runtime::ProviderKind::ExternalRuntime
            && provider.query_capability() != program.provider.provider().query_capability())
    {
        return finish(
            state,
            vec![],
            Exit::RuntimeFailure {
                address: program.entry,
                reason: "RUNTIME_CONTEXT_OR_PROVIDER_MISMATCH".into(),
            },
        );
    }
    runtime_result(execute_ppc_inner(
        code,
        program.entry,
        state,
        budget,
        Some((program, provider)),
    ))
}
fn execute_ppc_inner(
    code: &BTreeMap<u32, u32>,
    entry: u32,
    mut state: Machine,
    budget: usize,
    runtime_input: Option<(&Program, &dyn crate::runtime::RuntimeProvider)>,
) -> Execution {
    let mut runtime =
        runtime_input.map(|(p, provider)| WoomyRuntime::with_provider(p, &state, provider));
    let mut trace = Vec::new();
    let mut pc = entry;
    if budget == 0 || budget > 65_536 || !state.bounded() {
        return finish(state, trace, Exit::Limit);
    }
    for _ in 0..budget {
        if let Some(rt) = &runtime {
            if let Err(exit) = rt.fetch(&state, pc) {
                return finish(state, trace, exit);
            }
        }
        let Some(&word) = code.get(&pc) else {
            return unsupported(state, trace, pc, "No source instruction");
        };
        if pc & 3 != 0 {
            return unsupported(state, trace, pc, "Unaligned instruction address");
        }
        trace.push(pc);
        let op = word >> 26;
        let t = ((word >> 21) & 31) as usize;
        let a = ((word >> 16) & 31) as usize;
        let b = ((word >> 11) & 31) as usize;
        let signed = word as u16 as i16 as i32 as u32;
        let xo = (word >> 1) & 1023;
        let next = pc.wrapping_add(4);
        match op {
            10 | 11 if word & 0x0060_0000 == 0 => {
                let a = state.gpr[a];
                let b = if op == 11 { signed } else { word & 0xffff };
                let order = if op == 11 {
                    (a as i32).cmp(&(b as i32))
                } else {
                    a.cmp(&b)
                };
                state.cr[((word >> 23) & 7) as usize] = match order {
                    std::cmp::Ordering::Less => 8,
                    std::cmp::Ordering::Greater => 4,
                    std::cmp::Ordering::Equal => 2,
                } | (state.xer >> 31) as u8;
            }
            31 if (xo == 0 || xo == 32) && word & 0x0060_0001 == 0 => {
                let a = state.gpr[a];
                let b = state.gpr[b];
                let order = if xo == 0 {
                    (a as i32).cmp(&(b as i32))
                } else {
                    a.cmp(&b)
                };
                state.cr[((word >> 23) & 7) as usize] = match order {
                    std::cmp::Ordering::Less => 8,
                    std::cmp::Ordering::Greater => 4,
                    std::cmp::Ordering::Equal => 2,
                } | (state.xer >> 31) as u8;
            }
            8 | 12 | 13 => {
                let base = state.gpr[a];
                let (v, carry) = if op == 8 {
                    (signed.wrapping_sub(base), signed >= base)
                } else {
                    let (v, c) = base.overflowing_add(signed);
                    (v, c)
                };
                state.gpr[t] = v;
                state.xer = (state.xer & !0x2000_0000) | ((carry as u32) << 29);
                if op == 13 {
                    state.cr[0] = if v == 0 {
                        2
                    } else if (v as i32) < 0 {
                        8
                    } else {
                        4
                    } | (state.xer >> 31) as u8;
                }
            }
            14 | 15 => {
                let base = if a == 0 { 0 } else { state.gpr[a] };
                let addend = if op == 15 {
                    signed.wrapping_mul(65536)
                } else {
                    signed
                };
                state.gpr[t] = base.wrapping_add(addend);
            }
            24..=27 => {
                let n = if op & 1 != 0 {
                    (word & 0xffff) * 65536
                } else {
                    word & 0xffff
                };
                state.gpr[a] = if op < 26 {
                    state.gpr[t] | n
                } else {
                    state.gpr[t] ^ n
                };
            }
            21 => {
                let shift = (word >> 11) & 31;
                let begin = (word >> 6) & 31;
                let end = (word >> 1) & 31;
                let mut mask = 0u32;
                for bit in 0..32 {
                    if (begin <= end && bit >= begin && bit <= end)
                        || (begin > end && (bit >= begin || bit <= end))
                    {
                        mask |= 0x8000_0000 >> bit;
                    }
                }
                state.gpr[a] = state.gpr[t].rotate_left(shift) & mask;
                if word & 1 != 0 {
                    let v = state.gpr[a];
                    state.cr[0] = if v == 0 {
                        2
                    } else if (v as i32) < 0 {
                        8
                    } else {
                        4
                    } | (state.xer >> 31) as u8;
                }
            }
            31 if [266, 40, 10, 8].contains(&(xo & 511)) => {
                let sub = [40, 8].contains(&(xo & 511));
                let x = state.gpr[a];
                let y = state.gpr[b];
                let (v, carry, wide) = if sub {
                    (
                        y.wrapping_sub(x),
                        y >= x,
                        (y as i32 as i64) - (x as i32 as i64),
                    )
                } else {
                    let (v, c) = x.overflowing_add(y);
                    (v, c, (x as i32 as i64) + (y as i32 as i64))
                };
                state.gpr[t] = v;
                if [10, 8].contains(&(xo & 511)) {
                    state.xer = (state.xer & !0x2000_0000) | ((carry as u32) << 29);
                }
                if word & 0x400 != 0 {
                    let ov = wide < i32::MIN as i64 || wide > i32::MAX as i64;
                    state.xer =
                        (state.xer & !0x4000_0000) | ((ov as u32) << 30) | ((ov as u32) << 31);
                }
                if word & 1 != 0 {
                    state.cr[0] = if v == 0 {
                        2
                    } else if (v as i32) < 0 {
                        8
                    } else {
                        4
                    } | (state.xer >> 31) as u8;
                }
            }
            31 if [444, 28, 316].contains(&xo) => {
                let v = match xo {
                    444 => state.gpr[t] | state.gpr[b],
                    28 => state.gpr[t] & state.gpr[b],
                    _ => state.gpr[t] ^ state.gpr[b],
                };
                state.gpr[a] = v;
                if word & 1 != 0 {
                    state.cr[0] = if v == 0 {
                        2
                    } else if (v as i32) < 0 {
                        8
                    } else {
                        4
                    } | (state.xer >> 31) as u8;
                }
            }
            31 if word & 1 == 0 && [339, 467].contains(&xo) => {
                let spr = ((word >> 16) & 31) | ((word >> 6) & 0x3e0);
                if xo == 339 {
                    state.gpr[t] = match spr {
                        8 => state.lr,
                        9 => state.ctr,
                        _ => return unsupported(state, trace, pc, "Unsupported SPR"),
                    };
                } else {
                    match spr {
                        8 => state.lr = state.gpr[t],
                        9 => state.ctr = state.gpr[t],
                        _ => return unsupported(state, trace, pc, "Unsupported SPR"),
                    }
                }
            }
            7 => state.gpr[t] = state.gpr[a].wrapping_mul(signed),
            31 if [235, 75, 11, 24, 536, 104].contains(&xo) => {
                let left = if xo == 24 || xo == 536 {
                    state.gpr[t]
                } else {
                    state.gpr[a]
                };
                let right = state.gpr[b];
                let v = match xo {
                    235 => left.wrapping_mul(right),
                    75 => (((left as i32 as i64) * (right as i32 as i64)) >> 32) as u32,
                    11 => ((left as u64 * right as u64) >> 32) as u32,
                    24 => {
                        if right & 32 != 0 {
                            0
                        } else {
                            left << (right & 31)
                        }
                    }
                    536 => {
                        if right & 32 != 0 {
                            0
                        } else {
                            left >> (right & 31)
                        }
                    }
                    _ => 0u32.wrapping_sub(left),
                };
                state.gpr[if xo == 24 || xo == 536 { a } else { t }] = v;
                if word & 1 != 0 {
                    state.cr[0] = (if v == 0 {
                        2
                    } else if v >> 31 != 0 {
                        8
                    } else {
                        4
                    }) | (state.xer >> 31) as u8;
                }
            }
            32..=41 | 44..=47 | 50 | 51 | 54 | 55 => {
                let floating = op >= 50;
                let multiple = op == 46 || op == 47;
                let load = matches!(op, 32..=35 | 40 | 41 | 46 | 50 | 51);
                let update = !multiple && op & 1 != 0;
                if update && (a == 0 || (load && !floating && a == t)) || (op == 46 && a >= t) {
                    return unsupported(state, trace, pc, "Invalid memory instruction");
                }
                let width = if floating {
                    8
                } else if [34, 35, 38, 39].contains(&op) {
                    1
                } else if [40, 41, 44, 45].contains(&op) {
                    2
                } else {
                    4
                };
                let address = if a == 0 { 0 } else { state.gpr[a] }.wrapping_add(signed);
                for n in t..(if multiple { 32 } else { t + 1 }) {
                    let at = address.wrapping_add((n - t) as u32 * 4);
                    if at % (width as u32).min(4) != 0 {
                        return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                    }
                    if load {
                        let Some(value) = state.load_width(at, width) else {
                            return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                        };
                        if floating {
                            state.fpr[n] = value;
                        } else {
                            state.gpr[n] = value as u32;
                        }
                    } else {
                        let value = if floating {
                            state.fpr[n]
                        } else {
                            state.gpr[n] as u64
                        };
                        if state.store_width(at, value, width).is_none() {
                            return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                        }
                    }
                }
                if update {
                    state.gpr[a] = address;
                }
            }
            31 if [23, 55, 87, 119, 151, 183, 215, 247, 279, 311, 407, 439].contains(&xo)
                && word & 1 == 0 =>
            {
                let load = [23, 55, 87, 119, 279, 311].contains(&xo);
                let update = [55, 119, 183, 247, 311, 439].contains(&xo);
                if update && (a == 0 || (load && a == t)) {
                    return unsupported(state, trace, pc, "Invalid memory instruction");
                }
                let width = if [87, 119, 215, 247].contains(&xo) {
                    1
                } else if [279, 311, 407, 439].contains(&xo) {
                    2
                } else {
                    4
                };
                let address = if a == 0 { 0 } else { state.gpr[a] }.wrapping_add(state.gpr[b]);
                if address % width as u32 != 0 {
                    return unsupported(
                        state,
                        trace,
                        pc,
                        "Unmapped or unaligned memory; guest exception semantics not implemented",
                    );
                }
                if load {
                    let Some(value) = state.load_width(address, width) else {
                        return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                    };
                    state.gpr[t] = value as u32;
                } else if state
                    .store_width(address, state.gpr[t] as u64, width)
                    .is_none()
                {
                    return unsupported(
                        state,
                        trace,
                        pc,
                        "Unmapped or unaligned memory; guest exception semantics not implemented",
                    );
                }
                if update {
                    state.gpr[a] = address;
                }
            }
            16 if word & 1 == 0 => {
                let bo = (word >> 21) & 31;
                let bi = (word >> 16) & 31;
                if bo & 4 == 0 {
                    state.ctr = state.ctr.wrapping_sub(1);
                }
                let ctr_ok = bo & 4 != 0 || ((state.ctr != 0) ^ (bo & 2 != 0));
                let cr_ok = bo & 16 != 0
                    || (((state.cr[(bi / 4) as usize] >> (3 - bi % 4)) & 1 != 0) == (bo & 8 != 0));
                if ctr_ok && cr_ok {
                    let displacement = (word as u16 & 0xfffc) as i16 as i32 as u32;
                    pc = if word & 2 != 0 {
                        displacement
                    } else {
                        pc.wrapping_add(displacement)
                    };
                } else {
                    pc = next;
                }
                continue;
            }
            18 => {
                let displacement = ((word & 0x03ff_fffc) << 6) as i32 >> 6;
                let target = if word & 2 != 0 {
                    displacement as u32
                } else {
                    pc.wrapping_add(displacement as u32)
                };
                if word & 1 != 0 {
                    if target != next
                        || runtime
                            .as_ref()
                            .is_some_and(|rt| rt.program.imports.contains_key(&pc))
                    {
                        if let Some(rt) = &mut runtime {
                            let kind = if rt.program.imports.contains_key(&pc) {
                                2
                            } else {
                                0
                            };
                            match rt.call(&mut state, pc, if kind == 2 { 0 } else { target }, kind)
                            {
                                Ok(target) => {
                                    pc = target;
                                    continue;
                                }
                                Err(exit) => return finish(state, trace, exit),
                            }
                        }
                        return unsupported(state, trace, pc, "Call effects unknown");
                    }
                    state.lr = next;
                }
                pc = target;
                continue;
            }
            19 if [16, 528].contains(&xo)
                && word & 1 != 0
                && word & 0xf800 == 0
                && ((word >> 21) & 20) == 20 =>
            {
                let target = if xo == 16 { state.lr } else { state.ctr } & !3;
                let Some(rt) = &mut runtime else {
                    return unsupported(state, trace, pc, "Call effects unknown");
                };
                match rt.call(&mut state, pc, target, 1) {
                    Ok(target) => {
                        pc = target;
                        continue;
                    }
                    Err(exit) => return finish(state, trace, exit),
                }
            }
            19 if xo == 16
                && word & 0xf801 == 0
                && woomy_ppc::decode(pc, word).status == "KNOWN" =>
            {
                let bo = (word >> 21) & 31;
                let bi = (word >> 16) & 31;
                if bo & 4 == 0 {
                    state.ctr = state.ctr.wrapping_sub(1);
                }
                let count_ok = bo & 4 != 0 || ((state.ctr != 0) ^ (bo & 2 != 0));
                let condition_ok = bo & 16 != 0
                    || (((state.cr[(bi / 4) as usize] >> (3 - bi % 4)) & 1 != 0) == (bo & 8 != 0));
                if !(count_ok && condition_ok) {
                    pc = next;
                    continue;
                }
                let target = state.lr & !3;
                if let Some(rt) = &mut runtime {
                    match rt.returning(&mut state, pc) {
                        Ok(Some(target)) => {
                            pc = target;
                            continue;
                        }
                        Ok(None) => return finish(state, trace, Exit::Returned { target }),
                        Err(exit) => return finish(state, trace, exit),
                    }
                }
                if code.contains_key(&target) {
                    pc = target;
                    continue;
                }
                return finish(state, trace, Exit::Returned { target });
            }
            3 if (word >> 21) & 31 == 31 => {
                return finish(state, trace, Exit::Trapped { address: pc })
            }
            31 if xo == 4 && (word >> 21) & 31 == 31 && word & 1 == 0 => {
                return finish(state, trace, Exit::Trapped { address: pc })
            }
            _ => return unsupported(state, trace, pc, "PPC semantics outside reference subset"),
        }
        pc = next;
    }
    finish(state, trace, Exit::Limit)
}

pub fn execute_ir(function: &Function, state: Machine, budget: usize) -> Execution {
    execute_ir_inner(function, state, budget, None)
}
pub fn execute_ir_program(program: &Program, state: Machine, budget: usize) -> Execution {
    execute_ir_program_with_provider(program, state, budget, program.provider.provider())
}
pub fn execute_ir_program_with_provider(
    program: &Program,
    state: Machine,
    budget: usize,
    provider: &dyn crate::runtime::RuntimeProvider,
) -> Execution {
    if program.validate().is_err() {
        return finish(state, vec![], Exit::InvalidIr);
    }
    if state.runtime_context != program.runtime_context
        || provider.binding() != program.runtime_context
        || (program.provider != crate::runtime::ProviderKind::ExternalRuntime
            && provider.query_capability() != program.provider.provider().query_capability())
    {
        return finish(
            state,
            vec![],
            Exit::RuntimeFailure {
                address: program.entry,
                reason: "RUNTIME_CONTEXT_OR_PROVIDER_MISMATCH".into(),
            },
        );
    }
    runtime_result(execute_ir_inner(
        program.function(program.entry).unwrap(),
        state,
        budget,
        Some((program, provider)),
    ))
}
fn execute_ir_inner(
    function: &Function,
    mut state: Machine,
    budget: usize,
    runtime_input: Option<(&Program, &dyn crate::runtime::RuntimeProvider)>,
) -> Execution {
    let program = runtime_input.map(|(p, _)| p);
    let mut runtime =
        runtime_input.map(|(p, provider)| WoomyRuntime::with_provider(p, &state, provider));
    let mut trace = Vec::new();
    if !verify(function).structural_valid {
        return finish(state, trace, Exit::InvalidIr);
    }
    if budget == 0 || budget > 65_536 || !state.bounded() {
        return finish(state, trace, Exit::Limit);
    }
    let blocks: BTreeMap<_, _> = if let Some(p) = program {
        p.functions
            .iter()
            .flat_map(|f| &f.blocks)
            .map(|b| (b.id, b))
            .collect()
    } else {
        function.blocks.iter().map(|b| (b.id, b)).collect()
    };
    let mut current = function.entry;
    'dispatch: loop {
        let Some(block) = blocks.get(&current) else {
            return unsupported(state, trace, current, "Missing block");
        };
        let mut values: BTreeMap<ValueId, u64> = BTreeMap::new();
        let mut last_pc = None;
        for instruction in &block.instructions {
            let pc = instruction.source.address;
            if last_pc != Some(pc) {
                if let Some(rt) = &runtime {
                    if let Err(exit) = rt.fetch(&state, pc) {
                        return finish(state, trace, exit);
                    }
                }
                if trace.len() >= budget {
                    return finish(state, trace, Exit::Limit);
                }
                trace.push(pc);
                last_pc = Some(pc);
            }
            let get = |id: &ValueId| values.get(id).copied();
            let result = match &instruction.operation {
                Operation::Call { target, .. } => {
                    let Some(rt) = &mut runtime else {
                        return unsupported(state, trace, pc, "Opaque or unimplemented IR effect");
                    };
                    let (target, kind) = match target {
                        CallTarget::Static { module, address }
                            if rt
                                .program
                                .function(*address)
                                .is_some_and(|f| &f.module == module) =>
                        {
                            (*address, crate::static_call_event_kind(&instruction.source))
                        }
                        CallTarget::Static { address, .. } => {
                            (*address, crate::static_call_event_kind(&instruction.source))
                        }
                        CallTarget::Imported { .. } => (0, 2),
                        CallTarget::Indirect {
                            source: Register::Lr,
                        } => (state.lr & !3, 1),
                        CallTarget::Indirect {
                            source: Register::Ctr,
                        } => (state.ctr & !3, 1),
                        _ => return unsupported(state, trace, pc, "UNRESOLVED_CALL"),
                    };
                    match rt.call(&mut state, pc, target, kind) {
                        Ok(target) => {
                            current = target;
                            continue 'dispatch;
                        }
                        Err(exit) => return finish(state, trace, exit),
                    }
                }
                Operation::RelocationImmediate {
                    relocation,
                    signed,
                    shift,
                } => {
                    let Some(bits) = relocation.immediate(*signed, *shift) else {
                        return unsupported(
                            state,
                            trace,
                            pc,
                            "Imported relocation has no runtime address",
                        );
                    };
                    Some(bits as u64)
                }
                Operation::Compare32 {
                    lhs,
                    rhs,
                    xer,
                    signed,
                } => {
                    let (Some(a), Some(b), Some(x)) = (get(lhs), get(rhs), get(xer)) else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    let flip = if *signed { 0x8000_0000 } else { 0 };
                    let a = (a as u32) ^ flip;
                    let b = (b as u32) ^ flip;
                    Some(
                        (if a < b {
                            8
                        } else if a > b {
                            4
                        } else {
                            2
                        }) | ((x >> 31) & 1),
                    )
                }
                Operation::IntegerFlags32 {
                    lhs,
                    rhs,
                    result,
                    xer,
                    subtract,
                    carry,
                    overflow,
                } => {
                    let (Some(a), Some(b), Some(v), Some(x)) =
                        (get(lhs), get(rhs), get(result), get(xer))
                    else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    let (a, b, v, mut x) = (a as u32, b as u32, v as u32, x as u32);
                    if *carry {
                        let ca = if *subtract {
                            a >= b
                        } else {
                            a as u64 + b as u64 > u32::MAX as u64
                        };
                        x = (x & !0x2000_0000) | ((ca as u32) << 29);
                    }
                    if *overflow {
                        let ov =
                            (if *subtract { a ^ b } else { !(a ^ b) }) & (a ^ v) & 0x8000_0000 != 0;
                        x = (x & !0x4000_0000) | ((ov as u32) << 30) | ((ov as u32) << 31);
                    }
                    Some(x as u64)
                }
                Operation::BranchCondition {
                    cr_bit,
                    cr_expected,
                    decrement_ctr,
                    ctr_expected_zero,
                } => {
                    if *decrement_ctr {
                        state.ctr = state.ctr.wrapping_sub(1);
                    }
                    let count = !*decrement_ctr || (state.ctr == 0) == *ctr_expected_zero;
                    let cond = cr_bit.is_none_or(|bit| {
                        ((state.cr[(bit / 4) as usize] >> (3 - bit % 4)) & 1 != 0) == *cr_expected
                    });
                    Some((count && cond) as u64)
                }
                Operation::Constant { bits } => Some(*bits),
                Operation::ReadRegister { register } => {
                    if let Register::Fpr(n) = register {
                        Some(state.fpr[*n as usize])
                    } else {
                        let Some(value) = state.read(*register) else {
                            return unsupported(
                                state,
                                trace,
                                pc,
                                "Register execution not implemented",
                            );
                        };
                        Some(value as u64)
                    }
                }
                Operation::WriteRegister { register, value } => {
                    let Some(v) = get(value) else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    if let Register::Fpr(n) = register {
                        state.fpr[*n as usize] = v;
                    } else if !state.write(*register, v as u32) {
                        return unsupported(state, trace, pc, "Register execution not implemented");
                    }
                    None
                }
                Operation::Binary { operator, lhs, rhs } => {
                    let (Some(a), Some(b)) = (get(lhs), get(rhs)) else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    let (a, b) = (a as u32, b as u32);
                    Some(match operator {
                        Binary::AddWrapping => a.wrapping_add(b),
                        Binary::SubWrapping => a.wrapping_sub(b),
                        Binary::And => a & b,
                        Binary::Or => a | b,
                        Binary::Xor => a ^ b,
                        Binary::MultiplyLow => (a as u64 * b as u64) as u32,
                        Binary::MultiplyHighUnsigned => ((a as u64 * b as u64) >> 32) as u32,
                        Binary::MultiplyHighSigned => {
                            ((a as i32 as i64 * b as i32 as i64) >> 32) as u32
                        }
                        Binary::ShiftLeft => a.checked_shl(b & 63).unwrap_or(0),
                        Binary::ShiftRight => a.checked_shr(b & 63).unwrap_or(0),
                    } as u64)
                }
                Operation::RotateMask32 {
                    value,
                    shift,
                    begin,
                    end,
                } => {
                    let Some(v) = get(value) else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    Some(
                        ((v as u32).rotate_left(*shift as u32)
                            & woomy_ppc::dataflow::mask32(*begin as u32, *end as u32))
                            as u64,
                    )
                }
                Operation::Address32 { value } => get(value),
                Operation::Load {
                    address,
                    width,
                    endian: Endian::Big,
                    ..
                } => {
                    let Some(address) = get(address) else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    if !(address as u32).is_multiple_of((*width as u32).min(4)) {
                        return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                    }
                    let Some(value) = state.load_width(address as u32, *width) else {
                        return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                    };
                    Some(value)
                }
                Operation::Store {
                    address,
                    value,
                    width,
                    endian: Endian::Big,
                    ..
                } => {
                    let (Some(address), Some(value)) = (get(address), get(value)) else {
                        return finish(state, trace, Exit::InvalidIr);
                    };
                    if !(address as u32).is_multiple_of((*width as u32).min(4))
                        || state.store_width(address as u32, value, *width).is_none()
                    {
                        return unsupported(state,trace,pc,"Unmapped or unaligned memory; guest exception semantics not implemented");
                    }
                    None
                }
                _ => return unsupported(state, trace, pc, "Opaque or unimplemented IR effect"),
            };
            if let Some(value) = &instruction.result {
                let Some(bits) = result else {
                    return finish(state, trace, Exit::InvalidIr);
                };
                values.insert(value.id, bits);
            }
        }
        let pc = block.terminator_source.address;
        if last_pc != Some(pc) {
            if let Some(rt) = &runtime {
                if let Err(exit) = rt.fetch(&state, pc) {
                    return finish(state, trace, exit);
                }
            }
            if trace.len() >= budget {
                return finish(state, trace, Exit::Limit);
            }
            trace.push(pc);
        }
        if let Terminator::ConditionalLrBranch {
            condition,
            not_taken,
        } = &block.terminator
        {
            let Some(condition) = values.get(condition) else {
                return finish(state, trace, Exit::InvalidIr);
            };
            if *condition == 0 {
                if let Destination::Block(target) = not_taken {
                    current = *target;
                    continue;
                }
                return unsupported(state, trace, pc, "Unknown bclr fallthrough");
            }
        }
        match &block.terminator {
            Terminator::Branch {
                condition,
                taken,
                not_taken,
            } => {
                let Some(condition) = values.get(condition) else {
                    return finish(state, trace, Exit::InvalidIr);
                };
                let destination = if *condition != 0 { taken } else { not_taken };
                if let Destination::Block(target) = destination {
                    current = *target;
                } else {
                    return unsupported(state, trace, pc, "Opaque or external control flow");
                }
            }
            Terminator::Jump {
                target: Destination::Block(target),
            } => current = *target,
            Terminator::Return {
                source: Register::Lr,
                clear_low_bits: 2,
            }
            | Terminator::ConditionalLrBranch { .. } => {
                let target = state.lr & !3;
                if let Some(rt) = &mut runtime {
                    match rt.returning(&mut state, pc) {
                        Ok(Some(target)) => {
                            current = target;
                            continue;
                        }
                        Ok(None) => return finish(state, trace, Exit::Returned { target }),
                        Err(exit) => return finish(state, trace, exit),
                    }
                }
                if blocks.contains_key(&target) {
                    current = target;
                    continue;
                }
                if function
                    .blocks
                    .iter()
                    .any(|b| b.sources.iter().any(|s| s.address == target))
                {
                    return unsupported(
                        state,
                        trace,
                        pc,
                        "LR targets an instruction without a proven block boundary",
                    );
                }
                return finish(state, trace, Exit::Returned { target });
            }
            Terminator::Trap => return finish(state, trace, Exit::Trapped { address: pc }),
            _ => return unsupported(state, trace, pc, "Opaque or external control flow"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialReport {
    pub status: String,
    pub state_equal: bool,
    pub trace_equal: bool,
    pub exit_equal: bool,
    pub ppc: Execution,
    pub ir: Execution,
}
pub fn compare(
    function: &Function,
    code: &BTreeMap<u32, u32>,
    initial: Machine,
    budget: usize,
) -> DifferentialReport {
    let ppc = execute_ppc(code, function.entry, initial.clone(), budget);
    let ir = execute_ir(function, initial, budget);
    let state_equal = ppc.state == ir.state;
    let trace_equal = ppc.trace == ir.trace;
    let exit_equal = ppc.exit == ir.exit;
    let completed = matches!(ppc.exit, Exit::Returned { .. } | Exit::Trapped { .. })
        && matches!(ir.exit, Exit::Returned { .. } | Exit::Trapped { .. });
    DifferentialReport {
        status: if !completed {
            "INCONCLUSIVE"
        } else if state_equal && trace_equal && exit_equal {
            "MATCH"
        } else {
            "MISMATCH"
        }
        .into(),
        state_equal,
        trace_equal,
        exit_equal,
        ppc,
        ir,
    }
}

fn runtime_result(mut execution: Execution) -> Execution {
    if let Exit::Unsupported { address, reason } = &execution.exit {
        execution.exit = match reason.as_str() {
            "UNRESOLVED_CALL" => Exit::UnresolvedCall { address: *address },
            "UNRESOLVED_IMPORT" => Exit::UnresolvedImport { address: *address },
            "RUNTIME_COPY_LIMIT" => Exit::RuntimeFailure {
                address: *address,
                reason: reason.clone(),
            },
            _ => Exit::GuestFault {
                address: *address,
                reason: reason.clone(),
            },
        };
    }
    execution
}
