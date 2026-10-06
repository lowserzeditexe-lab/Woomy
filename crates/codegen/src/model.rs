//! Explicit bounded validation artifact. Never changes normal function eligibility.
use crate::{hash, validation::NativeRun, Generated};
use std::{collections::BTreeSet, fmt::Write, io, path::Path};
use woomy_ir::{
    execution::Machine, runtime::Program, CallTarget, Destination, Operation, Terminator,
};
use woomy_manmenmi::{CafeProcessDescriptor, Domain, ValueOrigin};

pub struct ModelGenerated {
    pub generated: Generated,
    pub identity: String,
    initial_hash: String,
    budget: usize,
}
impl ModelGenerated {
    pub fn run(
        &self,
        executable: &Path,
        executable_hash: &str,
        initial: Machine,
    ) -> io::Result<NativeRun> {
        if hash(&serde_json::to_vec(&initial)?) != self.initial_hash {
            return Err(io::Error::other(
                "NATIVE_INITIAL_STATE_OR_RUNTIME_IDENTITY_MISMATCH",
            ));
        }
        let metadata = std::fs::metadata(executable)?;
        if metadata.len() > 64 * 1024 * 1024 || hash(&std::fs::read(executable)?) != executable_hash
        {
            return Err(io::Error::other("NATIVE_EXECUTABLE_IDENTITY_MISMATCH"));
        }
        crate::validation::run_transport(
            executable,
            &[(initial, self.budget)],
            Some(&self.identity),
        )
    }
}

/// The descriptor is input to the real C runtime, never an implementation of it.
/// Only CFG reachable before explicit source barriers is emitted. A barrier emits
/// a fault before fetch, without executing or inventing the unresolved operation.
pub fn generate(
    program: &Program,
    process: &CafeProcessDescriptor,
    initial: &Machine,
) -> Result<ModelGenerated, String> {
    program.validate()?;
    process.validate().map_err(str::to_owned)?;
    let binding = process.binding().map_err(str::to_owned)?;
    if program.runtime_context.as_ref() != Some(&binding)
        || initial.runtime_context.as_ref() != Some(&binding)
        || program.image.as_ref() != Some(&process.image)
        || process.domain != Domain::CafeBootstrapModel
        || !initial
            .guest_memory
            .valid_with_limit(process.limits.memory_bytes)
        || !initial.memory.is_empty()
        || program.functions.len() != 1
    {
        return Err("NATIVE_MODEL_IDENTITY_OR_SCOPE".into());
    }
    let mut prefix = program.clone();
    let mut visited = BTreeSet::new();
    let mut queue = vec![program.entry];
    let mut blocks = vec![];
    while let Some(pc) = queue.pop() {
        if !visited.insert(pc) {
            continue;
        }
        let mut block = program
            .block(pc)
            .ok_or("NATIVE_PREFIX_MISSING_BLOCK")?
            .clone();
        if let Some(source) = block
            .sources
            .iter()
            .find(|s| program.source_barriers.contains_key(&s.address))
        {
            let address = source.address;
            let provenance = block
                .instructions
                .iter()
                .find(|i| i.source.address == address)
                .map(|i| i.source.clone())
                .unwrap_or_else(|| block.terminator_source.clone());
            block.sources.retain(|s| s.address <= address);
            block.instructions.retain(|i| i.source.address < address);
            block.terminator_source = provenance;
            block.successors.clear();
            block.terminator = Terminator::Trap; // Unreachable: fault injected before this source.
        } else {
            match block.terminator {
                Terminator::Jump {
                    target: Destination::Block(t),
                } => queue.push(t),
                Terminator::Branch {
                    taken: Destination::Block(a),
                    not_taken: Destination::Block(b),
                    ..
                } => queue.extend([a, b]),
                Terminator::Return { .. } | Terminator::Trap => (),
                _ => return Err("NATIVE_PREFIX_UNSUPPORTED_CONTROL".into()),
            }
        }
        if block.instructions.iter().any(|i| matches!(&i.operation, Operation::Call { target, .. } if !matches!(target,CallTarget::Imported { .. }))) {
            return Err("NATIVE_MODEL_ONLY_EXPLICIT_IMPORT_CONTRACTS".into());
        }
        blocks.push(block);
    }
    blocks.sort_by_key(|b| b.id);
    prefix.functions[0].blocks = blocks;
    let sites: BTreeSet<_> = prefix.functions[0]
        .blocks
        .iter()
        .flat_map(|b| &b.instructions)
        .filter(|i| matches!(i.operation, Operation::Call { .. }))
        .map(|i| i.source.address)
        .collect();
    prefix.imports.retain(|pc, _| sites.contains(pc));
    for import in prefix.imports.values() {
        if !process.available_imports.contains(import)
            || woomy_manmenmi::import_contract(import).is_none()
        {
            return Err("NATIVE_UNSUPPORTED_IMPORT_SIGNATURE".into());
        }
    }
    prefix.source_barriers.retain(|pc, _| {
        prefix.functions[0]
            .blocks
            .iter()
            .any(|b| b.sources.iter().any(|s| s.address == *pc))
    });
    let verification = woomy_ir::verify(&prefix.functions[0]);
    if !verification.structural_valid {
        return Err(format!("NATIVE_PREFIX_IR: {:?}", verification.diagnostics));
    }
    prefix.validate()?;
    let mut generated = crate::generate_inner(&prefix.functions[0], Some(&prefix));
    let mut source = generated.source.take().ok_or_else(|| {
        format!(
            "NATIVE_PREFIX_UNSUPPORTED: {:?}",
            generated.manifest.diagnostics
        )
    })?;
    let initial_hash = hash(&serde_json::to_vec(initial).map_err(|e| e.to_string())?);
    let identity = hash(
        &serde_json::to_vec(&(
            1u32,
            program,
            process,
            &initial_hash,
            include_str!("model.cpp"),
            include_str!("runtime.cpp"),
            include_str!("harness.cpp"),
            crate::CPP_BACKEND_VERSION,
            crate::NATIVE_ABI_VERSION,
        ))
        .map_err(|e| e.to_string())?,
    );
    let mut setup = String::new();
    writeln!(
        setup,
        "MnmOwner mnm; if (!mnm.create()) throw std::runtime_error(\"M10_RUNTIME_INIT\");"
    )
    .unwrap();
    let debugger = process
        .debugger_initialized
        .as_ref()
        .map_or(0, |b| if b.value { 2 } else { 1 });
    writeln!(setup,"if(mnm_process_set_debugger_state(mnm.process,static_cast<mnm_debugger_state>({debugger}u))!=MNM_OK) throw std::runtime_error(\"M10_DEBUGGER_INIT\");").unwrap();
    for t in &process.threads {
        let provenance = match t.address.provenance {
            ValueOrigin::RuntimeProvided => 0,
            ValueOrigin::ExplicitTestInput => 1,
            ValueOrigin::ReferenceCapture => 2,
        };
        let current = process
            .current_thread
            .as_ref()
            .is_some_and(|c| c.value == t.address.value);
        writeln!(
            setup,
            "if(!mnm.thread({}u,{}u,{})) throw std::runtime_error(\"M10_THREAD_INIT\");",
            t.address.value.get(),
            provenance,
            current
        )
        .unwrap();
    }
    let mut mapping = String::new();
    if let Some(current) = &process.current_thread {
        let thread = process
            .threads
            .iter()
            .find(|t| t.address.value == current.value)
            .ok_or("CURRENT_THREAD_OBJECT_UNAVAILABLE")?;
        for chunk in &thread.chunks {
            writeln!(mapping,"{{ size_t i,n; if(!location(r.state,{}u,{}u,1u,i,n)) return fail(r,pc,\"CURRENT_THREAD_MAPPING_UNAVAILABLE\"); }}",current.value.get().checked_add(chunk.offset).ok_or("THREAD_OVERFLOW")?,chunk.bytes.len()).unwrap();
        }
    }
    let bridge = include_str!("model.cpp").replace("// THREAD_MAPPING", &mapping);
    source = source.replace(
        "struct WoomyRuntime {",
        &format!("{bridge}\nstruct WoomyRuntime {{\nMnmOwner* mnm=nullptr;"),
    );
    let runtime_start = source
        .find("struct WoomyRuntime {")
        .ok_or("GENERATOR_MARKER")?;
    let begin = source[runtime_start..]
        .find("    bool invoke(Result& r,")
        .ok_or("GENERATOR_MARKER")?
        + runtime_start;
    let end = source[begin..]
        .find("    bool call(Result& r,")
        .ok_or("GENERATOR_MARKER")?
        + begin;
    source.replace_range(begin..end,"    bool invoke(Result& r,uint32_t pc,uint32_t handler) { return mnm && mnm->invoke(r,pc,handler); }\n");
    // Actual provider handler IDs are service identities, never guest addresses.
    for (pc, import) in &prefix.imports {
        let handler = if import.symbol == "OSGetCurrentThread" {
            1
        } else {
            2
        };
        source = source.replace(
            &format!("rt.imports.push_back({{{pc}u,0u}});"),
            &format!("rt.imports.push_back({{{pc}u,{handler}u}});"),
        );
    }
    source = source.replace(" WoomyRuntime rt(", &format!(" {setup}\n WoomyRuntime rt("));
    source = source.replace("rt.entries.push_back", "rt.mnm=&mnm; rt.entries.push_back");
    for (pc, why) in &prefix.source_barriers {
        let bytes = why
            .bytes()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(",");
        source = source.replace(&format!("if (!rt.fetch(r, {pc}u)) return r;"), &format!("{{ const char reason[]={{ {bytes},0 }}; return fault(std::move(r),{pc}u,reason,4u); }}"));
    }
    // Explicit identity transport and echo. No host-layout serialization.
    source = source.replace("if(read32()!=0x574f4f32u)", &format!("const char identity[]=\"{identity}\"; for(unsigned j=0;j<64;++j) if(read_number(1)!=uint8_t(identity[j])) return 8; for(unsigned j=0;j<64;++j) number(uint8_t(identity[j]),1);\n        if(read32()!=0x574f4f32u)"));
    generated.manifest.runtime_target = "MANMENMI_M10_C_ABI_MODEL_VALIDATION_ONLY".into();
    generated.manifest.ir_hash = hash(&serde_json::to_vec(program).map_err(|e| e.to_string())?);
    generated.manifest.cache_key = identity.clone();
    generated.manifest.source_hash = Some(hash(source.as_bytes()));
    generated.source = Some(source);
    Ok(ModelGenerated {
        generated,
        identity,
        initial_hash,
        budget: process.limits.steps,
    })
}
