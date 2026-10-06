//! Instruction-level guest runtime. No Cafe parameter or callee-save ABI is assumed.
use crate::{
    execution::{Exit, Machine},
    *,
};
pub const CALL_MODEL_VERSION: u32 = 1;
pub const FUNCTION_SUMMARY_VERSION: u32 = 3;
pub const GUEST_MEMORY_VERSION: u32 = 1;
pub const RUNTIME_ABI_VERSION: u32 = 2;
pub const CAFE_IMAGE_VERSION: u32 = 1;
pub const CAFE_LOADER_VERSION: u32 = 1;
pub const RELOCATION_APPLIER_VERSION: u32 = 1;
pub const EXECUTION_CONTEXT_VERSION: u32 = 1;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageContextVersions {
    pub image: u32,
    pub loader: u32,
    pub relocation: u32,
    pub context: u32,
}
impl ImageContextVersions {
    pub const CURRENT: Self = Self {
        image: CAFE_IMAGE_VERSION,
        loader: CAFE_LOADER_VERSION,
        relocation: RELOCATION_APPLIER_VERSION,
        context: EXECUTION_CONTEXT_VERSION,
    };
}
pub const READ: u32 = 1;
pub const WRITE: u32 = 2;
pub const EXECUTE: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub base: u32,
    pub permissions: u32,
    /// 0 executable, 1 read-only data, 2 mutable data, 3 stack, 4 synthetic runtime.
    pub kind: u32,
    pub provenance: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestMemory {
    pub regions: Vec<Region>,
}
impl GuestMemory {
    pub fn valid(&self) -> bool {
        self.valid_with_limit(1048576)
    }
    /// Image storage can exceed the execution budget. Executors still use valid().
    pub fn valid_with_limit(&self, limit: usize) -> bool {
        let mut total = 0usize;
        self.regions.len() <= 256
            && self.regions.iter().enumerate().all(|(i, r)| {
                total = total.saturating_add(r.bytes.len());
                !r.bytes.is_empty()
                    && total <= limit.min(256 * 1024 * 1024)
                    && r.kind <= 4
                    && r.permissions & !7 == 0
                    && !r.provenance.is_empty()
                    && r.provenance.len() <= 4096
                    && r.base as u64 + r.bytes.len() as u64 <= 0x1_0000_0000
                    && self.regions[..i].iter().all(|other| {
                        r.base as u64 >= other.base as u64 + other.bytes.len() as u64
                            || other.base as u64 >= r.base as u64 + r.bytes.len() as u64
                    })
            })
    }
    pub fn locate(&self, address: u32, width: usize, permission: u32) -> Option<(usize, usize)> {
        let end = (address as u64).checked_add(width as u64)?;
        if width == 0 || end > 0x1_0000_0000 {
            return None;
        }
        self.regions.iter().enumerate().find_map(|(i, r)| {
            (r.permissions & permission == permission
                && address >= r.base
                && end <= r.base as u64 + r.bytes.len() as u64)
                .then_some((i, address.wrapping_sub(r.base) as usize))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEvent {
    pub caller: u32,
    pub site: u32,
    pub target: u32,
    /// 0 direct, 1 indirect, 2 synthetic import, 3 return.
    pub kind: u32,
    pub return_target: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub calls: usize,
    pub depth: usize,
    pub trace: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            calls: 4096,
            depth: 64,
            trace: 8192,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    NoRuntime,
    TestRuntime,
    ExternalRuntime,
}
/// Serializable guest identity only. Never contains a host address or object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBinding {
    pub provider: String,
    pub provider_abi: u32,
    pub semantics: u32,
    pub versions: BTreeMap<String, u32>,
    pub context_hash: String,
    pub process: String,
    pub module: String,
    pub module_hash: String,
    pub image_hash: String,
    pub domain: String,
    pub current_thread: Option<u32>,
}
impl RuntimeBinding {
    pub fn valid(&self) -> bool {
        self.provider_abi > 0
            && self.semantics > 0
            && self.versions.len() <= 32
            && self
                .versions
                .iter()
                .all(|(k, v)| !k.is_empty() && k.len() <= 128 && *v > 0)
            && [&self.provider, &self.process, &self.module, &self.domain]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 4096)
            && [&self.context_hash, &self.module_hash, &self.image_hash]
                .iter()
                .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            && self.current_thread != Some(0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub library: String,
    pub symbol: String,
}
pub trait RuntimeProvider {
    fn resolve_import(&self, import: &Import) -> Option<u32>;
    fn invoke_import(&self, handler: u32, state: &mut Machine) -> Result<(), &'static str>;
    fn query_capability(&self) -> &'static str;
    fn binding(&self) -> Option<RuntimeBinding> {
        None
    }
}
pub struct NoRuntime;
impl RuntimeProvider for NoRuntime {
    fn resolve_import(&self, _: &Import) -> Option<u32> {
        None
    }
    fn invoke_import(&self, _: u32, _: &mut Machine) -> Result<(), &'static str> {
        Err("UNRESOLVED_IMPORT")
    }
    fn query_capability(&self) -> &'static str {
        "NO_RUNTIME"
    }
}
pub struct TestRuntime;
impl RuntimeProvider for TestRuntime {
    fn resolve_import(&self, import: &Import) -> Option<u32> {
        if import.library != "woomy_test" {
            return None;
        }
        match import.symbol.as_str() {
            "test_add" => Some(1),
            "test_memory_copy" => Some(2),
            "test_return_constant" => Some(3),
            _ => None,
        }
    }
    fn invoke_import(&self, handler: u32, s: &mut Machine) -> Result<(), &'static str> {
        match handler {
            1 => s.gpr[3] = s.gpr[3].wrapping_add(s.gpr[4]),
            2 => {
                let (dst, src, count) = (s.gpr[3], s.gpr[4], s.gpr[5]);
                if count > 4096 {
                    return Err("RUNTIME_COPY_LIMIT");
                }
                // Test-only memmove contract: validate the whole transfer before writing.
                let mut bytes = Vec::new();
                for n in 0..count {
                    let a = src.checked_add(n).ok_or("GUEST_MEMORY_FAULT")?;
                    let d = dst.checked_add(n).ok_or("GUEST_MEMORY_FAULT")?;
                    if !s.can_access(d, 1, WRITE) {
                        return Err("GUEST_MEMORY_FAULT");
                    }
                    bytes.push(s.load_width(a, 1).ok_or("GUEST_MEMORY_FAULT")?);
                }
                for (n, byte) in bytes.into_iter().enumerate() {
                    s.store_width(dst + n as u32, byte, 1)
                        .ok_or("GUEST_MEMORY_FAULT")?;
                }
            }
            3 => s.gpr[3] = 0x574f4f4d,
            _ => return Err("UNRESOLVED_IMPORT"),
        }
        Ok(())
    }
    fn query_capability(&self) -> &'static str {
        "TEST_RUNTIME_ONLY"
    }
}
impl ProviderKind {
    pub fn provider(self) -> &'static dyn RuntimeProvider {
        match self {
            Self::NoRuntime => &NoRuntime,
            Self::TestRuntime => &TestRuntime,
            // An external program requires explicit injection and matching binding.
            Self::ExternalRuntime => &NoRuntime,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestImageIdentity {
    pub versions: ImageContextVersions,
    pub module: String,
    pub module_hash: String,
    pub image_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    #[serde(default)]
    pub image: Option<GuestImageIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_context: Option<RuntimeBinding>,
    /// Stop before fetching these source words; unresolved bytes are not values.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub source_barriers: BTreeMap<u32, String>,
    pub version: u32,
    pub entry: u32,
    pub functions: Vec<Function>,
    /// Explicit site bindings; no address is fabricated for an import.
    pub imports: BTreeMap<u32, Import>,
    pub provider: ProviderKind,
    pub limits: Limits,
}
impl Program {
    pub fn validate(&self) -> Result<(), String> {
        if (self.provider == ProviderKind::ExternalRuntime) != self.runtime_context.is_some()
            || self.runtime_context.as_ref().is_some_and(|b| {
                !b.valid()
                    || self.image.as_ref().is_none_or(|i| {
                        i.module != b.module
                            || i.module_hash != b.module_hash
                            || i.image_hash != b.image_hash
                    })
            })
        {
            return Err("RUNTIME_CONTEXT_IDENTITY_MISMATCH".into());
        }
        if let Some(image) = &self.image {
            if image.versions != ImageContextVersions::CURRENT
                || image.image_hash.len() != 64
                || !image.image_hash.bytes().all(|b| b.is_ascii_hexdigit())
                || self
                    .functions
                    .iter()
                    .any(|f| f.module != image.module || f.module_hash != image.module_hash)
            {
                return Err("IMAGE_CONTEXT_IDENTITY_MISMATCH".into());
            }
        }
        if self.version != RUNTIME_ABI_VERSION
            || self.functions.is_empty()
            || self.functions.len() > 4096
            || self.limits.calls > 65536
            || self.limits.depth > 1024
            || self.limits.trace > 65536
        {
            return Err("RUNTIME_VERSION_OR_LIMIT".into());
        }
        let mut sources = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut module_hashes = BTreeMap::new();
        let mut operations = 0usize;
        for f in &self.functions {
            if module_hashes
                .insert(&f.module, &f.module_hash)
                .is_some_and(|hash| hash != &f.module_hash)
            {
                return Err("MODULE_HASH_MISMATCH".into());
            }
            operations = operations
                .saturating_add(f.blocks.iter().map(|b| b.instructions.len()).sum::<usize>());
            if operations > MAX_OPERATIONS {
                return Err("PROGRAM_OPERATION_LIMIT".into());
            }
            if !verify(f).structural_valid || !identities.insert((&f.module, &f.id)) {
                return Err("INVALID_FUNCTION_REGISTRATION".into());
            }
            for b in &f.blocks {
                for s in &b.sources {
                    if !sources.insert(s.address) {
                        return Err("OVERLAPPING_FUNCTION_REGISTRATION".into());
                    }
                }
            }
        }
        if sources.len() > 65536 || self.imports.len() > 65536 {
            return Err("PROGRAM_SOURCE_LIMIT".into());
        }
        if self.source_barriers.len() > 65536
            || self
                .source_barriers
                .iter()
                .any(|(pc, why)| !sources.contains(pc) || why.is_empty() || why.len() > 4096)
        {
            return Err("INVALID_SOURCE_BARRIER".into());
        }
        for f in &self.functions {
            for i in f.blocks.iter().flat_map(|b| &b.instructions) {
                if let Operation::Call {
                    target: CallTarget::Static { module, address },
                    ..
                } = &i.operation
                {
                    if self
                        .function(*address)
                        .is_some_and(|callee| &callee.module != module)
                    {
                        return Err("CALL_MODULE_IDENTITY_MISMATCH".into());
                    }
                }
            }
        }
        if self.function(self.entry).is_none() {
            return Err("ENTRY_NOT_REGISTERED".into());
        }
        for (pc, import) in &self.imports {
            if !self.functions.iter().flat_map(|f| &f.blocks).flat_map(|b| &b.instructions).any(|i| {
                i.source.address == *pc && matches!(&i.operation, Operation::Call { target: CallTarget::Imported { library, symbol }, .. } if *library == import.library && *symbol == import.symbol)
            }) { return Err("IMPORT_SITE_CONTRACT_MISMATCH".into()); }
        }
        Ok(())
    }
    pub fn function(&self, entry: u32) -> Option<&Function> {
        self.functions.iter().find(|f| f.entry == entry)
    }
    pub fn owner(&self, pc: u32) -> Option<u32> {
        self.functions
            .iter()
            .find(|f| {
                f.blocks
                    .iter()
                    .any(|b| b.sources.iter().any(|s| s.address == pc))
            })
            .map(|f| f.entry)
    }
    pub fn block(&self, pc: u32) -> Option<&Block> {
        self.functions
            .iter()
            .flat_map(|f| &f.blocks)
            .find(|b| b.id == pc)
    }
}

/// Bookkeeping never restores LR or any register. Guest instructions own state.
pub struct WoomyRuntime<'a> {
    pub program: &'a Program,
    provider: &'a dyn RuntimeProvider,
    outer_return: u32,
    frames: Vec<u32>,
    calls: usize,
}
impl<'a> WoomyRuntime<'a> {
    pub fn new(program: &'a Program, state: &Machine) -> Self {
        Self::with_provider(program, state, program.provider.provider())
    }
    pub fn with_provider(
        program: &'a Program,
        state: &Machine,
        provider: &'a dyn RuntimeProvider,
    ) -> Self {
        Self {
            program,
            provider,
            outer_return: state.lr & !3,
            frames: vec![],
            calls: 0,
        }
    }
    pub fn fetch(&self, s: &Machine, pc: u32) -> Result<(), Exit> {
        if let Some(reason) = self.program.source_barriers.get(&pc) {
            return Err(Self::fault(pc, reason));
        }
        if pc & 3 != 0 || s.guest_memory.locate(pc, 4, EXECUTE).is_none() {
            return Err(Self::fault(pc, "NON_EXECUTABLE_GUEST_ADDRESS"));
        }
        Ok(())
    }
    pub fn fault(address: u32, reason: &str) -> Exit {
        Exit::Unsupported {
            address,
            reason: reason.into(),
        }
    }
    fn event(&self, s: &mut Machine, event: CallEvent) -> Result<(), Exit> {
        if s.calls.len() >= self.program.limits.trace {
            return Err(Exit::Limit);
        }
        s.calls.push(event);
        Ok(())
    }
    pub fn call(&mut self, s: &mut Machine, pc: u32, target: u32, kind: u32) -> Result<u32, Exit> {
        // Target is captured before LK changes LR (including bclrl).
        s.lr = pc.wrapping_add(4);
        if self.calls >= self.program.limits.calls || self.frames.len() >= self.program.limits.depth
        {
            return Err(Exit::Limit);
        }
        self.event(
            s,
            CallEvent {
                caller: self.program.owner(pc).unwrap_or(0),
                site: pc,
                target,
                kind,
                return_target: s.lr,
            },
        )?;
        self.calls += 1;
        if kind == 2 {
            let import = self
                .program
                .imports
                .get(&pc)
                .ok_or_else(|| Self::fault(pc, "UNRESOLVED_IMPORT"))?;
            let provider = self.provider;
            let handler = provider
                .resolve_import(import)
                .ok_or_else(|| Self::fault(pc, "UNRESOLVED_IMPORT"))?;
            provider
                .invoke_import(handler, s)
                .map_err(|why| Self::fault(pc, why))?;
            return Ok(s.lr & !3);
        }
        if self.program.function(target).is_none() {
            return Err(Self::fault(pc, "UNRESOLVED_CALL"));
        }
        self.fetch(s, target)?;
        self.frames.push(s.lr);
        Ok(target)
    }
    pub fn returning(&mut self, s: &mut Machine, pc: u32) -> Result<Option<u32>, Exit> {
        let target = s.lr & !3;
        self.event(
            s,
            CallEvent {
                caller: self.program.owner(pc).unwrap_or(0),
                site: pc,
                target,
                kind: 3,
                return_target: target,
            },
        )?;
        if self.frames.last() == Some(&target) {
            self.frames.pop();
        } else if target == self.outer_return && self.frames.is_empty() {
            return Ok(None);
        }
        if self.program.block(target).is_none() {
            return Err(Self::fault(pc, "UNREGISTERED_RETURN_TARGET"));
        }
        self.fetch(s, target)?;
        Ok(Some(target))
    }
}
