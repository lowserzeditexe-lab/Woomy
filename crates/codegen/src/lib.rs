//! IR-only deterministic C++ backend. No PPC decoding or host guest pointers.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write;
use woomy_ir::{Binary, Destination, Endian, Function, Operation, Terminator, Type, ValueId};
use woomy_ppc::Register;
pub mod compiler;
pub mod model;
pub mod validation;
use woomy_ir::runtime::{
    ImageContextVersions, Program, CAFE_IMAGE_VERSION, CAFE_LOADER_VERSION, CALL_MODEL_VERSION,
    EXECUTION_CONTEXT_VERSION, FUNCTION_SUMMARY_VERSION, GUEST_MEMORY_VERSION,
    RELOCATION_APPLIER_VERSION, RUNTIME_ABI_VERSION,
};
pub const CPP_BACKEND_VERSION: u32 = 4;
pub const NATIVE_ABI_VERSION: u32 = 2;
pub const NATIVE_VALIDATION_VERSION: u32 = 2;
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub function: String,
    pub block: Option<u32>,
    pub ppc_address: Option<u32>,
    pub operation: String,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub function: String,
    pub module_hash: String,
    pub ir_version: u32,
    pub ir_hash: String,
    pub backend_version: u32,
    pub native_abi_version: u32,
    pub call_model_version: u32,
    pub function_summary_version: u32,
    pub guest_memory_version: u32,
    pub runtime_abi_version: u32,
    #[serde(default)]
    pub image_context_versions: ImageContextVersions,
    pub runtime_target: String,
    pub status: String,
    pub diagnostics: Vec<Diagnostic>,
    pub source_file: Option<String>,
    pub source_hash: Option<String>,
    pub cache_key: String,
}
pub struct Generated {
    pub manifest: Manifest,
    pub source: Option<String>,
}
fn reg(r: Register) -> Option<String> {
    Some(match r {
        Register::Gpr(n) if n < 32 => format!("s.gpr[{n}]"),
        Register::Fpr(n) if n < 32 => format!("s.fpr[{n}]"),
        Register::Lr => "s.lr".into(),
        Register::Ctr => "s.ctr".into(),
        Register::Xer => "s.xer".into(),
        Register::Cr(n) if n < 8 => format!("s.cr[{n}]"),
        _ => return None,
    })
}
fn value(id: &ValueId) -> String {
    format!("v_{:08x}_{}", id.address, id.slot)
}
pub fn eligibility(f: &Function) -> Manifest {
    eligibility_inner(f, false)
}
fn eligibility_inner(f: &Function, linked: bool) -> Manifest {
    let ir_hash = hash(&serde_json::to_vec(f).expect("serializable IR"));
    let cache_key =
        hash(format!("{ir_hash}:{CPP_BACKEND_VERSION}:{NATIVE_ABI_VERSION}:{CALL_MODEL_VERSION}:{FUNCTION_SUMMARY_VERSION}:{GUEST_MEMORY_VERSION}:{RUNTIME_ABI_VERSION}:{CAFE_IMAGE_VERSION}:{CAFE_LOADER_VERSION}:{RELOCATION_APPLIER_VERSION}:{EXECUTION_CONTEXT_VERSION}").as_bytes());
    let mut m = Manifest {
        function: f.id.clone(),
        module_hash: f.module_hash.clone(),
        ir_version: f.ir_version,
        ir_hash,
        backend_version: CPP_BACKEND_VERSION,
        native_abi_version: NATIVE_ABI_VERSION,
        call_model_version: CALL_MODEL_VERSION,
        function_summary_version: FUNCTION_SUMMARY_VERSION,
        guest_memory_version: GUEST_MEMORY_VERSION,
        runtime_abi_version: RUNTIME_ABI_VERSION,
        image_context_versions: ImageContextVersions::CURRENT,
        runtime_target: "None".into(),
        status: "ELIGIBLE".into(),
        diagnostics: vec![],
        source_file: None,
        source_hash: None,
        cache_key,
    };
    let verification = woomy_ir::verify(f);
    if !verification.structural_valid {
        m.diagnostics.push(Diagnostic {
            function: f.id.clone(),
            block: None,
            ppc_address: None,
            operation: "Verifier".into(),
            reason: format!("INVALID IR: {:?}", verification.diagnostics),
        });
    } else {
        if (f.status != "LIFTED" && !linked)
            || f.limitations.iter().any(|s| s == "Source CFG truncated")
        {
            m.diagnostics.push(Diagnostic {
                function: f.id.clone(),
                block: None,
                ppc_address: None,
                operation: "Coverage".into(),
                reason: format!("CODEGEN_UNSUPPORTED: function coverage is {}", f.status),
            });
        }
        for b in &f.blocks {
            for i in &b.instructions {
                let supported = match &i.operation {
                    Operation::RelocationImmediate {
                        relocation,
                        signed,
                        shift,
                    } => relocation.immediate(*signed, *shift).is_some(),
                    Operation::Constant { .. } => {
                        i.result.as_ref().is_some_and(|v| v.ty == Type::I32)
                    }
                    Operation::ReadRegister { register }
                    | Operation::WriteRegister { register, .. } => reg(*register).is_some(),
                    Operation::Binary { .. }
                    | Operation::RotateMask32 { .. }
                    | Operation::Compare32 { .. }
                    | Operation::IntegerFlags32 { .. }
                    | Operation::BranchCondition { .. }
                    | Operation::Address32 { .. } => true,
                    Operation::Load {
                        width: 1 | 2 | 4 | 8,
                        endian: Endian::Big,
                        ..
                    }
                    | Operation::Store {
                        width: 1 | 2 | 4 | 8,
                        endian: Endian::Big,
                        ..
                    } => true,
                    Operation::Call { target, .. } => {
                        linked && !matches!(target, woomy_ir::CallTarget::Unknown)
                    }
                    _ => false,
                };
                if !supported {
                    m.diagnostics.push(Diagnostic {
                        function: f.id.clone(),
                        block: Some(b.id),
                        ppc_address: Some(i.source.address),
                        operation: format!("{:?}", i.operation),
                        reason: "CODEGEN_UNSUPPORTED: outside the validated scalar backend subset"
                            .into(),
                    });
                }
            }
            if !matches!(
                b.terminator,
                Terminator::Jump {
                    target: Destination::Block(_)
                } | Terminator::Return {
                    source: Register::Lr,
                    clear_low_bits: 2
                } | Terminator::ConditionalLrBranch {
                    not_taken: Destination::Block(_),
                    ..
                } | Terminator::Trap
                    | Terminator::Branch {
                        taken: Destination::Block(_),
                        not_taken: Destination::Block(_),
                        ..
                    }
            ) {
                m.diagnostics.push(Diagnostic {
                    function: f.id.clone(),
                    block: Some(b.id),
                    ppc_address: Some(b.terminator_source.address),
                    operation: format!("{:?}", b.terminator),
                    reason: "CODEGEN_UNSUPPORTED: unresolved or unsupported control flow".into(),
                });
            }
        }
    }
    if !m.diagnostics.is_empty() {
        m.status = "NOT_ELIGIBLE".into();
    }
    m
}
/// Source identities/strings are never interpolated into C++. Only verified
/// numeric IR and fixed runtime text reach the compiler.
pub fn generate(f: &Function) -> Generated {
    generate_inner(f, None)
}
pub fn generate_program(program: &Program) -> Result<Generated, String> {
    program.validate()?;
    if program.runtime_context.is_some() || !program.source_barriers.is_empty() {
        return Err("EXTERNAL_RUNTIME_NATIVE_BRIDGE_UNAVAILABLE_OR_SOURCE_BARRIER".into());
    }
    for f in &program.functions {
        let m = eligibility_inner(f, true);
        if m.status != "ELIGIBLE" {
            return Err(format!(
                "NON_EXECUTABLE_CALLEE {}: {:?}",
                f.id, m.diagnostics
            ));
        }
        for i in f.blocks.iter().flat_map(|b| &b.instructions) {
            if let Operation::Call {
                target: woomy_ir::CallTarget::Static { module, address },
                ..
            } = &i.operation
            {
                if program
                    .function(*address)
                    .is_none_or(|target| &target.module != module)
                {
                    return Err("UNREGISTERED_STATIC_CALLEE".into());
                }
            }
        }
    }
    Ok(generate_inner(
        program.function(program.entry).unwrap(),
        Some(program),
    ))
}
fn generate_inner(f: &Function, program: Option<&Program>) -> Generated {
    let mut manifest = eligibility_inner(f, program.is_some());
    if let Some(p) = program {
        manifest.ir_hash = hash(&serde_json::to_vec(p).expect("serializable program"));
        manifest.cache_key = hash(format!("{}:{CPP_BACKEND_VERSION}:{NATIVE_ABI_VERSION}:{CALL_MODEL_VERSION}:{FUNCTION_SUMMARY_VERSION}:{GUEST_MEMORY_VERSION}:{RUNTIME_ABI_VERSION}:{CAFE_IMAGE_VERSION}:{CAFE_LOADER_VERSION}:{RELOCATION_APPLIER_VERSION}:{EXECUTION_CONTEXT_VERSION}",manifest.ir_hash).as_bytes());
        manifest.runtime_target = p.provider.provider().query_capability().into();
    }
    if manifest.status != "ELIGIBLE" {
        return Generated {
            manifest,
            source: None,
        };
    }
    let mut out = String::from(include_str!("runtime.cpp"));
    writeln!(out,"\n// IR SHA-256: {}\n// Backend {CPP_BACKEND_VERSION}; ABI {NATIVE_ABI_VERSION}; RuntimeTarget::{}",manifest.ir_hash,manifest.runtime_target).unwrap();
    writeln!(out,"static Result run(State initial, uint32_t budget) {{\n Result r{{std::move(initial), {{}}, 3u, 0u, \"\"}};\n auto& s = r.state;").unwrap();
    writeln!(out," if (!bounded(s) || budget == 0u || budget > 65536u) return r;\n uint32_t block = {}u;\n for (;;) {{ switch(block) {{",f.entry).unwrap();
    let functions: Vec<&Function> =
        program.map_or_else(|| vec![f], |p| p.functions.iter().collect());
    // Registry data is numeric, checked and deterministic; never a guest-to-host pointer cast.
    if let Some(p) = program {
        // Insert runtime setup before the switch loop.
        let marker = format!(" uint32_t block = {}u;", f.entry);
        let mut setup = format!(
            " WoomyRuntime rt(s.lr & ~3u, {}u, {}u, {}u);\n",
            p.limits.calls, p.limits.depth, p.limits.trace
        );
        for fun in &p.functions {
            writeln!(setup, "rt.entries.push_back({}u);", fun.entry).unwrap();
            for b in &fun.blocks {
                writeln!(setup, "rt.blocks.push_back({}u);", b.id).unwrap();
                for src in &b.sources {
                    writeln!(
                        setup,
                        "rt.owners.push_back({{{}u,{}u}});",
                        src.address, fun.entry
                    )
                    .unwrap();
                }
            }
        }
        for (site, import) in &p.imports {
            writeln!(
                setup,
                "rt.imports.push_back({{{site}u,{}u}});",
                p.provider.provider().resolve_import(import).unwrap_or(0)
            )
            .unwrap();
        }
        out = out.replace(&marker, &format!("{setup}{marker}"));
    }
    for f in &functions {
        for b in &f.blocks {
            writeln!(out, "case {}u: {{", b.id).unwrap();
            let mut last = None;
            for i in &b.instructions {
                let pc = i.source.address;
                if last != Some(pc) {
                    trace(&mut out, pc, program.is_some());
                    last = Some(pc);
                }
                let expression = match &i.operation {
                    Operation::Call { target, .. } => {
                        let (target, kind) = match target {
                            woomy_ir::CallTarget::Static { address, .. } => (
                                format!("{address}u"),
                                woomy_ir::static_call_event_kind(&i.source),
                            ),
                            woomy_ir::CallTarget::Imported { .. } => ("0u".into(), 2),
                            woomy_ir::CallTarget::Indirect { source } => {
                                (format!("{} & ~3u", reg(*source).unwrap()), 1)
                            }
                            _ => unreachable!("eligible call"),
                        };
                        writeln!(
                            out,
                            "if (!rt.call(r, {pc}u, {target}, {kind}u, block)) return r; continue;"
                        )
                        .unwrap();
                        None
                    }
                    Operation::RelocationImmediate {
                        relocation,
                        signed,
                        shift,
                    } => Some(format!(
                        "{}u",
                        relocation
                            .immediate(*signed, *shift)
                            .expect("eligible local relocation")
                    )),
                    Operation::Compare32 {
                        lhs,
                        rhs,
                        xer,
                        signed,
                    } => Some(format!(
                        "compare32({}, {}, {}, {})",
                        value(lhs),
                        value(rhs),
                        value(xer),
                        signed
                    )),
                    Operation::IntegerFlags32 {
                        lhs,
                        rhs,
                        result,
                        xer,
                        subtract,
                        carry,
                        overflow,
                    } => Some(format!(
                        "flags32({}, {}, {}, {}, {}, {}, {})",
                        value(lhs),
                        value(rhs),
                        value(result),
                        value(xer),
                        subtract,
                        carry,
                        overflow
                    )),
                    Operation::BranchCondition {
                        cr_bit,
                        cr_expected,
                        decrement_ctr,
                        ctr_expected_zero,
                    } => Some(format!(
                        "branch_condition(s, {}u, {}, {}, {})",
                        cr_bit.unwrap_or(32),
                        cr_expected,
                        decrement_ctr,
                        ctr_expected_zero
                    )),
                    Operation::Constant { bits } => Some(format!("{bits}u")),
                    Operation::ReadRegister { register } => reg(*register),
                    Operation::WriteRegister { register, value: v } => {
                        let expression = if matches!(register, Register::Cr(_)) {
                            format!("uint8_t({} & 15u)", value(v))
                        } else {
                            value(v)
                        };
                        writeln!(out, "{} = {};", reg(*register).unwrap(), expression).unwrap();
                        None
                    }
                    Operation::Binary { operator, lhs, rhs } => {
                        let (a, b) = (value(lhs), value(rhs));
                        Some(match operator {
                            Binary::MultiplyLow => {
                                format!("uint32_t(uint64_t({a}) * uint64_t({b}))")
                            }
                            Binary::MultiplyHighUnsigned => {
                                format!("uint32_t((uint64_t({a}) * uint64_t({b})) >> 32u)")
                            }
                            Binary::MultiplyHighSigned => format!("mul_high_signed({a},{b})"),
                            Binary::ShiftLeft => {
                                format!("(({b} & 32u) != 0u ? 0u : uint32_t({a} << ({b} & 31u)))")
                            }
                            Binary::ShiftRight => {
                                format!("(({b} & 32u) != 0u ? 0u : uint32_t({a} >> ({b} & 31u)))")
                            }
                            _ => format!(
                                "uint32_t({a} {} {b})",
                                match operator {
                                    Binary::AddWrapping => "+",
                                    Binary::SubWrapping => "-",
                                    Binary::And => "&",
                                    Binary::Or => "|",
                                    Binary::Xor => "^",
                                    _ => unreachable!(),
                                }
                            ),
                        })
                    }
                    Operation::RotateMask32 {
                        value: v,
                        shift,
                        begin,
                        end,
                    } => Some(format!(
                        "rotate32({}, {}u) & {}u",
                        value(v),
                        shift,
                        woomy_ppc::dataflow::mask32(*begin as u32, *end as u32)
                    )),
                    Operation::Address32 { value: v } => Some(value(v)),
                    Operation::Load { address, width, .. } => {
                        writeln!(out,"uint64_t loaded_{}_{} = 0u; if (!load(s, {}, loaded_{}_{}, {width}u)) return fault(std::move(r), {pc}u, memory_fault);",i.id.address,i.id.slot,value(address),i.id.address,i.id.slot).unwrap();
                        Some(format!("loaded_{}_{}", i.id.address, i.id.slot))
                    }
                    Operation::Store {
                        address,
                        value: v,
                        width,
                        ..
                    } => {
                        writeln!(
                        out,
                        "if (!store(s, {}, {}, {width}u)) return fault(std::move(r), {pc}u, memory_fault);",
                        value(address),
                        value(v)
                    )
                    .unwrap();
                        None
                    }
                    _ => unreachable!("eligibility whitelist"),
                };
                if let Some(v) = &i.result {
                    writeln!(
                        out,
                        "const {} {} = {};",
                        if matches!(v.ty, Type::I64 | Type::F64) {
                            "uint64_t"
                        } else {
                            "uint32_t"
                        },
                        value(&v.id),
                        expression.unwrap()
                    )
                    .unwrap();
                }
            }
            let pc = b.terminator_source.address;
            if last != Some(pc) {
                trace(&mut out, pc, program.is_some());
            }
            if let Terminator::ConditionalLrBranch {
                condition,
                not_taken: Destination::Block(next),
            } = &b.terminator
            {
                writeln!(
                    out,
                    "if ({} == 0u) {{ block = {next}u; break; }}",
                    value(condition)
                )
                .unwrap();
            }
            match &b.terminator {
                Terminator::Branch {
                    condition,
                    taken: Destination::Block(taken),
                    not_taken: Destination::Block(not_taken),
                } => {
                    writeln!(
                        out,
                        "block = {} != 0u ? {taken}u : {not_taken}u; break;",
                        value(condition)
                    )
                    .unwrap();
                }
                Terminator::Jump {
                    target: Destination::Block(target),
                } => {
                    writeln!(out, "block = {target}u; break;").unwrap();
                }
                Terminator::Return { .. } | Terminator::ConditionalLrBranch { .. }
                    if program.is_some() =>
                {
                    writeln!(out, "if (!rt.returning(r, {pc}u, block)) return r; break;").unwrap();
                }
                Terminator::Return { .. } | Terminator::ConditionalLrBranch { .. } => {
                    writeln!(
                        out,
                        "const uint32_t target = s.lr & ~3u;\nswitch(target) {{"
                    )
                    .unwrap();
                    for t in &f.blocks {
                        writeln!(out, "case {}u: block = target; break;", t.id).unwrap();
                    }
                    writeln!(out, "default:").unwrap();
                    let inner: Vec<_> = f
                        .blocks
                        .iter()
                        .flat_map(|b| b.sources.iter().map(|s| s.address))
                        .filter(|pc| !f.blocks.iter().any(|b| b.id == *pc))
                        .collect();
                    if !inner.is_empty() {
                        writeln!(out,"if ({}) return fault(std::move(r), {pc}u, \"LR targets an instruction without a proven block boundary\");",inner.iter().map(|pc|format!("target == {pc}u")).collect::<Vec<_>>().join(" || ")).unwrap();
                    }
                    writeln!(out, "r.exit = 0u; r.address = target; return r;\n}} break;").unwrap();
                }
                Terminator::Trap => {
                    writeln!(out, "r.exit=1u; r.address={pc}u; return r;").unwrap();
                }
                _ => unreachable!("eligibility whitelist"),
            }
            writeln!(out, "}}").unwrap();
        }
    }
    writeln!(
        out,
        "default: return fault(std::move(r), block, \"Missing block\");\n}} }} }}"
    )
    .unwrap();
    if program.is_some() {
        out = out.replace("memory_fault);", "memory_fault, 4u);");
    }
    out.push_str(include_str!("harness.cpp"));
    manifest.status = "GENERATED".into();
    manifest.source_hash = Some(hash(out.as_bytes()));
    Generated {
        manifest,
        source: Some(out),
    }
}
fn trace(out: &mut String, pc: u32, runtime: bool) {
    if runtime {
        writeln!(out, "if (!rt.fetch(r, {pc}u)) return r;").unwrap();
    }
    writeln!(
        out,
        "if (r.trace.size() >= budget) return r; r.trace.push_back({pc}u);"
    )
    .unwrap();
}
