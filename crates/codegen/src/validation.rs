//! Native subprocess transport and complete three-way comparison. This module
//! is invoked with controlled synthetic inputs, never automatically for games.
use crate::compiler::{execute, ProcessRecord};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, path::Path, process::Command, time::Duration};
use woomy_ir::runtime::{CallEvent, Region};
use woomy_ir::{
    execution::{Execution, Exit, Machine, MemoryWrite},
    Function,
};
const MAGIC: u32 = 0x574f4f32;
fn num(out: &mut Vec<u8>, v: u64, n: usize) {
    out.extend_from_slice(&v.to_le_bytes()[..n]);
}
fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    num(out, bytes.len() as u64, 4);
    out.extend_from_slice(bytes);
}
fn put_state(out: &mut Vec<u8>, s: &Machine) {
    for r in s.gpr {
        num(out, r as u64, 4);
    }
    for r in s.fpr {
        num(out, r, 8);
    }
    out.extend_from_slice(&s.cr);
    for r in [s.lr, s.ctr, s.xer, s.fpscr, s.memory_base] {
        num(out, r as u64, 4);
    }
    blob(out, &s.memory);
    num(out, s.writes.len() as u64, 4);
    for w in &s.writes {
        num(out, w.address as u64, 4);
        blob(out, &w.bytes);
    }
    num(out, s.guest_memory.regions.len() as u64, 4);
    for r in &s.guest_memory.regions {
        for v in [r.base, r.permissions, r.kind] {
            num(out, v as u64, 4);
        }
        blob(out, r.provenance.as_bytes());
        blob(out, &r.bytes);
    }
    num(out, s.calls.len() as u64, 4);
    for e in &s.calls {
        for v in [e.caller, e.site, e.target, e.kind, e.return_target] {
            num(out, v as u64, 4);
        }
    }
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn num(&mut self, n: usize) -> io::Result<u64> {
        if self.0.len() < n {
            return Err(io::Error::other("truncated native result"));
        }
        let mut bytes = [0; 8];
        bytes[..n].copy_from_slice(&self.0[..n]);
        self.0 = &self.0[n..];
        Ok(u64::from_le_bytes(bytes))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(self.num(4)? as u32)
    }
    fn count(&mut self, max: usize) -> io::Result<usize> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(io::Error::other("native result exceeds bound"));
        }
        Ok(n)
    }
    fn blob(&mut self, max: usize) -> io::Result<Vec<u8>> {
        let n = self.count(max)?;
        if self.0.len() < n {
            return Err(io::Error::other("truncated native bytes"));
        }
        let v = self.0[..n].to_vec();
        self.0 = &self.0[n..];
        Ok(v)
    }
    fn state(&mut self) -> io::Result<Machine> {
        let mut s = Machine::new(0, vec![]);
        for r in &mut s.gpr {
            *r = self.u32()?;
        }
        for r in &mut s.fpr {
            *r = self.num(8)?;
        }
        for r in &mut s.cr {
            *r = self.num(1)? as u8;
        }
        s.lr = self.u32()?;
        s.ctr = self.u32()?;
        s.xer = self.u32()?;
        s.fpscr = self.u32()?;
        s.memory_base = self.u32()?;
        s.memory = self.blob(1024 * 1024)?;
        let n = self.count(131072)?;
        for _ in 0..n {
            s.writes.push(MemoryWrite {
                address: self.u32()?,
                bytes: self.blob(8)?,
            });
        }
        let n = self.count(256)?;
        for _ in 0..n {
            s.guest_memory.regions.push(Region {
                base: self.u32()?,
                permissions: self.u32()?,
                kind: self.u32()?,
                provenance: String::from_utf8(self.blob(4096)?).map_err(io::Error::other)?,
                bytes: self.blob(1048576)?,
            });
        }
        let n = self.count(65536)?;
        for _ in 0..n {
            s.calls.push(CallEvent {
                caller: self.u32()?,
                site: self.u32()?,
                target: self.u32()?,
                kind: self.u32()?,
                return_target: self.u32()?,
            });
        }
        Ok(s)
    }
    fn execution(&mut self) -> io::Result<Execution> {
        let state = self.state()?;
        let n = self.count(65536)?;
        let mut trace = Vec::with_capacity(n);
        for _ in 0..n {
            trace.push(self.u32()?);
        }
        let exit = self.u32()?;
        let address = self.u32()?;
        let reason = String::from_utf8(self.blob(4096)?).map_err(io::Error::other)?;
        let exit = match exit {
            0 => Exit::Returned { target: address },
            1 => Exit::Trapped { address },
            2 => Exit::Unsupported { address, reason },
            3 => Exit::Limit,
            4 => Exit::GuestFault { address, reason },
            5 => Exit::UnresolvedCall { address },
            6 => Exit::UnresolvedImport { address },
            7 => Exit::RuntimeFailure { address, reason },
            _ => return Err(io::Error::other("invalid native exit kind")),
        };
        Ok(Execution { state, trace, exit })
    }
}
pub struct NativeRun {
    pub process: ProcessRecord,
    pub executions: Vec<Execution>,
}
pub fn run(executable: &Path, cases: &[(Machine, usize)]) -> io::Result<NativeRun> {
    if cases.iter().any(|(s, _)| s.runtime_context.is_some()) {
        return Err(io::Error::other(
            "EXTERNAL_RUNTIME_NATIVE_TRANSPORT_UNAVAILABLE: context cannot be discarded",
        ));
    }
    run_transport(executable, cases, None)
}
pub(crate) fn run_transport(
    executable: &Path,
    cases: &[(Machine, usize)],
    identity: Option<&str>,
) -> io::Result<NativeRun> {
    if cases.len() > 4096 {
        return Err(io::Error::other("native case budget exceeded"));
    }
    let mut input = Vec::new();
    if let Some(id) = identity {
        input.extend_from_slice(id.as_bytes());
    }
    num(&mut input, MAGIC as u64, 4);
    num(&mut input, cases.len() as u64, 4);
    for (s, budget) in cases {
        if !s.guest_memory.valid()
            || s.calls.len() > 65536
            || s.memory.len() > 1024 * 1024
            || s.writes.len() > 65536
            || s.writes.iter().any(|w| w.bytes.len() > 8)
            || *budget > 65536
        {
            return Err(io::Error::other("native input budget exceeded"));
        }
        num(&mut input, *budget as u64, 4);
        put_state(&mut input, s);
        if input.len() > 64 * 1024 * 1024 {
            return Err(io::Error::other("native input exceeds 64 MiB"));
        }
    }
    let captured = execute(
        Command::new(executable),
        input,
        128 * 1024 * 1024,
        Duration::from_secs(30),
    )?;
    if captured.record.exit_code != Some(0)
        || captured.record.timed_out
        || captured.record.output_truncated
    {
        return Err(io::Error::other(format!(
            "NATIVE_FAULT: {:?}",
            captured.record
        )));
    }
    let mut reader = Reader(&captured.stdout);
    if let Some(id) = identity {
        if !reader.0.starts_with(id.as_bytes()) {
            return Err(io::Error::other("NATIVE_RUNTIME_IDENTITY_MISMATCH"));
        }
        reader.0 = &reader.0[id.len()..];
    }
    if reader.u32()? != MAGIC || reader.count(4096)? != cases.len() {
        return Err(io::Error::other("NATIVE_FAULT: protocol mismatch"));
    }
    let mut executions = Vec::new();
    for _ in cases {
        let mut execution = reader.execution()?;
        if identity.is_some() {
            execution.state.runtime_context = cases[executions.len()].0.runtime_context.clone();
        }
        executions.push(execution);
    }
    if !reader.0.is_empty() {
        return Err(io::Error::other("NATIVE_FAULT: trailing output"));
    }
    Ok(NativeRun {
        process: captured.record,
        executions,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub status: String,
    pub ppc_ir_equal: bool,
    pub ppc_native_equal: bool,
    pub ir_native_equal: bool,
    pub ppc: Execution,
    pub ir: Execution,
    pub native: Execution,
}
pub fn compare(
    f: &Function,
    code: &BTreeMap<u32, u32>,
    initial: Machine,
    budget: usize,
    native: Execution,
) -> Report {
    let pair = woomy_ir::execution::compare(f, code, initial, budget);
    let ppc_ir_equal = pair.ppc == pair.ir;
    let ppc_native_equal = pair.ppc == native;
    let ir_native_equal = pair.ir == native;
    let completed = |e: &Execution| matches!(e.exit, Exit::Returned { .. } | Exit::Trapped { .. });
    let status = if !completed(&pair.ppc) || !completed(&pair.ir) || !completed(&native) {
        "INCONCLUSIVE"
    } else if ppc_ir_equal && ppc_native_equal && ir_native_equal {
        "MATCH"
    } else {
        "MISMATCH"
    };
    Report {
        status: status.into(),
        ppc_ir_equal,
        ppc_native_equal,
        ir_native_equal,
        ppc: pair.ppc,
        ir: pair.ir,
        native,
    }
}

/// Full guest runtime comparison, including ordered writes and bounded call events.
pub fn compare_program(
    program: &woomy_ir::runtime::Program,
    code: &BTreeMap<u32, u32>,
    initial: Machine,
    budget: usize,
    native: Execution,
) -> Report {
    let ppc = woomy_ir::execution::execute_ppc_program(program, code, initial.clone(), budget);
    let ir = woomy_ir::execution::execute_ir_program(program, initial, budget);
    let ppc_ir_equal = ppc == ir;
    let ppc_native_equal = ppc == native;
    let ir_native_equal = ir == native;
    let completed = |e: &Execution| matches!(e.exit, Exit::Returned { .. } | Exit::Trapped { .. });
    // A divergent result is always a mismatch, including different faults/limits.
    let status = if !(ppc_ir_equal && ppc_native_equal && ir_native_equal) {
        "MISMATCH"
    } else if completed(&ppc) && completed(&ir) && completed(&native) {
        "MATCH"
    } else {
        "INCONCLUSIVE"
    };
    Report {
        status: status.into(),
        ppc_ir_equal,
        ppc_native_equal,
        ir_native_equal,
        ppc,
        ir,
        native,
    }
}
