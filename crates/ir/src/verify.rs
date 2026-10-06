use super::*;

fn error(
    out: &mut Vec<Diagnostic>,
    code: &str,
    block: Option<u32>,
    instruction: Option<ValueId>,
    message: &str,
) {
    if out.len() < 1024 {
        out.push(Diagnostic {
            code: code.into(),
            block,
            instruction,
            message: message.into(),
        });
    }
}
fn valid_register(r: Register) -> bool {
    match r {
        Register::Gpr(n) | Register::Fpr(n) => n < 32,
        Register::Cr(n) => n < 8,
        _ => true,
    }
}
fn valid_effects(e: &Effects) -> bool {
    e.uses.iter().chain(&e.defs).all(|r| valid_register(*r))
}
fn width_type(ty: Type, width: u8) -> bool {
    matches!(
        (ty, width),
        (Type::I8, 1)
            | (Type::I16, 2)
            | (Type::I32, 1 | 2 | 4)
            | (Type::F32, 4)
            | (Type::I64 | Type::F64, 8)
    )
}
fn valid_source(f: &Function, p: &Provenance, sources: &BTreeMap<u32, &Source>) -> bool {
    p.module == f.module
        && p.decoder_version == f.decoder_version
        && p.analysis_version == f.analysis_version
        && sources.get(&p.address).is_some_and(|s| {
            s.raw == p.raw
                && p.kind
                    == if s.status == "KNOWN" {
                        "REAL"
                    } else {
                        "PARTIAL"
                    }
        })
}
pub fn verify(f: &Function) -> Verification {
    let mut diagnostics = Vec::new();
    let mut unknown = 0usize;
    if f.ir_version != IR_VERSION
        || f.decoder_version == 0
        || f.cfg_version == 0
        || f.analysis_version == 0
    {
        error(
            &mut diagnostics,
            "VERSION",
            None,
            None,
            "Unsupported or missing IR/source versions",
        );
    }
    if f.id.is_empty()
        || f.module.is_empty()
        || f.module_hash.len() != 64
        || !f.module_hash.bytes().all(|c| c.is_ascii_hexdigit())
    {
        error(
            &mut diagnostics,
            "IDENTITY",
            None,
            None,
            "Stable function/module identity and SHA-256 are required",
        );
    }
    if !["LIFTED", "PARTIALLY_LIFTED"].contains(&f.status.as_str()) {
        error(
            &mut diagnostics,
            "LIFT_STATUS",
            None,
            None,
            "Lifter did not produce a complete structural artifact",
        );
    }
    if f.blocks.len() > 4096
        || f.blocks.iter().map(|b| b.instructions.len()).sum::<usize>() > MAX_OPERATIONS
    {
        error(
            &mut diagnostics,
            "BUDGET",
            None,
            None,
            "IR exceeds verification budget",
        );
        return report(diagnostics, unknown);
    }
    let ids: BTreeSet<_> = f.blocks.iter().map(|b| b.id).collect();
    if ids.len() != f.blocks.len() {
        error(
            &mut diagnostics,
            "DUPLICATE_BLOCK",
            None,
            None,
            "Block IDs must be unique",
        );
    }
    if !ids.contains(&f.entry) {
        error(
            &mut diagnostics,
            "ENTRY",
            None,
            None,
            "Entry block is missing",
        );
    }
    let mut all_instructions = BTreeSet::new();
    let mut all_sources = BTreeSet::new();
    for b in &f.blocks {
        let mut bad = |code: &str, instruction: Option<ValueId>, message: &str| {
            error(&mut diagnostics, code, Some(b.id), instruction, message)
        };
        let sources: BTreeMap<_, _> = b.sources.iter().map(|s| (s.address, s)).collect();
        if b.id & 3 != 0
            || b.sources.first().is_none_or(|s| s.address != b.id)
            || sources.len() != b.sources.len()
            || b.sources
                .windows(2)
                .any(|s| s[0].address.checked_add(4) != Some(s[1].address))
            || b.sources.iter().any(|s| !all_sources.insert(s.address))
        {
            bad(
                "SOURCE_RANGE",
                None,
                "Blocks must cover unique contiguous aligned source instructions",
            );
        }
        if !valid_source(f, &b.terminator_source, &sources)
            || b.sources
                .last()
                .is_none_or(|s| s.address != b.terminator_source.address)
        {
            bad(
                "PROVENANCE",
                None,
                "Terminator must refer to the block's last PPC instruction",
            );
        }
        let mut values: BTreeMap<ValueId, Type> = BTreeMap::new();
        let mut represented = BTreeSet::new();
        let mut next_slots = BTreeMap::<u32, u16>::new();
        let mut last_address = b.id;
        for i in &b.instructions {
            let at = Some(i.id);
            represented.insert(i.source.address);
            let slot = next_slots.entry(i.source.address).or_default();
            if i.id.address != i.source.address
                || i.id.slot != *slot
                || !all_instructions.insert(i.id)
                || i.source.address < last_address
            {
                bad(
                    "INSTRUCTION_ID",
                    at,
                    "Instruction IDs must be unique, ordered, and derived from source address/slot",
                );
            }
            *slot = slot.saturating_add(1);
            last_address = i.source.address;
            if !valid_source(f, &i.source, &sources) {
                bad(
                    "PROVENANCE",
                    at,
                    "Instruction source/module/raw word/version mismatch",
                );
            }
            if i.result.as_ref().is_some_and(|v| v.id != i.id) {
                bad(
                    "VALUE_ID",
                    at,
                    "Result ID must match its defining instruction",
                );
            }
            let ty = i.result.as_ref().map(|v| v.ty);
            let mut needs = Vec::new();
            let mut expected = None;
            match &i.operation {
                Operation::RelocationImmediate {
                    relocation: r,
                    shift,
                    ..
                } => {
                    expected = Some(Type::I32);
                    if ![4, 5, 6].contains(&r.kind)
                        || ![0, 16].contains(shift)
                        || r.module != f.module
                        || i.source.address.checked_add(2) != Some(r.place)
                    {
                        bad(
                            "RELOCATION",
                            at,
                            "Invalid halfword relocation provenance or shift",
                        );
                    }
                    match (&r.target, r.local_address) {
                        (woomy_ppc::dataflow::Target::Address { module, .. }, Some(_))
                            if module == &f.module => {}
                        (
                            woomy_ppc::dataflow::Target::ImportedSymbol {
                                library,
                                symbol,
                                addend,
                                ..
                            },
                            None,
                        ) if !library.is_empty() && !symbol.is_empty() && *addend == r.addend => {
                            unknown += 1;
                        }
                        _ => bad(
                            "RELOCATION",
                            at,
                            "Local addresses require local identity; imports remain symbolic",
                        ),
                    }
                }
                Operation::Compare32 { lhs, rhs, xer, .. } => {
                    expected = Some(Type::I32);
                    needs.extend([(*lhs, Type::I32), (*rhs, Type::I32), (*xer, Type::I32)]);
                }
                Operation::IntegerFlags32 {
                    lhs,
                    rhs,
                    result,
                    xer,
                    ..
                } => {
                    expected = Some(Type::I32);
                    needs.extend([
                        (*lhs, Type::I32),
                        (*rhs, Type::I32),
                        (*result, Type::I32),
                        (*xer, Type::I32),
                    ]);
                }
                Operation::BranchCondition { cr_bit, .. } => {
                    expected = Some(Type::Bool);
                    if cr_bit.is_some_and(|bit| bit >= 32) {
                        bad("CR_BIT", at, "CR bit must be in 0..32");
                    }
                }
                Operation::Constant { bits } => {
                    expected = ty;
                    let valid = match ty {
                        Some(Type::I8) => *bits <= u8::MAX as u64,
                        Some(Type::I16) => *bits <= u16::MAX as u64,
                        Some(Type::I32) => *bits <= u32::MAX as u64,
                        Some(Type::I64) => true,
                        Some(Type::Bool) => *bits <= 1,
                        _ => false,
                    };
                    if !valid {
                        bad(
                            "CONSTANT_TYPE",
                            at,
                            "Constant does not fit an explicit integer/boolean type",
                        );
                    }
                }
                Operation::ReadRegister { register } => {
                    expected = Some(register_type(*register));
                    if !valid_register(*register) {
                        bad("REGISTER", at, "Invalid register index");
                    }
                }
                Operation::WriteRegister { register, value } => {
                    needs.push((*value, register_type(*register)));
                    if !valid_register(*register) {
                        bad("REGISTER", at, "Invalid register index");
                    }
                }
                Operation::Binary { lhs, rhs, .. } => {
                    expected = Some(Type::I32);
                    needs.extend([(*lhs, Type::I32), (*rhs, Type::I32)]);
                }
                Operation::RotateMask32 {
                    value,
                    shift,
                    begin,
                    end,
                } => {
                    expected = Some(Type::I32);
                    needs.push((*value, Type::I32));
                    if [shift, begin, end].iter().any(|n| **n > 31) {
                        bad("ROTATE", at, "Rotate/mask fields must fit five bits");
                    }
                }
                Operation::Address32 { value } => {
                    expected = Some(Type::Ptr);
                    needs.push((*value, Type::I32));
                }
                Operation::Load {
                    address,
                    width,
                    endian,
                    may_trap,
                } => {
                    expected = ty;
                    needs.push((*address, Type::Ptr));
                    if !ty.is_some_and(|t| width_type(t, *width))
                        || *endian != Endian::Big
                        || !may_trap
                    {
                        bad("MEMORY",at,"Load requires explicit big endian, width/type agreement and possible guest faults");
                    }
                }
                Operation::Store {
                    address,
                    value,
                    width,
                    endian,
                    may_trap,
                } => {
                    needs.push((*address, Type::Ptr));
                    if let Some(t) = values.get(value).copied() {
                        needs.push((*value, t));
                        if !width_type(t, *width) {
                            bad("MEMORY", at, "Store width does not match its value type");
                        }
                    } else {
                        bad(
                            "VALUE_REFERENCE",
                            at,
                            "Store value is missing or used before definition",
                        );
                    }
                    if *endian != Endian::Big || !may_trap {
                        bad(
                            "MEMORY",
                            at,
                            "Store requires big endian and possible guest faults",
                        );
                    }
                }
                Operation::Call {
                    target,
                    link_value,
                    effects,
                    signature,
                } => {
                    unknown += 1;
                    let valid = match target {
                        CallTarget::Static { module, address } => {
                            !module.is_empty() && address & 3 == 0
                        }
                        CallTarget::Imported { library, symbol } => {
                            !library.is_empty() && !symbol.is_empty()
                        }
                        CallTarget::Indirect { source } => {
                            matches!(source, Register::Ctr | Register::Lr)
                        }
                        CallTarget::Unknown => true,
                    };
                    if b.instructions.last().is_none_or(|last| last.id != i.id)
                        || i.source.address != b.terminator_source.address
                        || !matches!(b.terminator,Terminator::Jump { target: Destination::Block(address) | Destination::External(address) } if address==*link_value)
                    {
                        bad(
                            "CALL_CONTINUATION",
                            at,
                            "Guest call must end the block with an explicit CIA+4 continuation",
                        );
                    }
                    if !valid
                        || *link_value != i.source.address.wrapping_add(4)
                        || *signature != Type::Unknown
                        || !valid_effects(effects)
                        || effects.defs != woomy_ppc::all_registers()
                        || effects.uses != woomy_ppc::all_registers()
                        || effects.memory != MemoryEffect::ReadWrite
                    {
                        bad("CALL",at,"Call must retain target identity, unknown ABI and complete state/memory effects");
                    }
                }
                Operation::UnknownInstruction {
                    opcode,
                    reason,
                    effects,
                } => {
                    unknown += 1;
                    if opcode.is_empty() || reason.is_empty() || !valid_effects(effects) {
                        bad(
                            "UNKNOWN_EFFECT",
                            at,
                            "Unknown instruction requires reason and valid explicit effects",
                        );
                    }
                }
                Operation::UnknownSideEffect { reason, effects } => {
                    unknown += 1;
                    if reason.is_empty() || !valid_effects(effects) {
                        bad(
                            "UNKNOWN_EFFECT",
                            at,
                            "Unknown side effects require explicit reason",
                        );
                    }
                }
                Operation::UnknownValue { reason } => {
                    unknown += 1;
                    expected = ty;
                    if reason.is_empty() || ty.is_none() {
                        bad(
                            "UNKNOWN_VALUE",
                            at,
                            "Unknown value requires a type and reason",
                        );
                    }
                }
                Operation::OpaqueMemoryEffect { reason } => {
                    unknown += 1;
                    if reason.is_empty() {
                        bad("UNKNOWN_MEMORY", at, "Opaque memory requires a reason");
                    }
                }
            }
            if ty != expected {
                bad(
                    "RESULT_TYPE",
                    at,
                    "Operation result type is missing or incompatible",
                );
            }
            for (value, wanted) in needs {
                match values.get(&value) {
                    None => bad(
                        "VALUE_REFERENCE",
                        at,
                        "Value missing, cross-block, or used before definition",
                    ),
                    Some(actual) if *actual != wanted => {
                        bad("TYPE", at, "Operand type incompatible with operation")
                    }
                    _ => {}
                }
            }
            if let Some(v) = &i.result {
                if values.insert(v.id, v.ty).is_some() {
                    bad("DUPLICATE_VALUE", at, "Value has multiple definitions");
                }
            }
        }
        for s in &b.sources {
            if !represented.contains(&s.address) && s.address != b.terminator_source.address {
                bad(
                    "DROPPED_SOURCE",
                    None,
                    "PPC instruction has no IR representation",
                );
            }
            if ["UNKNOWN", "INVALID"].contains(&s.status.as_str())
                && !b.instructions.iter().any(|i| {
                    i.source.address == s.address
                        && matches!(i.operation, Operation::UnknownInstruction { .. })
                })
            {
                bad(
                    "DROPPED_UNKNOWN",
                    None,
                    "Unknown PPC must remain an explicit instruction",
                );
            }
        }
        for s in &b.successors {
            match s.destination {
                Destination::Block(target) if !ids.contains(&target) => {
                    bad("BRANCH_TARGET", None, "CFG points to a missing block")
                }
                Destination::External(a) if a & 3 != 0 => {
                    bad("BRANCH_TARGET", None, "External target is not aligned")
                }
                Destination::External(_) | Destination::Unknown => unknown += 1,
                _ => {}
            }
        }
        match &b.terminator {
            Terminator::ConditionalLrBranch {
                condition,
                not_taken,
            } => {
                if values.get(condition) != Some(&Type::Bool) {
                    bad(
                        "BRANCH_CONDITION",
                        None,
                        "Conditional LR branch requires a boolean",
                    );
                }
                let targets: BTreeSet<_> =
                    b.successors.iter().map(|s| s.destination.clone()).collect();
                if targets != BTreeSet::from([Destination::Unknown, not_taken.clone()]) {
                    bad("CFG", None, "Conditional LR branch successors disagree");
                }
                let raw = b.terminator_source.raw;
                if raw >> 26 != 19 || (raw >> 1) & 1023 != 16 || raw & 0xf801 != 0 {
                    bad(
                        "BCLR_FORM",
                        None,
                        "Only non-linking documented bclr form is supported",
                    );
                }
            }
            Terminator::Branch {
                condition,
                taken,
                not_taken,
            } => {
                if values.get(condition) != Some(&Type::Bool) {
                    bad(
                        "BRANCH_CONDITION",
                        None,
                        "Branch requires a defined boolean in this block",
                    );
                }
                let targets: BTreeSet<_> =
                    b.successors.iter().map(|s| s.destination.clone()).collect();
                if targets != BTreeSet::from([taken.clone(), not_taken.clone()]) {
                    bad("CFG", None, "Conditional branch and successors disagree");
                }
            }
            Terminator::Jump { target } => {
                if b.successors.len() != 1 || b.successors[0].destination != *target {
                    bad("CFG", None, "Jump and successor list disagree");
                }
            }
            Terminator::Return {
                source,
                clear_low_bits,
            } => {
                if *source != Register::Lr || *clear_low_bits != 2 || !b.successors.is_empty() {
                    bad(
                        "RETURN",
                        None,
                        "Return must use LR with PPC alignment and no intra-function successor",
                    );
                }
            }
            Terminator::Trap => {
                if !b.successors.is_empty() {
                    bad(
                        "TRAP",
                        None,
                        "Unconditional trap cannot have a continuation",
                    );
                }
            }
            Terminator::UnresolvedIndirectBranch {
                source,
                clear_low_bits,
            } => {
                unknown += 1;
                if !matches!(source, Register::Lr | Register::Ctr) || *clear_low_bits != 2 {
                    bad(
                        "INDIRECT",
                        None,
                        "Indirect branch requires LR/CTR and PPC alignment",
                    );
                }
            }
            Terminator::OpaqueTerminator { reason } => {
                unknown += 1;
                if reason.is_empty() {
                    bad(
                        "OPAQUE_TERMINATOR",
                        None,
                        "Unknown terminator must explain missing semantics",
                    );
                }
            }
        }
    }
    if unknown > 0 && f.status == "LIFTED" {
        error(
            &mut diagnostics,
            "COVERAGE",
            None,
            None,
            "Unknown effects cannot be marked fully lifted",
        );
    }
    report(diagnostics, unknown)
}
fn report(diagnostics: Vec<Diagnostic>, unknown_operations: usize) -> Verification {
    let structural_valid = diagnostics.is_empty();
    Verification {
        status: if structural_valid {
            "STRUCTURALLY_VALID"
        } else {
            "INVALID"
        }
        .into(),
        ir_version: IR_VERSION,
        structural_valid,
        executable_semantics_validated: false,
        unknown_operations,
        diagnostics,
    }
}
