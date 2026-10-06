//! Woomy-side input descriptors and transport for the external Manmenmi M10 runtime.
//! Actual CafeProcess/CafeThread objects and services belong to upstream C++ M10.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use woomy_ir::{execution::Machine, runtime::*};
pub mod bridge;
pub mod provider;
pub mod thread;
pub use provider::{import_contract, ImportAbiContract, Manmenmi};
pub use thread::{CafeThreadObject, ThreadChunk};

pub const CAFE_PROCESS_VERSION: u32 = 1;
pub const CAFE_THREAD_MODEL_VERSION: u32 = 1;
pub const MANMENMI_PROVIDER_ABI_VERSION: u32 = 2;
pub const IMPORT_SEMANTICS_VERSION: u32 = 1;
pub type Result<T> = std::result::Result<T, &'static str>;

/// A guest value, deliberately without pointer conversions.
/// ```compile_fail
/// let address = woomy_manmenmi::Address32::try_from(0x70000000u64).unwrap();
/// let host: *const u8 = address.into();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Address32(u32);
impl Address32 {
    pub fn get(self) -> u32 {
        self.0
    }
    pub fn offset(self, offset: u32) -> Result<Self> {
        self.0
            .checked_add(offset)
            .map(Self)
            .ok_or("GUEST_ADDRESS_OVERFLOW")
    }
}
impl TryFrom<u64> for Address32 {
    type Error = &'static str;
    fn try_from(value: u64) -> Result<Self> {
        u32::try_from(value)
            .map(Self)
            .map_err(|_| "GUEST_ADDRESS_OVERFLOW")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceClass {
    Documented,
    PublicImplementationObservation,
    EmpiricallyObservedInLocalDump,
    ReferenceCapture,
    WoomyInferred,
    ManmenmiModel,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub class: EvidenceClass,
    pub source: String,
    pub detail: String,
}
impl Evidence {
    pub fn model(detail: &str) -> Self {
        Self {
            class: EvidenceClass::ManmenmiModel,
            source: "Manmenmi single-thread model v1".into(),
            detail: detail.into(),
        }
    }
    pub fn valid(&self) -> bool {
        self.class != EvidenceClass::Unknown
            && !self.source.is_empty()
            && self.source.len() <= 4096
            && !self.detail.is_empty()
            && self.detail.len() <= 4096
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ValueOrigin {
    RuntimeProvided,
    ExplicitTestInput,
    ReferenceCapture,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Known<T> {
    pub value: T,
    pub provenance: ValueOrigin,
    pub evidence: Evidence,
}
impl<T> Known<T> {
    pub fn model(value: T, detail: &str) -> Self {
        Self {
            value,
            provenance: ValueOrigin::RuntimeProvided,
            evidence: Evidence::model(detail),
        }
    }
    pub fn valid_for(&self, domain: Domain) -> bool {
        self.evidence.valid()
            && match domain {
                Domain::GameFunctionReferenceContext => {
                    self.provenance == ValueOrigin::ReferenceCapture
                        && self.evidence.class == EvidenceClass::ReferenceCapture
                }
                _ => {
                    self.provenance != ValueOrigin::ReferenceCapture
                        && self.evidence.class != EvidenceClass::ReferenceCapture
                }
            }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Domain {
    GameFunctionSyntheticDomain,
    CafeBootstrapModel,
    GameFunctionReferenceContext,
}
impl Domain {
    pub fn name(self) -> &'static str {
        match self {
            Self::GameFunctionSyntheticDomain => "GAME_FUNCTION_SYNTHETIC_DOMAIN",
            Self::CafeBootstrapModel => "CAFE_BOOTSTRAP_MODEL",
            Self::GameFunctionReferenceContext => "GAME_FUNCTION_REFERENCE_CONTEXT",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessVersions {
    pub process: u32,
    pub thread: u32,
    pub provider: u32,
    pub imports: u32,
    pub runtime: u32,
}
impl ProcessVersions {
    pub const CURRENT: Self = Self {
        process: CAFE_PROCESS_VERSION,
        thread: CAFE_THREAD_MODEL_VERSION,
        provider: MANMENMI_PROVIDER_ABI_VERSION,
        imports: IMPORT_SEMANTICS_VERSION,
        runtime: RUNTIME_ABI_VERSION,
    };
    pub fn map(self) -> BTreeMap<String, u32> {
        BTreeMap::from([
            ("process".into(), self.process),
            ("thread".into(), self.thread),
            ("provider".into(), self.provider),
            ("imports".into(), self.imports),
            ("runtime".into(), self.runtime),
            ("manmenmi_runtime_abi".into(), 1),
            ("manmenmi_service_abi".into(), 1),
            ("manmenmi_thread_abi".into(), 1),
            ("manmenmi_memory_abi".into(), 1),
            ("woomy_m10_transport".into(), 2),
        ])
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestRange {
    pub start: Address32,
    pub length: u32,
}
impl GuestRange {
    pub fn end(&self) -> u64 {
        self.start.get() as u64 + self.length as u64
    }
    pub fn valid(&self) -> bool {
        self.length > 0 && self.end() <= 0x1_0000_0000
    }
    pub fn overlaps(&self, other: &Self) -> bool {
        (self.start.get() as u64) < other.end() && (other.start.get() as u64) < self.end()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StackBounds {
    pub low: Address32,
    pub high: Address32,
    pub initial_sp: Address32,
}
impl StackBounds {
    pub fn range(self) -> Result<GuestRange> {
        let length = self
            .high
            .get()
            .checked_sub(self.low.get())
            .filter(|n| *n > 0)
            .ok_or("INVALID_STACK_BOUNDS")?;
        if self.initial_sp.get() < self.low.get()
            || self.initial_sp.get() >= self.high.get()
            || self.initial_sp.get() & 3 != 0
        {
            return Err("INVALID_STACK_POINTER");
        }
        Ok(GuestRange {
            start: self.low,
            length,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeRegion {
    pub region: Region,
    pub provenance: ValueOrigin,
    pub evidence: Evidence,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessLimits {
    pub steps: usize,
    pub calls: usize,
    pub depth: usize,
    pub trace: usize,
    pub memory_bytes: usize,
}
impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            steps: 512,
            calls: 32,
            depth: 16,
            trace: 128,
            memory_bytes: 1048576,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CafeProcessDescriptor {
    pub versions: ProcessVersions,
    /// Pinned adapter executable, which links the selected external M9 sources.
    pub runtime_artifact_hash: Option<String>,
    pub id: String,
    pub image: GuestImageIdentity,
    pub domain: Domain,
    pub current_core: Option<Known<u8>>,
    pub current_thread: Option<Known<Address32>>,
    pub debugger_initialized: Option<Known<bool>>,
    pub stack: Option<Known<StackBounds>>,
    pub threads: Vec<CafeThreadObject>,
    pub runtime_regions: Vec<RuntimeRegion>,
    /// Reserved initial-image ranges are not implicitly mapped as live memory.
    pub image_ranges: Vec<GuestRange>,
    pub available_imports: Vec<Import>,
    pub limits: ProcessLimits,
}
impl CafeProcessDescriptor {
    pub fn empty(
        id: &str,
        image: GuestImageIdentity,
        image_ranges: Vec<GuestRange>,
        domain: Domain,
    ) -> Self {
        Self {
            versions: ProcessVersions::CURRENT,
            runtime_artifact_hash: None,
            id: id.into(),
            image,
            domain,
            current_core: None,
            current_thread: None,
            debugger_initialized: None,
            stack: None,
            threads: vec![],
            runtime_regions: vec![],
            image_ranges,
            available_imports: vec![
                Import {
                    library: "coreinit".into(),
                    symbol: "OSGetCurrentThread".into(),
                },
                Import {
                    library: "coreinit".into(),
                    symbol: "OSIsDebuggerInitialized".into(),
                },
            ],
            limits: ProcessLimits::default(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self
            .runtime_artifact_hash
            .as_ref()
            .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("M9_ADAPTER_IDENTITY");
        }
        if self.versions != ProcessVersions::CURRENT
            || self.image.versions != ImageContextVersions::CURRENT
        {
            return Err("PROCESS_VERSION_MISMATCH");
        }
        if self.id.is_empty()
            || self.id.len() > 4096
            || self.image.module.is_empty()
            || self.image.module.len() > 4096
            || [&self.image.module_hash, &self.image.image_hash]
                .iter()
                .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("PROCESS_IMAGE_IDENTITY");
        }
        let l = &self.limits;
        if l.steps == 0
            || l.steps > 65536
            || l.calls > 65536
            || l.depth > 1024
            || l.trace > 65536
            || l.memory_bytes == 0
            || l.memory_bytes > 1048576
            || self.threads.len() > 32
            || self.runtime_regions.len() > 256
            || self.image_ranges.len() > 256
            || self.available_imports.len() > 256
        {
            return Err("PROCESS_LIMIT");
        }
        if self
            .current_core
            .as_ref()
            .is_some_and(|k| k.value > 2 || !k.valid_for(self.domain))
            || self
                .current_thread
                .as_ref()
                .is_some_and(|k| k.value.get() == 0 || !k.valid_for(self.domain))
            || self
                .debugger_initialized
                .as_ref()
                .is_some_and(|k| !k.valid_for(self.domain))
            || self
                .stack
                .as_ref()
                .is_some_and(|k| !k.valid_for(self.domain))
        {
            return Err("PROCESS_VALUE_PROVENANCE");
        }
        let mut reserved = self.image_ranges.clone();
        for (i, r) in reserved.iter().enumerate() {
            if !r.valid() || reserved[..i].iter().any(|p| p.overlaps(r)) {
                return Err("IMAGE_RANGE_OVERLAP_OR_OVERFLOW");
            }
        }
        for t in &self.threads {
            t.validate(self.domain)?;
            let r = t.range();
            if reserved.iter().any(|p| p.overlaps(&r)) {
                return Err("RUNTIME_OBJECT_OVERLAP");
            }
            reserved.push(r);
        }
        if self
            .current_thread
            .as_ref()
            .is_some_and(|k| !self.threads.iter().any(|t| t.address.value == k.value))
        {
            return Err("CURRENT_THREAD_OBJECT_UNAVAILABLE");
        }
        for r in &self.runtime_regions {
            if r.region.bytes.len() > l.memory_bytes {
                return Err("PROCESS_MEMORY_LIMIT");
            }
            let known = Known {
                value: (),
                provenance: r.provenance,
                evidence: r.evidence.clone(),
            };
            if !known.valid_for(self.domain) || ![3, 4].contains(&r.region.kind) {
                return Err("RUNTIME_REGION_PROVENANCE");
            }
            let range = GuestRange {
                start: Address32(r.region.base),
                length: u32::try_from(r.region.bytes.len()).map_err(|_| "PROCESS_LIMIT")?,
            };
            if !range.valid() || reserved.iter().any(|p| p.overlaps(&range)) {
                return Err("RUNTIME_REGION_OVERLAP_OR_OVERFLOW");
            }
            reserved.push(range);
        }
        let total = self
            .runtime_regions
            .iter()
            .map(|r| r.region.bytes.len())
            .sum::<usize>()
            + self
                .threads
                .iter()
                .flat_map(|t| &t.chunks)
                .map(|c| c.bytes.len())
                .sum::<usize>();
        if total > l.memory_bytes {
            return Err("PROCESS_MEMORY_LIMIT");
        }
        let memory = self.runtime_memory()?;
        if !memory.valid_with_limit(l.memory_bytes) {
            return Err("RUNTIME_MEMORY_LAYOUT_OR_BUDGET");
        }
        if let Some(stack) = &self.stack {
            let r = stack.value.range()?;
            if memory
                .locate(r.start.get(), r.length as usize, READ | WRITE)
                .is_none()
            {
                return Err("STACK_MAPPING_UNAVAILABLE");
            }
        }
        if self
            .available_imports
            .iter()
            .enumerate()
            .any(|(i, x)| import_contract(x).is_none() || self.available_imports[..i].contains(x))
        {
            return Err("UNSUPPORTED_IMPORT_BINDING");
        }
        Ok(())
    }
    pub fn runtime_memory(&self) -> Result<GuestMemory> {
        let mut regions: Vec<_> = self
            .runtime_regions
            .iter()
            .map(|r| r.region.clone())
            .collect();
        for t in &self.threads {
            regions.extend(t.regions()?);
        }
        regions.sort_by_key(|r| r.base);
        Ok(GuestMemory { regions })
    }
    pub fn identity(&self) -> Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| "PROCESS_SERIALIZATION")?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
    pub fn binding(&self) -> Result<RuntimeBinding> {
        Ok(RuntimeBinding {
            provider: "Manmenmi".into(),
            provider_abi: self.versions.provider,
            semantics: self.versions.imports,
            versions: self.versions.map(),
            context_hash: self.identity()?,
            process: self.id.clone(),
            module: self.image.module.clone(),
            module_hash: self.image.module_hash.clone(),
            image_hash: self.image.image_hash.clone(),
            domain: self.domain.name().into(),
            current_thread: self.current_thread.as_ref().map(|t| t.value.get()),
        })
    }
    /// Attach supplied bytes, not a living image or an implicit global thread.
    pub fn attach(&self, machine: &mut Machine, image: &GuestImageIdentity) -> Result<()> {
        self.validate()?;
        if image != &self.image {
            return Err("PROCESS_IMAGE_IDENTITY_MISMATCH");
        }
        if machine.runtime_context.is_some() || !machine.memory.is_empty() {
            return Err("MACHINE_ALREADY_BOUND_OR_FLAT_MEMORY");
        }
        let mut memory = machine.guest_memory.clone();
        memory.regions.extend(self.runtime_memory()?.regions);
        if !memory.valid_with_limit(self.limits.memory_bytes) {
            return Err("MACHINE_MEMORY_OVERLAP_OR_BUDGET");
        }
        let binding = self.binding()?;
        machine.guest_memory = memory;
        machine.runtime_context = Some(binding);
        Ok(())
    }
}
