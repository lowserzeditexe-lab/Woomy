//! Independent synthetic image/relocation/context/snapshot cases; no game bytes.
use std::collections::{BTreeMap, BTreeSet};
use woomy_binary::{Import, Module, Relocation, Section, Symbol};
use woomy_core::{
    cafe::CafeGuestImage,
    context::{ExecutionContextBuilder, InputProvenance},
    function_validation::register_only_program,
    snapshot::{Origin, ReferenceSnapshot, Registers, SnapshotRegion, SNAPSHOT_VERSION},
};
use woomy_ir::runtime::ImageContextVersions;
fn section(index: u32, address: u32, size: u32, kind: u32, flags: u32) -> Section {
    Section {
        index,
        name: format!("section{index}"),
        kind,
        flags,
        address,
        offset: 0,
        file_size: size,
        size,
        link: 0,
        info: 0,
        alignment: 4,
        entry_size: 0,
        data: if kind == 8 {
            vec![]
        } else {
            vec![0; size as usize]
        },
    }
}
fn module() -> Module {
    let mut text = section(1, 0x1000, 32, 1, 6);
    text.data[..8].copy_from_slice(
        &[0x38630001u32, 0x4e800020]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect::<Vec<_>>(),
    );
    Module {
        entry_point: 0x1000,
        sections: vec![
            section(0, 0, 0, 0, 0),
            text,
            section(2, 0x2000, 64, 1, 3),
            section(3, 0x8000, 64, 8, 3),
            section(4, 0x3000, 16, 1, 2),
        ],
        imports: vec![],
        exports: vec![],
        symbols: vec![Symbol {
            table: 5,
            index: 1,
            name: "local".into(),
            value: 0x8000,
            size: 4,
            info: 0x11,
            section: 3,
        }],
        relocations: vec![],
        provenance: "SYNTHETIC",
    }
}
fn load(m: &Module) -> woomy_core::Result<CafeGuestImage> {
    CafeGuestImage::load("synthetic", &"a".repeat(64), "RPX", m, 1024)
}
fn reloc(kind: u8, place: u32, addend: i32) -> Relocation {
    Relocation {
        section: 6,
        target_section: 2,
        symbol_table: 5,
        offset: place,
        kind,
        symbol: 1,
        addend: Some(addend),
    }
}
fn function(m: &Module) -> woomy_ir::Function {
    let words = BTreeMap::from([
        (0x1000, woomy_ppc::decode(0x1000, m.word(0x1000).unwrap())),
        (0x1004, woomy_ppc::decode(0x1004, m.word(0x1004).unwrap())),
    ]);
    let cfg = woomy_ppc::build_cfg(0x1000, &words, &BTreeSet::from([0x1000]), 100);
    woomy_ir::lift(
        "synthetic.1000",
        "synthetic",
        &"a".repeat(64),
        0x1000,
        &cfg,
        &words,
    )
}
#[test]
fn image_maps_only_program_storage_with_permissions_zero_fill_and_stable_identity() {
    let m = module();
    let image = load(&m).unwrap();
    image.verify().unwrap();
    assert_eq!(image.regions().len(), 4);
    assert_eq!(image.status()["zero_filled_bytes"], 64);
    assert_eq!(image.memory().regions[0].permissions, 5);
    assert_eq!(image.memory().regions[1].permissions, 3);
    assert_eq!(image.memory().regions[3].permissions, 1);
    assert!(image.memory().regions[2].bytes.iter().all(|b| *b == 0));
    assert_eq!(image.hash(), load(&m).unwrap().hash());
    assert_eq!(image.slice(0x1000, 4).unwrap().bytes, [0x38, 0x63, 0, 1]);
    assert!(!image.status()["execution_ready"].as_bool().unwrap());
}
#[test]
fn local_relocation_known_answers_include_negative_addend_and_ha_carry() {
    for (kind, addend, expected) in [
        (1, 4, vec![0, 0, 0x80, 4]),
        (4, 4, vec![0x80, 4]),
        (5, 4, vec![0, 0]),
        (6, 0, vec![0, 1]),
        (6, -1, vec![0, 0]),
    ] {
        let mut m = module();
        m.sections[3].address = 0x7ff0;
        m.sections[3].alignment = 16;
        m.relocations.push(reloc(kind, 0x2000, addend));
        let image = load(&m).unwrap();
        let r = &image.relocations()[0];
        assert_eq!(r.state, "APPLIED_PROVEN_LOCAL");
        assert_eq!(r.relocated, expected);
        assert_eq!(&m.sections[2].data[..4], &[0; 4]);
        assert_eq!(r.target, Some((0x8000i64 + addend as i64) as u32));
    }
}
#[test]
fn rel24_independent_branch_expected_encoding_and_rejection_cases() {
    let mut m = module();
    m.symbols[0].section = 1;
    m.symbols[0].value = 0x1000;
    m.sections[1].data[..4].copy_from_slice(&0x48000001u32.to_be_bytes());
    let mut r = reloc(10, 0x1000, 4);
    r.target_section = 1;
    m.relocations.push(r);
    assert_eq!(
        load(&m).unwrap().relocations()[0].relocated,
        [0x48, 0, 0, 5]
    );
    m.relocations[0].addend = Some(2);
    assert_eq!(load(&m).unwrap().relocations()[0].state, "INVALID");
    m.relocations[0].addend = Some(0x02000000);
    m.sections.push(section(7, 0x02001000, 4, 1, 6));
    assert_eq!(load(&m).unwrap().relocations()[0].state, "INVALID");
    m.relocations[0].addend = Some(4);
    m.sections[1].data[3] = 3;
    assert_eq!(load(&m).unwrap().relocations()[0].state, "INVALID");
}
#[test]
fn import_deferred_unsupported_and_ambiguous_are_never_concrete() {
    let mut m = module();
    m.imports.push(Import {
        section: 9,
        library: "coreinit".into(),
        count: 1,
        signature: 0,
        code: true,
    });
    m.symbols[0].section = 9;
    m.symbols[0].name = "UnknownService".into();
    m.relocations.push(reloc(1, 0x2000, 0));
    let image = load(&m).unwrap();
    let r = &image.relocations()[0];
    assert_eq!(r.state, "DEFERRED_IMPORT");
    assert_eq!(r.target, None);
    assert!(r.relocated.is_empty());
    assert!(image.slice(0x2000, 4).is_err());
    m.imports.clear();
    m.symbols[0].section = 3;
    m.relocations[0].kind = 26;
    assert_eq!(load(&m).unwrap().relocations()[0].state, "UNSUPPORTED");
    m.relocations[0].kind = 1;
    m.relocations.push(reloc(4, 0x2002, 0));
    let image = load(&m).unwrap();
    assert!(image.relocations().iter().all(|r| r.state == "AMBIGUOUS"));
    assert_eq!(image.memory().regions[1].bytes, m.sections[2].data);
    m.relocations.pop();
    m.symbols.push(Symbol {
        table: 5,
        index: 1,
        name: "duplicate".into(),
        value: 0x8000,
        size: 0,
        info: 0,
        section: 3,
    });
    assert_eq!(load(&m).unwrap().relocations()[0].state, "AMBIGUOUS");
}
#[test]
fn invalid_layouts_and_relocation_ranges_fail_without_panics() {
    type MutateModule = Box<dyn Fn(&mut Module)>;
    let invalid: Vec<MutateModule> = vec![
        Box::new(|m| m.sections[2].address = 0x1004),
        Box::new(|m| m.sections[2].address = u32::MAX - 3),
        Box::new(|m| m.sections[2].alignment = 3),
        Box::new(|m| m.sections[2].address = 0x2002),
        Box::new(|m| m.sections[2].size = 3),
        Box::new(|m| m.sections[2].offset = 1024),
        Box::new(|m| m.entry_point = 0x2000),
        Box::new(|m| m.sections[3].data = vec![1]),
    ];
    for mutate in invalid {
        let mut m = module();
        mutate(&mut m);
        assert!(load(&m).is_err());
    }
    for place in [0x1ffc, 0x203e, 0x2040, u32::MAX] {
        let mut m = module();
        m.relocations.push(reloc(1, place, 0));
        assert_eq!(load(&m).unwrap().relocations()[0].state, "INVALID");
    }
    let mut m = module();
    m.symbols[0].value = 0xfffffff0;
    m.sections[3].address = 0xfffffff0;
    m.sections[3].size = 16;
    m.sections[3].file_size = 16;
    m.relocations.push(reloc(1, 0x2000, 32));
    assert_eq!(load(&m).unwrap().relocations()[0].state, "INVALID");
}
#[test]
fn register_requirements_are_inputs_and_synthetic_program_is_separate() {
    let m = module();
    let image = load(&m).unwrap();
    let f = function(&m);
    let context = ExecutionContextBuilder::new(&image).analyze(&f, true);
    assert!(context.register_only_synthetic_eligible);
    assert_eq!(
        context.required_registers,
        BTreeSet::from([woomy_ppc::Register::Gpr(3), woomy_ppc::Register::Lr])
    );
    assert!(!context.actual_game_execution_ready);
    let (mut program, raw, memory) = register_only_program(&image, f).unwrap();
    let mut state = woomy_ir::execution::Machine::new(0, vec![]);
    state.gpr[3] = u32::MAX;
    state.lr = 0xffff0000;
    state.guest_memory = memory;
    let ppc = woomy_ir::execution::execute_ppc_program(&program, &raw, state.clone(), 20);
    let ir = woomy_ir::execution::execute_ir_program(&program, state, 20);
    assert_eq!(ppc, ir);
    assert_eq!(ppc.state.gpr[3], 0);
    program.image.as_mut().unwrap().versions.loader = 0;
    assert!(program.validate().is_err());
    program.image.as_mut().unwrap().versions = ImageContextVersions::CURRENT;
    program.image.as_mut().unwrap().versions.context = 0;
    assert!(program.validate().is_err());
}
#[test]
fn memory_and_unknown_import_requirements_prevent_synthetic_promotion() {
    let mut m = module();
    m.sections[1].data[..4].copy_from_slice(&0x80630000u32.to_be_bytes());
    let image = load(&m).unwrap();
    let f = function(&m);
    let c = ExecutionContextBuilder::new(&image).analyze(&f, false);
    assert_eq!(c.memory_accesses, 1);
    assert!(c
        .requirements
        .iter()
        .any(|r| r.expression.contains("CALLER_PROVIDED_POINTER")));
    assert!(register_only_program(&image, f).is_err());
    m.sections[1].data[..4].copy_from_slice(&0x48000005u32.to_be_bytes());
    let mut f = function(&m);
    let source = f.blocks[0].terminator_source.clone();
    f.blocks[0].instructions.push(woomy_ir::Instruction {
        id: woomy_ir::ValueId {
            address: 0x1000,
            slot: 200,
        },
        result: None,
        operation: woomy_ir::Operation::UnknownValue {
            reason: "missing import".into(),
        },
        source,
    });
    assert!(
        !ExecutionContextBuilder::new(&image)
            .analyze(&f, false)
            .register_only_synthetic_eligible
    );
}
fn snapshot(image: &CafeGuestImage) -> ReferenceSnapshot {
    let r = image.slice(0x1000, 8).unwrap();
    ReferenceSnapshot {
        format_version: SNAPSHOT_VERSION,
        versions: ImageContextVersions::CURRENT,
        module: "synthetic".into(),
        module_sha256: "a".repeat(64),
        image_hash: image.hash().into(),
        function: "synthetic.1000".into(),
        function_entry: 0x1000,
        cia: 0x1000,
        origin: Origin::Synthetic,
        source: "unit fixture".into(),
        registers: Registers {
            gpr: [0; 32],
            fpr: [0; 32],
            cr: [0; 8],
            lr: 0xffff0000,
            ctr: 0,
            xer: 0,
            fpscr: 0,
        },
        register_provenance: InputProvenance::ExplicitTestInput,
        regions: vec![SnapshotRegion {
            base: r.base,
            permissions: r.permissions,
            kind: r.kind,
            source: "fixture image".into(),
            sha256: woomy_core::hash_bytes(&r.bytes),
            bytes: r.bytes,
            provenance: InputProvenance::ExplicitTestInput,
        }],
        expected_exit_sha256: None,
    }
}
#[test]
fn snapshots_check_origin_identity_bytes_permissions_bounds_and_versions() {
    let m = module();
    let image = load(&m).unwrap();
    let s = snapshot(&image);
    s.validate_for(&image, &function(&m)).unwrap();
    let bytes = serde_json::to_vec(&s).unwrap();
    ReferenceSnapshot::parse(&bytes).unwrap();
    assert_eq!(
        s.metadata()["validation_domain"],
        "GAME_FUNCTION_SYNTHETIC_DOMAIN"
    );
    let mut bad = s.clone();
    bad.module_sha256 = "b".repeat(64);
    assert!(bad.validate_for(&image, &function(&m)).is_err());
    let mut bad = s.clone();
    bad.regions.push(bad.regions[0].clone());
    assert!(bad.validate_structure().is_err());
    let mut bad = s.clone();
    bad.regions[0].permissions = 8;
    assert!(bad.validate_structure().is_err());
    let mut bad = s.clone();
    bad.regions[0].bytes[0] ^= 1;
    assert!(bad.validate_structure().is_err());
    bad.regions[0].sha256 = woomy_core::hash_bytes(&bad.regions[0].bytes);
    assert!(bad.validate_for(&image, &function(&m)).is_err());
    let mut bad = s.clone();
    bad.versions.loader = 0;
    assert!(bad.validate_structure().is_err());
    bad.versions = ImageContextVersions::CURRENT;
    bad.versions.context = 0;
    assert!(bad.validate_structure().is_err());
    let mut bad = s.clone();
    bad.origin = Origin::ExternalReference;
    assert!(bad.validate_structure().is_err());
    let mut bad = s;
    bad.regions[0].bytes = vec![0; 1048577];
    assert!(bad.validate_structure().is_err());
    assert!(ReferenceSnapshot::parse(&vec![b' '; 16 * 1024 * 1024 + 1]).is_err());
}

#[path = "../../../tests/support/mod.rs"]
mod support;
#[test]
fn closure_persistence_import_blockers_and_version_invalidation() -> woomy_core::Result<()> {
    let directory = tempfile::tempdir()?;
    for folder in ["code", "content", "meta"] {
        std::fs::create_dir(directory.path().join(folder))?;
    }
    let mut bytes = support::cafe_fixture();
    // Redirect the fixture's one RELA to the linked branch and imported symbol 2.
    let sh = 64 + 7 * 40;
    let offset = u32::from_be_bytes(bytes[sh + 16..sh + 20].try_into().unwrap()) as usize;
    support::put32(&mut bytes, sh + 28, 1);
    support::put32(&mut bytes, offset, 0x100c);
    support::put32(&mut bytes, offset + 4, 0x20a);
    std::fs::write(directory.path().join("code/synthetic.rpx"), &bytes)?;
    let mut project = woomy_core::ProjectService::create(directory.path())?;
    woomy_core::AnalysisService::run(&mut project, &std::sync::atomic::AtomicU8::new(0), |_| {})?;
    let id: String =
        project
            .db
            .query_row("SELECT id FROM functions WHERE address=4096", [], |r| {
                r.get(0)
            })?;
    let closure = project.function_closure(&id)?;
    assert_eq!(closure["imports"][0]["symbol"], "OSReport");
    assert_eq!(closure["imports"][0]["provider_available"], false);
    assert_eq!(closure["closed"], false);
    assert!(closure["states"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("BLOCKED_UNKNOWN_IMPORT")));
    assert_eq!(project.import_catalog()?["unique_symbols"], 1);
    assert_eq!(
        project.import_catalog()?["items"][0]["entry_reachable"],
        true
    );
    assert!(
        !project.execution_context(&id)?["register_only_synthetic_eligible"]
            .as_bool()
            .unwrap()
    );
    assert!(project.validate_function_synthetic(&id).is_err());
    assert!(project
        .inspect_snapshot("../code/synthetic.rpx", true)
        .is_err());
    let semantic = project.semantic_analysis(Some(&id))?;
    assert_eq!(semantic["version"], woomy_core::semantic::SEMANTIC_VERSION);
    assert_eq!(semantic["metrics"]["platform_import_sites"], 1);
    assert_eq!(semantic["metrics"]["platform_sites_categorized"], 0);
    let persisted: String = project.db.query_row(
        "SELECT artifact FROM analysis_products WHERE kind='SEMANTIC' AND subject=?1",
        [&id],
        |r| r.get(0),
    )?;
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&persisted)?,
        semantic
    );
    project.db.execute(
        "UPDATE analysis_products SET artifact='{}' WHERE kind='SEMANTIC'",
        [],
    )?;
    assert_eq!(project.semantic_analysis(Some(&id))?, semantic);
    let audit = project.link_audit(None)?;
    assert_eq!(audit["newly_resolved"], 0);
    project
        .db
        .execute("UPDATE analysis_products SET versions='{}'", [])?;
    project.invalidate_versions()?;
    assert_eq!(
        project
            .db
            .query_row::<i64, _, _>("SELECT count(*) FROM analysis_products", [], |r| r.get(0))?,
        0
    );
    assert_eq!(
        std::fs::read(directory.path().join("code/synthetic.rpx"))?,
        bytes
    );
    Ok(())
}

#[test]
fn undefined_binding_is_unsupported_but_malformed_image_never_executes() {
    let mut m = module();
    m.symbols[0].section = 0;
    m.relocations.push(reloc(1, 0x2000, 0));
    let image = load(&m).unwrap();
    assert_eq!(image.relocations()[0].state, "UNSUPPORTED");
    assert_eq!(image.relocations()[0].target, None);
    m.relocations[0].offset = 0x6000;
    let image = load(&m).unwrap();
    assert_eq!(image.relocations()[0].state, "INVALID");
    assert!(register_only_program(&image, function(&m)).is_err());
}

#[test]
fn rel24_signed_limits_and_high_halfword_wrap_have_independent_answers() {
    for (place, target, expected) in [
        (0x02001000, 0x1000, 0x4a000001u32),
        (0x1000, 0x02000ffc, 0x49fffffd),
        (0x1004, 0x1000, 0x4bfffffd),
    ] {
        let mut m = module();
        let target_section = if place > 0x101f {
            m.sections.push(section(7, place, 4, 1, 6));
            m.sections
                .last_mut()
                .unwrap()
                .data
                .copy_from_slice(&0x48000001u32.to_be_bytes());
            7
        } else {
            let n = (place - 0x1000) as usize;
            m.sections[1].data[n..n + 4].copy_from_slice(&0x48000001u32.to_be_bytes());
            1
        };
        let symbol_section = if target > 0x101f {
            m.sections.push(section(8, target, 4, 1, 6));
            8
        } else {
            1
        };
        m.symbols[0].section = symbol_section;
        m.symbols[0].value = target;
        let mut r = reloc(10, place, 0);
        r.target_section = target_section;
        m.relocations.push(r);
        let image = load(&m).unwrap();
        assert_eq!(image.relocations()[0].state, "APPLIED_PROVEN_LOCAL");
        assert_eq!(image.relocations()[0].relocated, expected.to_be_bytes());
    }
    for (target, lo, hi, ha) in [
        (0u32, 0u16, 0u16, 0u16),
        (0x7fff, 0x7fff, 0, 0),
        (0x8000, 0x8000, 0, 1),
        (0x1234ffff, 0xffff, 0x1234, 0x1235),
        (0xffff8000, 0x8000, 0xffff, 0),
        (0xffffffff, 0xffff, 0xffff, 0),
    ] {
        for (kind, expected) in [(4, lo), (5, hi), (6, ha)] {
            let mut m = module();
            m.sections[3].address = target & !3;
            m.sections[3].size = 4;
            m.sections[3].file_size = 4;
            m.symbols[0].value = target;
            m.relocations.push(reloc(kind, 0x2000, 0));
            assert_eq!(
                load(&m).unwrap().relocations()[0].relocated,
                expected.to_be_bytes()
            );
        }
    }
}

#[test]
fn immutable_loader_values_do_not_turn_mutable_initial_memory_into_constants() {
    use woomy_ppc::dataflow::Value;
    let mut m = module();
    m.sections[3].flags = 2;
    let image = load(&m).unwrap();
    let original = woomy_core::module_model::ModuleModel::new("synthetic", &m);
    assert_eq!(original.read_word(0x8000), Value::Unknown);
    let with_image =
        woomy_core::module_model::ModuleModel::with_image("synthetic", &m, &image).unwrap();
    assert_eq!(with_image.read_word(0x8000), Value::constant(0));
    assert_eq!(with_image.read_word(0x2000), Value::Unknown);
    assert_eq!(with_image.read_word(0x8001), Value::Unknown);
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]
    #[test]
    fn bounded_layout_and_snapshot_metadata_never_panic(address: u32, alignment:u32, addend:i32, kind:u8, bytes:Vec<u8>) {
        let mut m=module();m.sections[2].address=address;m.sections[2].alignment=alignment;m.relocations.push(reloc(kind,address,addend));let _=load(&m);let _=ReferenceSnapshot::parse(&bytes);
    }
}

#[test]
fn entry_barrier_stops_both_engines_before_unresolved_instruction() {
    use woomy_ir::execution::{execute_ir_program, execute_ppc_program, Exit};
    let mut m = module();
    m.symbols[0].section = 0;
    let mut relocation = reloc(4, 0x1002, 0);
    relocation.target_section = 1;
    m.relocations.push(relocation);
    let image = load(&m).unwrap();
    let (program, code, memory, evidence) =
        woomy_core::bootstrap::entry_program(&image, function(&m), None).unwrap();
    assert!(program.source_barriers.contains_key(&0x1000));
    assert_eq!(evidence.len(), 1);
    let mut initial = woomy_ir::execution::Machine::new(0, vec![]);
    initial.gpr[3] = 0x1234;
    initial.guest_memory = memory;
    let ppc = execute_ppc_program(&program, &code, initial.clone(), 16);
    let ir = execute_ir_program(&program, initial.clone(), 16);
    assert_eq!(ppc, ir);
    assert_eq!(ppc.state, initial);
    assert!(ppc.trace.is_empty());
    assert!(matches!(
        ppc.exit,
        Exit::GuestFault {
            address: 0x1000,
            ..
        }
    ));
    assert!(woomy_codegen::generate_program(&program).is_err());
}

#[test]
fn bound_model_requires_external_provider_and_cannot_drop_identity_for_native() {
    use woomy_ir::execution::{execute_ir_program, execute_ppc_program, Exit};
    let m = module();
    let image = load(&m).unwrap();
    let process = woomy_core::bootstrap::process_descriptor(&image, Some(false), None).unwrap();
    let (program, code, memory, _) =
        woomy_core::bootstrap::entry_program(&image, function(&m), Some(&process)).unwrap();
    let mut initial = woomy_ir::execution::Machine::new(0, vec![]);
    initial.guest_memory = memory;
    process
        .attach(&mut initial, program.image.as_ref().unwrap())
        .unwrap();
    for result in [
        execute_ppc_program(&program, &code, initial.clone(), 16),
        execute_ir_program(&program, initial.clone(), 16),
    ] {
        assert!(matches!(result.exit, Exit::RuntimeFailure { .. }));
        assert!(result.trace.is_empty());
        assert_eq!(result.state, initial);
    }
    assert!(woomy_codegen::generate_program(&program).is_err());
    let mut wrong = program;
    wrong.runtime_context.as_mut().unwrap().image_hash = "b".repeat(64);
    assert!(wrong.validate().is_err());
}

#[test]
fn snapshot_replay_inputs_preserve_origin_and_reject_mid_instruction_entry() {
    use woomy_ir::execution::{execute_ir_program, execute_ppc_program};
    let m = module();
    let image = load(&m).unwrap();
    let mut capture = snapshot(&image);
    capture.registers.lr = 0xffff0003;
    let (program, code, _, _) =
        woomy_core::bootstrap::entry_program(&image, function(&m), None).unwrap();
    capture.validate_for(&image, &function(&m)).unwrap();
    let initial = capture.machine().unwrap();
    let ppc = execute_ppc_program(&program, &code, initial.clone(), 16);
    let ir = execute_ir_program(&program, initial, 16);
    assert_eq!(ppc, ir);
    assert_eq!(ppc.state.gpr[3], 1);
    assert_eq!(capture.origin, Origin::Synthetic);
    capture.cia += 4;
    assert!(capture.validate_structure().is_err());
}

#[test]
fn link_audit_is_name_independent_and_does_not_resolve_zero_or_weak_symbols() {
    let mut m = module();
    m.symbols[0].section = 0;
    m.symbols[0].value = 0;
    m.relocations.push(reloc(6, 0x2000, 0));
    m.relocations.push(reloc(4, 0x2004, 0));
    for (name, info) in [("$UNDEF", 0x11), ("unrelated", 0x21)] {
        m.symbols[0].name = name.into();
        m.symbols[0].info = info;
        let before = serde_json::to_vec(&m).unwrap();
        let image = load(&m).unwrap();
        let audit = woomy_core::linking::audit(&m).unwrap();
        assert_eq!(audit["reference_count"], 2);
        assert_eq!(audit["newly_resolved"], 0);
        assert_eq!(audit["resolution"], "UNKNOWN");
        assert_eq!(image.relocations()[0].state, "UNSUPPORTED");
        assert_eq!(before, serde_json::to_vec(&m).unwrap());
    }
}

#[test]
fn audit_rejects_duplicate_identity_and_obeys_record_budget() {
    let mut m = module();
    m.symbols[0].section = 0;
    m.relocations = (0..4097).map(|_| reloc(6, 0x2000, 0)).collect();
    assert!(woomy_core::linking::audit(&m).is_err());
    m.relocations.clear();
    m.symbols.push(Symbol {
        table: 5,
        index: 1,
        name: "duplicate".into(),
        value: 0,
        size: 0,
        info: 0,
        section: 0,
    });
    assert!(woomy_core::linking::audit(&m).is_err());
}
