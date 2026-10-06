//! Static entry-frontier candidates and explicitly requested model execution.
use crate::{
    cafe::CafeGuestImage, context::ExecutionContextBuilder, Error, ProjectService, Result,
};
use manmenmi::{
    Address32, CafeProcessDescriptor, CafeThreadObject, Domain, Evidence, GuestRange, Known,
    Manmenmi, RuntimeRegion, StackBounds, ValueOrigin,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{
    execution::{Execution, Exit, Machine},
    runtime::*,
    CallTarget, Destination, Function, Operation, Terminator,
};
fn failure(why: &str) -> Error {
    Error::Analysis(why.into())
}
pub fn image_identity(image: &CafeGuestImage) -> GuestImageIdentity {
    GuestImageIdentity {
        versions: ImageContextVersions::CURRENT,
        module: image.module().into(),
        module_hash: image.module_hash().into(),
        image_hash: image.hash().into(),
    }
}
pub fn process_descriptor(
    image: &CafeGuestImage,
    profile: Option<bool>,
    artifact: Option<String>,
) -> Result<CafeProcessDescriptor> {
    let ranges = image
        .regions()
        .iter()
        .map(|r| {
            Ok(GuestRange {
                start: Address32::try_from(r.start as u64).map_err(failure)?,
                length: (r.end - r.start as u64) as u32,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut process = CafeProcessDescriptor::empty(
        "woomy-bootstrap-single-thread-v1",
        image_identity(image),
        ranges,
        Domain::CafeBootstrapModel,
    );
    process.runtime_artifact_hash = artifact;
    if let Some(debugger) = profile {
        let address = |v| Address32::try_from(v).map_err(failure);
        let stack = StackBounds {
            low: address(0x70010000)?,
            high: address(0x70020000)?,
            initial_sp: address(0x7001ff00)?,
        };
        let thread_address = address(0x70000000)?;
        process.current_core = Some(Known::model(
            1,
            "Explicit single-core model selection; actual Cafe entry core is not captured",
        ));
        process.current_thread = Some(Known::model(
            thread_address,
            "Model-selected nonoverlapping runtime address; never a host pointer",
        ));
        process.debugger_initialized = Some(Known::model(
            debugger,
            "Explicit debugger-absent/debugger-present profile",
        ));
        process.stack = Some(Known::model(
            stack,
            "Model-only 64 KiB stack with entry SP and caller save space",
        ));
        process.threads.push(
            CafeThreadObject::model_zeroed(
                thread_address,
                stack,
                address(image.entry() as u64)?,
                1,
            )
            .map_err(failure)?,
        );
        process.runtime_regions.push(RuntimeRegion {
            region: Region {
                base: stack.low.get(),
                permissions: READ | WRITE,
                kind: 3,
                provenance: "RuntimeProvided: explicit zero-filled bootstrap model stack".into(),
                bytes: vec![0; 65536],
            },
            provenance: ValueOrigin::RuntimeProvided,
            evidence: Evidence::model(
                "Every initial stack byte explicitly zeroed for this model; not Cafe initial RAM",
            ),
        });
    }
    process.validate().map_err(failure)?;
    Ok(process)
}
pub type PreparedEntry = (Program, BTreeMap<u32, u32>, GuestMemory, Vec<Value>);
pub fn entry_program(
    image: &CafeGuestImage,
    function: Function,
    process: Option<&CafeProcessDescriptor>,
) -> Result<PreparedEntry> {
    image.verify()?;
    if function.module != image.module()
        || function.module_hash != image.module_hash()
        || !woomy_ir::verify(&function).structural_valid
    {
        return Err(failure("ENTRY_IMAGE_IR_IDENTITY"));
    }
    let mut imports = BTreeMap::new();
    let mut barriers = BTreeMap::new();
    let mut memory = GuestMemory::default();
    let mut code = BTreeMap::new();
    let mut evidence = vec![];
    for block in &function.blocks {
        for op in &block.instructions {
            if let Operation::Call {
                target: CallTarget::Imported { library, symbol },
                ..
            } = &op.operation
            {
                imports.insert(
                    op.source.address,
                    Import {
                        library: library.clone(),
                        symbol: symbol.clone(),
                    },
                );
            }
        }
    }
    for block in &function.blocks {
        let size = block
            .sources
            .len()
            .checked_mul(4)
            .ok_or_else(|| failure("ENTRY_SIZE"))?;
        let (i, n) = image
            .memory()
            .locate(block.id, size, READ | EXECUTE)
            .ok_or_else(|| failure("ENTRY_SOURCE_MAPPING"))?;
        let source = &image.memory().regions[i];
        memory.regions.push(Region {base:block.id,permissions:source.permissions,kind:0,provenance:format!("{}; source barriers before unresolved fields; import dispatch by proven site binding",source.provenance),bytes:source.bytes[n..n+size].to_vec()});
        for src in &block.sources {
            let offset = n + (src.address - block.id) as usize;
            let word = u32::from_be_bytes(
                source.bytes[offset..offset + 4]
                    .try_into()
                    .expect("bounded word"),
            );
            code.insert(src.address, word);
            if !image.range_resolved(src.address, 4) {
                let relocations: Vec<_> = image
                    .relocations()
                    .iter()
                    .filter(|r| {
                        r.state != "APPLIED_PROVEN_LOCAL"
                            && (r.place as u64) < src.address as u64 + 4
                            && r.place as u64 + r.original.len().max(2) as u64 > src.address as u64
                    })
                    .collect();
                let bound = imports.get(&src.address).is_some_and(|import| {
                    !relocations.is_empty()
                        && relocations.iter().all(|r| {
                            r.state == "DEFERRED_IMPORT"
                                && r.kind == 10
                                && r.place == src.address
                                && r.addend == Some(0)
                                && r.import.as_ref()
                                    == Some(&(import.library.clone(), import.symbol.clone()))
                        })
                }) && word >> 26 == 18
                    && word & 3 == 1;
                if !bound {
                    let why = format!(
                        "UNRESOLVED_SOURCE_RELOCATION: {}",
                        relocations
                            .iter()
                            .map(|r| format!(
                                "{}:{}@{:08X}",
                                r.state,
                                r.symbol.as_deref().unwrap_or("UNKNOWN"),
                                r.place
                            ))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                    barriers.insert(src.address, why.clone());
                    evidence.push(json!({"pc":src.address,"reason":why,"relocations":relocations}));
                }
            }
        }
    }
    if !memory.valid() {
        return Err(failure("ENTRY_MEMORY_BUDGET"));
    }
    let program = Program {
        image: Some(image_identity(image)),
        runtime_context: process.map(|p| p.binding().map_err(failure)).transpose()?,
        source_barriers: barriers,
        version: RUNTIME_ABI_VERSION,
        entry: function.entry,
        functions: vec![function],
        imports,
        provider: if process.is_some() {
            ProviderKind::ExternalRuntime
        } else {
            ProviderKind::NoRuntime
        },
        limits: Limits {
            calls: 32,
            depth: 16,
            trace: 128,
        },
    };
    program.validate().map_err(Error::Analysis)?;
    Ok((program, code, memory, evidence))
}
/// Enumerates conditional CFG candidates without running a Machine or provider.
/// Memory/register prerequisites remain explicit; these counts are not executions.
pub fn static_frontiers(program: &Program, with_handlers: bool) -> Vec<Value> {
    let mut queue = vec![(
        program.entry,
        Vec::<u32>::new(),
        Vec::<u32>::new(),
        Vec::<u32>::new(),
        Vec::<String>::new(),
    )];
    let mut result = vec![];
    while let Some((pc, mut trace, mut blocks, mut calls, conditions)) = queue.pop() {
        if result.len() + queue.len() >= 64 {
            result.push(json!({"reason":"STATIC_FRONTIER_PATH_LIMIT"}));
            break;
        }
        let Some(block) = program.block(pc) else {
            result.push(json!({"pc":pc,"reason":"UNREGISTERED_BLOCK"}));
            continue;
        };
        if blocks.contains(&pc) || trace.len() >= 512 {
            result.push(json!({"pc":pc,"reason":"STATIC_LOOP_OR_STEP_LIMIT"}));
            continue;
        }
        let mut stop = None;
        for source in &block.sources {
            if let Some(why) = program.source_barriers.get(&source.address) {
                stop = Some((source.address, why.clone()));
                break;
            }
            let operations: Vec<_> = block
                .instructions
                .iter()
                .filter(|i| i.source.address == source.address)
                .collect();
            if operations.iter().any(|i| {
                matches!(
                    i.operation,
                    Operation::UnknownInstruction { .. }
                        | Operation::UnknownValue { .. }
                        | Operation::UnknownSideEffect { .. }
                        | Operation::OpaqueMemoryEffect { .. }
                )
            }) {
                stop = Some((source.address, "UNSUPPORTED_IR".into()));
                break;
            }
            if let Some(call) = operations.iter().find_map(|i| {
                if let Operation::Call { target, .. } = &i.operation {
                    Some(target)
                } else {
                    None
                }
            }) {
                match call {
                    CallTarget::Imported { library, symbol }
                        if with_handlers
                            && manmenmi::import_contract(&Import {
                                library: library.clone(),
                                symbol: symbol.clone(),
                            })
                            .is_some() =>
                    {
                        calls.push(source.address)
                    }
                    _ => {
                        stop = Some((
                            source.address,
                            format!("CALL_REQUIRES_RUNTIME_OR_CALLEE: {call:?}"),
                        ));
                        break;
                    }
                }
            }
            trace.push(source.address);
        }
        if stop.is_none() {
            blocks.push(pc);
        }
        let endpoint = match &block.terminator {
            Terminator::Jump {
                target: Destination::Block(target),
            } if stop.is_none() => {
                queue.push((*target, trace, blocks, calls, conditions));
                continue;
            }
            Terminator::Branch {
                taken: Destination::Block(a),
                not_taken: Destination::Block(b),
                ..
            } if stop.is_none() => {
                for (target, label) in [(*b, "not_taken"), (*a, "taken")] {
                    let mut guards = conditions.clone();
                    guards.push(format!(
                        "branch@{:08X}={label}",
                        block.terminator_source.address
                    ));
                    queue.push((target, trace.clone(), blocks.clone(), calls.clone(), guards));
                }
                continue;
            }
            Terminator::Trap => "ARCHITECTURAL_TRAP_CAFE_DEBUG_CONTROL_UNKNOWN",
            Terminator::Return { .. } => "RETURN_REQUIRES_CALLER_CONTEXT",
            _ => "UNRESOLVED_CONTROL_FLOW",
        };
        let (site, reason) = stop.unwrap_or((block.terminator_source.address, endpoint.into()));
        result.push(json!({"frontier_pc":site,"reason":reason,"conditional_instructions":trace.len(),"completed_blocks":blocks.len(),"modeled_imports":calls.len(),"trace":trace,"blocks":blocks,"calls":calls,"conditions":conditions,"execution_performed":false,"prerequisite":"All register/memory inputs and provider context must be explicitly supplied; paths are candidates, not a dynamic trace"}));
    }
    result
}
fn execution_counts(program: &Program, e: &Execution) -> Value {
    let blocks: BTreeSet<_> = program
        .functions
        .iter()
        .flat_map(|f| &f.blocks)
        .filter(|b| b.sources.iter().any(|s| e.trace.contains(&s.address)))
        .map(|b| b.id)
        .collect();
    json!({"instructions":e.trace.len(),"blocks":blocks.len(),"calls":e.state.calls.len(),"import_calls":e.state.calls.iter().filter(|c|c.kind==2).count(),"writes":e.state.writes.len(),"last_executed_pc":e.trace.last(),"exit":e.exit})
}
impl ProjectService {
    fn entry_ir(&self, image: &CafeGuestImage) -> Result<Function> {
        let id: String = self.db.query_row(
            "SELECT id FROM functions WHERE module=?1 AND address=?2",
            rusqlite::params![image.module(), image.entry()],
            |r| r.get(0),
        )?;
        self.read_ir(&id)
    }
    pub fn runtime_process(&self, profile: Option<bool>, thread_only: bool) -> Result<Value> {
        let image = self.guest_image(None)?;
        let adapter = if profile.is_some() {
            Some(self.manmenmi_adapter()?)
        } else {
            None
        };
        let process = process_descriptor(&image, profile, adapter.map(|(m, _)| m.executable_hash))?;
        if thread_only {
            Ok(
                json!({"domain":process.domain,"current_thread":process.current_thread,"stack":process.stack,"threads":process.threads.iter().map(|t|json!({"guest_address":t.address,"initialization":t.initialization,"fields":t.fields,"chunks":t.chunks.iter().map(|c|json!({"offset":c.offset,"bytes":c.bytes.len(),"sha256":crate::hash_bytes(&c.bytes),"provenance":c.provenance,"evidence":c.evidence})).collect::<Vec<_>>(),"other_field_semantics":"UNKNOWN"})).collect::<Vec<_>>(),"execution_performed":false}),
            )
        } else {
            let mut value = serde_json::to_value(&process)?;
            value["threads"] = json!(process.threads.len());
            value["runtime_regions"]=json!(process.runtime_regions.iter().map(|r|json!({"base":r.region.base,"bytes":r.region.bytes.len(),"permissions":r.region.permissions,"provenance":r.provenance,"evidence":r.evidence})).collect::<Vec<_>>());
            value["descriptor_only"] = json!(true);
            value["runtime_object_owner"] =
                json!("External Manmenmi M10; instantiated only by explicit execution");
            Ok(value)
        }
    }
    pub fn entry_frontier(&self) -> Result<Value> {
        let image = self.guest_image(None)?;
        let ir = self.entry_ir(&image)?;
        let context = ExecutionContextBuilder::new(&image).analyze(&ir, false);
        let (program, _, _, evidence) = entry_program(&image, ir, None)?;
        Ok(
            json!({"entry":image.entry(),"image_hash":image.hash(),"runtime_abi":RUNTIME_ABI_VERSION,"process_versions":manmenmi::ProcessVersions::CURRENT,"execution_performed":false,"actual_game_execution_ready":false,"without_provider":static_frontiers(&program,false),"with_two_modeled_handlers_and_supplied_context":static_frontiers(&program,true),"context":{"state":context.state,"required_registers":context.required_registers,"memory_accesses":context.memory_accesses,"calls":context.calls,"blockers":context.blockers},"source_barriers":evidence,"scope":"Conditional known entry CFG only; branch choices require explicit context; no game instruction executed"}),
        )
    }
    pub fn run_entry_model(&self, debugger: bool) -> Result<Value> {
        let (adapter, executable) = self.manmenmi_adapter()?;
        let image = self.guest_image(None)?;
        let ir = self.entry_ir(&image)?;
        let process = process_descriptor(
            &image,
            Some(debugger),
            Some(adapter.executable_hash.clone()),
        )?;
        let (program, code, memory, barriers) = entry_program(&image, ir, Some(&process))?;
        let mut initial = Machine::new(0, vec![]);
        // Explicit profile initialization of every register, not missing-state defaults.
        initial.gpr = [0; 32];
        initial.fpr = [0; 32];
        initial.cr = [0; 8];
        initial.ctr = 0;
        initial.xer = 0;
        initial.fpscr = 0;
        initial.lr = 0xffff0003;
        initial.gpr[1] = process
            .stack
            .as_ref()
            .ok_or_else(|| failure("MODEL_STACK"))?
            .value
            .initial_sp
            .get();
        initial.guest_memory = memory;
        process
            .attach(&mut initial, &image_identity(&image))
            .map_err(failure)?;
        let ppc_provider = Manmenmi::connect(process.clone(), &executable).map_err(failure)?;
        let ppc = woomy_ir::execution::execute_ppc_program_with_provider(
            &program,
            &code,
            initial.clone(),
            process.limits.steps,
            &ppc_provider,
        );
        let ir_provider = Manmenmi::connect(process.clone(), &executable).map_err(failure)?;
        let ir = woomy_ir::execution::execute_ir_program_with_provider(
            &program,
            initial.clone(),
            process.limits.steps,
            &ir_provider,
        );
        let generated = woomy_codegen::model::generate(&program, &process, &initial)
            .map_err(Error::Analysis)?;
        let compiler =
            woomy_codegen::compiler::NativeCompiler::detect().map_err(Error::Analysis)?;
        let directory = self.output.path("generated/runtime")?;
        let root = &adapter.source_root;
        self.output.write_atomic(
            &format!("generated/runtime/model-{}.cpp", generated.identity),
            generated
                .generated
                .source
                .as_ref()
                .ok_or_else(|| failure("NATIVE_MODEL_SOURCE"))?
                .as_bytes(),
        )?;
        let build = compiler.build_adapter(
            generated
                .generated
                .source
                .as_ref()
                .ok_or_else(|| failure("NATIVE_MODEL_SOURCE"))?,
            &[
                root.join("abi/include"),
                root.join("cafe/include"),
                root.join("core/include"),
            ],
            &crate::manmenmi_integration::M10_UNITS
                .iter()
                .map(|p| root.join(p))
                .collect::<Vec<_>>(),
            &directory,
        )?;
        self.output.write_atomic(
            "reports/native-model-build.json",
            &serde_json::to_vec_pretty(&build.record)?,
        )?;
        let bytes = build.executable.ok_or_else(|| {
            failure("NATIVE_MODEL_BUILD_FAILED: see reports/native-model-build.json")
        })?;
        // Recheck the pinned external sources after compilation as well.
        self.manmenmi_adapter()?;
        let executable_hash = crate::hash_bytes(&bytes);
        let native_path = format!("generated/runtime/{executable_hash}.exe");
        self.output.write_atomic(&native_path, &bytes)?;
        let mut native_run = generated.run(
            &self.output.path(&native_path)?,
            &executable_hash,
            initial.clone(),
        )?;
        let native = native_run
            .executions
            .pop()
            .ok_or_else(|| failure("NATIVE_MODEL_NO_RESULT"))?;
        let equal = ppc == ir && ir == native;
        let complete = matches!(ppc.exit, Exit::Returned { .. });
        let summary = json!({"domain":"CAFE_BOOTSTRAP_MODEL","status":if equal {"BOOTSTRAP_MODEL_EXECUTED"}else{"BOOTSTRAP_MODEL_MISMATCH"},"comparison":if !equal {"MISMATCH"}else if complete {"MODEL_EXECUTION_AGREEMENT"}else{"INCONCLUSIVE"},"ppc_ir_equal":ppc==ir,"ppc_native_equal":ppc==native,"ir_native_equal":ir==native,"real_game_context_match":false,"reference_validated":false,"native_execution":true,"native_status":if equal {"NATIVE_BOOTSTRAP_MODEL_AGREEMENT"}else{"MISMATCH"},"native_runtime_abi":[1,1,1,1],"woomy_native_model_abi":1,"runtime_executable_subset":if equal {"Two explicit no-argument coreinit model imports and supported pre-barrier CFG"}else{"NONE"},"runtime_execution_state":if equal {"RUNTIME_EXECUTABLE"}else{"MISMATCH"},"native_counts":execution_counts(&program,&native),"native_identity":generated.identity,"native_executable":native_path,"native_executable_hash":executable_hash,"counts":execution_counts(&program,&ppc),"debugger_initialized":debugger,"current_thread":process.current_thread,"runtime_context":initial.runtime_context,"source_barriers":barriers,"register_provenance":"MANMENMI_MODEL: explicit zeros for all GPR/FPR/CR/CTR/XER/FPSCR; r1=model stack, LR=outer harness sentinel","complete_entry":complete});
        let report = json!({"summary":summary,"adapter":adapter,"process":process,"program":program,"initial":initial,"ppc":ppc,"ir":ir,"native":native,"native_manifest":generated.generated.manifest});
        let name = if debugger {
            "debugger-present"
        } else {
            "debugger-absent"
        };
        let path = format!("reports/entry-model-{name}.json");
        self.output
            .write_atomic(&path, &serde_json::to_vec_pretty(&report)?)?;
        let mut result = summary;
        result["report"] = json!(path);
        Ok(result)
    }
}
