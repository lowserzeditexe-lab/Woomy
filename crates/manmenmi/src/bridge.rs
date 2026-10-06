//! Bounded duplex adapter: C++ M9 owns runtime objects; Rust owns guest RAM.
use super::*;
use std::{
    io::{Read, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
pub const BRIDGE_VERSION: u32 = 2;
pub const ADAPTER_SOURCE: &str = include_str!("adapter.cpp");
const MAX_FRAME: usize = 65560;
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn word(bytes: &[u8], n: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        bytes
            .get(n..n + 4)
            .ok_or("M9_PROTOCOL_TRUNCATED")?
            .try_into()
            .map_err(|_| "M9_PROTOCOL_TRUNCATED")?,
    ))
}
pub struct Bridge {
    child: Child,
    input: Option<mpsc::SyncSender<Vec<u8>>>,
    output: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    workers: Vec<thread::JoinHandle<()>>,
    serial: u32,
    failed: bool,
    pub capabilities: Vec<(u32, u32)>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
impl Bridge {
    pub fn connect(
        executable: &Path,
        expected_hash: &str,
        process: &CafeProcessDescriptor,
    ) -> Result<Self> {
        process.validate()?;
        let metadata = std::fs::metadata(executable).map_err(|_| "M9_ADAPTER_UNAVAILABLE")?;
        if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
            return Err("M9_ADAPTER_SIZE");
        }
        let bytes = std::fs::read(executable).map_err(|_| "M9_ADAPTER_UNAVAILABLE")?;
        if format!("{:x}", Sha256::digest(bytes)) != expected_hash {
            return Err("M9_ADAPTER_HASH_MISMATCH");
        }
        let mut command = Command::new(executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn().map_err(|_| "M9_ADAPTER_START_FAILED")?;
        let mut stdin = child.stdin.take().ok_or("M9_TRANSPORT")?;
        let mut stdout = child.stdout.take().ok_or("M9_TRANSPORT")?;
        let (send_input, receive_input) = mpsc::sync_channel::<Vec<u8>>(2);
        let (send_output, receive_output) = mpsc::sync_channel(8);
        let writer = thread::spawn(move || {
            while let Ok(frame) = receive_input.recv() {
                if stdin
                    .write_all(&(frame.len() as u32).to_le_bytes())
                    .and_then(|_| stdin.write_all(&frame))
                    .and_then(|_| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let reader = thread::spawn(move || loop {
            let mut read = || -> std::io::Result<Vec<u8>> {
                let mut length = [0; 4];
                stdout.read_exact(&mut length)?;
                let n = u32::from_le_bytes(length) as usize;
                if !(4..=MAX_FRAME).contains(&n) {
                    return Err(std::io::Error::other("M9 frame limit"));
                }
                let mut frame = vec![0; n];
                stdout.read_exact(&mut frame)?;
                Ok(frame)
            };
            let result = (read)();
            let failed = result.is_err();
            if send_output.try_send(result).is_err() || failed {
                break;
            }
        });
        let mut bridge = Self {
            child,
            input: Some(send_input),
            output: receive_output,
            workers: vec![writer, reader],
            serial: 0,
            failed: false,
            capabilities: vec![],
        };
        let hello = bridge.receive(Instant::now() + Duration::from_secs(5))?;
        if hello != words(&[1, BRIDGE_VERSION, 1, 1, 1, 1]) {
            return Err("M9_VERSION_MISMATCH");
        }
        let current = process
            .current_thread
            .as_ref()
            .and_then(|k| {
                process
                    .threads
                    .iter()
                    .position(|t| t.address.value == k.value)
            })
            .map_or(u32::MAX, |i| i as u32);
        let debugger = process
            .debugger_initialized
            .as_ref()
            .map_or(0, |k| if k.value { 2 } else { 1 });
        let mut init = words(&[2, debugger, process.threads.len() as u32, current]);
        for t in &process.threads {
            init.extend(words(&[
                t.address.value.get(),
                match t.address.provenance {
                    ValueOrigin::RuntimeProvided => 0,
                    ValueOrigin::ExplicitTestInput => 1,
                    ValueOrigin::ReferenceCapture => 2,
                },
            ]));
        }
        bridge.send(init)?;
        let ready = bridge.receive(Instant::now() + Duration::from_secs(5))?;
        if ready.len() != 56 || word(&ready, 0)? != 2 || word(&ready, 4)? != 0 {
            return Err("M9_PROCESS_CREATION_FAILED");
        }
        for n in 0..6 {
            bridge
                .capabilities
                .push((word(&ready, 8 + n * 8)?, word(&ready, 12 + n * 8)?));
        }
        if bridge.capabilities != vec![(1, 1); 6] {
            return Err("M10_CAPABILITY_MISMATCH");
        }
        Ok(bridge)
    }
    fn send(&mut self, frame: Vec<u8>) -> Result<()> {
        if self.failed || frame.len() > MAX_FRAME {
            return Err("M9_TRANSPORT_FAILED");
        }
        self.input
            .as_ref()
            .ok_or("M9_TRANSPORT_FAILED")?
            .try_send(frame)
            .map_err(|_| "M9_TRANSPORT_FAILED")
    }
    fn receive(&mut self, deadline: Instant) -> Result<Vec<u8>> {
        match self
            .output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Ok(frame)) => Ok(frame),
            _ => {
                self.failed = true;
                let _ = self.child.kill();
                Err("M9_TRANSPORT_TIMEOUT_OR_FAULT")
            }
        }
    }
    /// Guest memory callbacks operate only on this authoritative Machine.
    /// The helper never receives a copy of the guest address space.
    pub fn invoke(&mut self, handler: u32, state: &mut Machine) -> Result<Option<u32>> {
        let result = self.invoke_inner(handler, state);
        if result.as_ref().is_err_and(|reason| {
            !matches!(
                *reason,
                "CURRENT_THREAD_UNAVAILABLE" | "DEBUGGER_STATE_UNAVAILABLE" | "M9_SERVICE_FAILED"
            )
        }) {
            self.failed = true;
            let _ = self.child.kill();
        }
        result
    }
    fn invoke_inner(&mut self, handler: u32, state: &mut Machine) -> Result<Option<u32>> {
        self.serial = self.serial.checked_add(1).ok_or("M9_CALL_LIMIT")?;
        if self.serial > 65536 || !(1..=6).contains(&handler) {
            return Err("M9_CALL_LIMIT_OR_HANDLER");
        }
        let mut request = words(&[3, self.serial, handler]);
        request.extend(words(&state.gpr[3..11]));
        self.send(request)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        for _ in 0..4096 {
            let frame = self.receive(deadline)?;
            match word(&frame, 0)? {
                3 => {
                    if frame.len() < 20 || word(&frame, 4)? != self.serial {
                        self.failed = true;
                        return Err("M9_PROTOCOL_RESULT");
                    }
                    if word(&frame, 8)? != 0 {
                        return Err(match handler {
                            1 => "CURRENT_THREAD_UNAVAILABLE",
                            2 => "DEBUGGER_STATE_UNAVAILABLE",
                            _ => "M9_SERVICE_FAILED",
                        });
                    }
                    if frame.len() != 20 || word(&frame, 12)? > 1 {
                        self.failed = true;
                        return Err("M9_PROTOCOL_RESULT");
                    }
                    return Ok((word(&frame, 12)? == 1).then_some(word(&frame, 16)?));
                }
                op @ (10..=12) => {
                    let response = memory_callback(op, &frame, state)?;
                    self.send(response)?;
                }
                _ => {
                    self.failed = true;
                    return Err("M9_PROTOCOL_OPERATION");
                }
            }
        }
        self.failed = true;
        let _ = self.child.kill();
        Err("M9_CALLBACK_LIMIT")
    }
}
fn memory_callback(op: u32, frame: &[u8], state: &mut Machine) -> Result<Vec<u8>> {
    let address = word(frame, 4)?;
    let size = word(frame, 8)? as usize;
    if size > 65536 || !state.guest_memory.valid() {
        return Ok(words(&[13, 1]));
    }
    let permission = if op == 10 {
        word(frame, 12)?
    } else if op == 11 {
        READ
    } else {
        WRITE
    };
    if permission == 0
        || permission & !7 != 0
        || frame.len()
            != match op {
                10 => 16,
                11 => 12,
                _ => 12 + size,
            }
    {
        return Err("M9_MEMORY_PROTOCOL");
    }
    let Some((i, n)) = state.guest_memory.locate(address, size, permission) else {
        return Ok(words(&[13, 1]));
    };
    let mut response = words(&[13, 0]);
    if op == 11 {
        response.extend_from_slice(&state.guest_memory.regions[i].bytes[n..n + size]);
    }
    if op == 12 {
        if state.writes.len().saturating_add(size) > 65536 {
            return Ok(words(&[13, 1]));
        }
        // Prevalidated complete range and write budget: no partial failed copy.
        for (offset, byte) in frame[12..].iter().enumerate() {
            state
                .store_width(address + offset as u32, *byte as u64, 1)
                .ok_or("M9_MEMORY_WRITE_INVARIANT")?;
        }
    }
    Ok(response)
}
