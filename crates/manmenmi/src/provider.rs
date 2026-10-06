use super::*;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportAbiContract {
    pub version: u32,
    pub import: Import,
    pub status: String,
    pub arguments: Vec<String>,
    pub result: String,
    pub modeled_writes: Vec<String>,
    pub modeled_memory_effects: String,
    pub modeled_preservation: String,
    pub cafe_unknown_effects: Vec<String>,
    pub evidence: Vec<Evidence>,
}
pub fn import_contract(import: &Import) -> Option<ImportAbiContract> {
    if import.library != "coreinit"
        || !["OSGetCurrentThread", "OSIsDebuggerInitialized"].contains(&import.symbol.as_str())
    {
        return None;
    }
    let thread = import.symbol == "OSGetCurrentThread";
    Some(ImportAbiContract { version:IMPORT_SEMANTICS_VERSION,import:import.clone(),status:"MODEL_IMPLEMENTED".into(),arguments:vec![],
        result:if thread {"r3 = associated guest OSThread address"} else {"r3 = explicit debugger_initialized boolean (0 or 1)"}.into(),
        modeled_writes:vec!["r3".into()],modeled_memory_effects:"No guest memory read/write; consult explicit process state and validate object mappings".into(),
        modeled_preservation:"Model preserves all other machine fields; dispatcher handles LR=site+4. This is not a proven Cafe preservation set.".into(),
        cafe_unknown_effects:vec!["Full volatile/nonvolatile register effects".into(),"Internal memory, synchronization, exceptions and timing".into()],
        evidence:vec![Evidence { class:EvidenceClass::PublicImplementationObservation, source:if thread {thread::THREAD_SOURCE} else {"https://github.com/devkitPro/wut/blob/master/include/coreinit/debug.h"}.into(), detail:"Independent public zero-argument declaration and pointer/BOOL return; not an official Cafe specification".into() },
            Evidence { class:EvidenceClass::Documented,source:"https://refspecs.linuxfoundation.org/elf/elfspec_ppc.pdf#page=38".into(),detail:"PPC ABI scalar/pointer result in r3; corroboration, not automatic proof of the complete Cafe ABI".into() },
            Evidence::model("Only the explicit model effect set is executable; no reference validation claimed") ] })
}
pub struct Manmenmi {
    process: CafeProcessDescriptor,
    binding: RuntimeBinding,
    bridge: std::sync::Mutex<crate::bridge::Bridge>,
}
impl Manmenmi {
    pub fn connect(process: CafeProcessDescriptor, executable: &std::path::Path) -> Result<Self> {
        let binding = process.binding()?;
        let hash = process
            .runtime_artifact_hash
            .as_ref()
            .ok_or("M9_ADAPTER_NOT_CONFIGURED")?;
        let bridge = crate::bridge::Bridge::connect(executable, hash, &process)?;
        Ok(Self {
            process,
            binding,
            bridge: std::sync::Mutex::new(bridge),
        })
    }
    pub fn process(&self) -> &CafeProcessDescriptor {
        &self.process
    }
    fn validate_machine(&self, state: &Machine) -> Result<()> {
        if state.runtime_context.as_ref() != Some(&self.binding) {
            return Err("RUNTIME_CONTEXT_OR_PROVIDER_MISMATCH");
        }
        if self.process.domain == Domain::GameFunctionReferenceContext {
            return Err("MODEL_HANDLER_NOT_REFERENCE_VALIDATED");
        }
        if !state
            .guest_memory
            .valid_with_limit(self.process.limits.memory_bytes)
        {
            return Err("RUNTIME_MEMORY_LAYOUT_OR_BUDGET");
        }
        Ok(())
    }
}
impl RuntimeProvider for Manmenmi {
    fn binding(&self) -> Option<RuntimeBinding> {
        Some(self.binding.clone())
    }
    fn resolve_import(&self, import: &Import) -> Option<u32> {
        if !self.process.available_imports.contains(import) {
            return None;
        }
        match (import.library.as_str(), import.symbol.as_str()) {
            ("coreinit", "OSGetCurrentThread") => Some(1),
            ("coreinit", "OSIsDebuggerInitialized") => Some(2),
            _ => None,
        }
    }
    fn invoke_import(&self, handler: u32, state: &mut Machine) -> Result<()> {
        self.validate_machine(state)?;
        let symbol = match handler {
            1 => "OSGetCurrentThread",
            2 => "OSIsDebuggerInitialized",
            _ => return Err("UNRESOLVED_IMPORT"),
        };
        if self.resolve_import(&Import {
            library: "coreinit".into(),
            symbol: symbol.into(),
        }) != Some(handler)
        {
            return Err("UNRESOLVED_IMPORT");
        }
        if handler == 1 {
            if let Some(current) = &self.process.current_thread {
                let address = current.value;
                let thread = self
                    .process
                    .threads
                    .iter()
                    .find(|t| t.address.value == address)
                    .ok_or("CURRENT_THREAD_OBJECT_UNAVAILABLE")?;
                for c in &thread.chunks {
                    if state
                        .guest_memory
                        .locate(address.offset(c.offset)?.get(), c.bytes.len(), READ)
                        .is_none()
                    {
                        return Err("CURRENT_THREAD_MAPPING_UNAVAILABLE");
                    }
                }
            }
        }
        // Values come from actual CafeRuntime -> CafeProcess -> service invocation.
        let value = self
            .bridge
            .lock()
            .map_err(|_| "M9_TRANSPORT_POISONED")?
            .invoke(handler, state)?
            .ok_or("M9_RETURN_ABI_MISMATCH")?;
        state.gpr[3] = value;
        Ok(())
    }
    fn query_capability(&self) -> &'static str {
        "MANMENMI_M10_C_ABI_EXTERNAL_RUNTIME"
    }
}
