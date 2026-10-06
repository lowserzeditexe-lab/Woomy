#[path = "../../../tests/support/mod.rs"]
mod support;
use std::collections::{BTreeMap, BTreeSet};
use woomy_binary::{Module, Relocation};
use woomy_core::module_model::ModuleModel;
use woomy_ppc::dataflow::{self, Atom, Target, Value};
use woomy_ppc::{build_cfg, Flow, Register};

fn module(words: &[u32]) -> Module {
    let mut m = woomy_binary::parse(&support::cafe_fixture()).unwrap();
    m.sections[1].data = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    m.sections[1].size = words.len() as u32 * 4;
    m.relocations.clear();
    m
}
fn relocation(offset: u32, kind: u8, symbol: u32) -> Relocation {
    Relocation {
        section: 7,
        target_section: 1,
        symbol_table: 4,
        offset,
        kind,
        symbol,
        addend: Some(0),
    }
}
fn facts(m: &Module) -> dataflow::Dataflow {
    let model = ModuleModel::new("module.test", m);
    let code: BTreeMap<_, _> = (0..m.sections[1].size / 4)
        .filter_map(|n| {
            let pc = 0x1000 + n * 4;
            model.instruction(pc).map(|d| (pc, d))
        })
        .collect();
    let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
    dataflow::analyze(0x1000, &cfg, &code, &model, || false)
}
fn value(f: &dataflow::Dataflow, pc: u32, reg: Register) -> Value {
    f.writes
        .iter()
        .find(|x| x.address == pc && x.register == reg)
        .unwrap()
        .fact
        .value
        .clone()
}
#[test]
fn adjusted_high_low_is_symbolic_and_raw_bytes_stay_unchanged() {
    let mut m = module(&[0x3c601235, 0x3863abcd, 0x4e800020]);
    m.sections[2].address = 0x1234abcd;
    m.symbols[1].section = 2;
    m.symbols[1].value = 0x1234abcd;
    m.relocations = vec![relocation(0x1002, 6, 1), relocation(0x1006, 4, 1)];
    let original = m.sections[1].data.clone();
    let f = facts(&m);
    assert_eq!(
        value(&f, 0x1004, Register::Gpr(3)),
        Value::Known(Atom::Target(Target::Address {
            module: "module.test".into(),
            section: 2,
            offset: 0
        }))
    );
    assert_eq!(m.sections[1].data, original);
}
#[test]
fn relocation_ir_preserves_identity_and_imports_have_no_native_address() {
    for imported in [false, true] {
        let mut m = module(&[0x3c600000, 0x38630000, 0x4e800020]);
        m.sections[2].address = 0x1234abcd;
        m.symbols[1].section = 2;
        m.symbols[1].value = 0x1234abcd;
        m.relocations = vec![
            relocation(0x1002, 6, if imported { 2 } else { 1 }),
            relocation(0x1006, 4, if imported { 2 } else { 1 }),
        ];
        let original = m.sections[1].data.clone();
        let model = ModuleModel::new("m", &m);
        let code: BTreeMap<_, _> = [0x1000, 0x1004, 0x1008]
            .into_iter()
            .map(|pc| (pc, model.instruction(pc).unwrap()))
            .collect();
        let relocs = code
            .keys()
            .filter_map(|pc| model.ir_relocation(*pc).map(|r| (*pc, r)))
            .collect();
        let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
        let f = woomy_ir::lift_with_relocations(
            "f",
            "m",
            &"0".repeat(64),
            0x1000,
            &cfg,
            &code,
            &relocs,
        );
        assert!(woomy_ir::verify(&f).structural_valid);
        let g = woomy_codegen::generate(&f);
        if imported {
            assert!(g.source.is_none());
            assert_eq!(f.status, "PARTIALLY_LIFTED");
        } else {
            assert!(g.source.is_some());
            assert_eq!(f.status, "LIFTED");
            let mut initial = woomy_ir::execution::Machine::new(0, vec![]);
            initial.lr = 0x80000000;
            let result = woomy_ir::execution::execute_ir(&f, initial, 10);
            assert_eq!(result.state.gpr[3], 0x1234abcd);
        }
        assert_eq!(m.sections[1].data, original);
        m.relocations
            .push(relocation(0x1002, 6, if imported { 2 } else { 1 }));
        assert!(ModuleModel::new("m", &m).ir_relocation(0x1000).is_none());
    }
}
#[test]
fn high_ori_import_address_never_becomes_a_runtime_number() {
    let mut m = module(&[0x3c600000, 0x60630000, 0x4e800020]);
    m.relocations = vec![relocation(0x1002, 5, 2), relocation(0x1006, 4, 2)];
    let f = facts(&m);
    assert!(matches!(
        value(&f, 0x1004, Register::Gpr(3)),
        Value::Known(Atom::Target(Target::ImportedSymbol { .. }))
    ));
    let model = ModuleModel::new("module.test", &m);
    assert!(model
        .address(&model.relocated_target(&m.relocations[0]).unwrap())
        .is_none());
    m.relocations[0].kind = 6; // HA + ORI is not an HI/LO reconstruction.
    assert_eq!(value(&facts(&m), 0x1004, Register::Gpr(3)), Value::Unknown);
}
#[test]
fn mismatched_symbols_unsupported_and_overlapping_relocations_stay_unknown() {
    let mut m = module(&[0x3c600000, 0x38630000, 0x4e800020]);
    m.relocations = vec![relocation(0x1002, 6, 1), relocation(0x1006, 4, 2)];
    m.symbols[1].value = 0x1000;
    assert_eq!(value(&facts(&m), 0x1004, Register::Gpr(3)), Value::Unknown);
    m.relocations[0].kind = 255;
    assert_eq!(
        ModuleModel::new("m", &m)
            .instruction(0x1000)
            .unwrap()
            .control_flow,
        Flow::Unknown
    );
    m.relocations[0].kind = 6;
    m.relocations.push(relocation(0x1002, 6, 1));
    assert_eq!(
        ModuleModel::new("m", &m)
            .instruction(0x1000)
            .unwrap()
            .status,
        "RELOCATION_REQUIRED"
    );
}
#[test]
fn immutable_word_relocations_and_mutable_memory_are_distinct() {
    let mut m = module(&[0x38602000, 0x80830000, 0x4e800020]);
    m.sections[2].flags = 2;
    m.symbols[1].value = 0x1000;
    let mut r = relocation(0x2000, 1, 1);
    r.target_section = 2;
    m.relocations.push(r);
    assert!(matches!(
        value(&facts(&m), 0x1004, Register::Gpr(4)),
        Value::Known(Atom::Target(_))
    ));
    m.sections[2].flags = 3;
    assert_eq!(value(&facts(&m), 0x1004, Register::Gpr(4)), Value::Unknown);
    m.sections[2].flags = 2;
    m.relocations[0].addend = None;
    assert_eq!(value(&facts(&m), 0x1004, Register::Gpr(4)), Value::Unknown);
}
#[test]
fn prior_store_invalidates_memory_reads_and_truncated_words_are_unknown() {
    let mut m = module(&[0x38602000, 0x90610008, 0x80830000, 0x4e800020]);
    m.sections[2].flags = 2;
    m.sections[2].data = 0x1000u32.to_be_bytes().to_vec();
    assert_eq!(value(&facts(&m), 0x1008, Register::Gpr(4)), Value::Unknown);
    m.sections[2].data.truncate(3);
    assert_eq!(ModuleModel::new("m", &m).read_word(0x2000), Value::Unknown);
}
#[test]
fn rel24_static_import_and_bad_place() {
    let mut m = module(&[0x48000001, 0x4e800020, 0x4e800020]);
    m.symbols[1].value = 0x1008;
    m.relocations = vec![relocation(0x1000, 10, 1)];
    assert_eq!(
        ModuleModel::new("m", &m)
            .instruction(0x1000)
            .unwrap()
            .control_flow,
        Flow::Call {
            target: 0x1008,
            conditional: false
        }
    );
    m.relocations[0].symbol = 2;
    assert_eq!(
        ModuleModel::new("m", &m)
            .instruction(0x1000)
            .unwrap()
            .control_flow,
        Flow::ExternalCall
    );
    m.relocations[0].target_section = 2;
    assert_eq!(
        ModuleModel::new("m", &m)
            .instruction(0x1000)
            .unwrap()
            .control_flow,
        Flow::Unknown
    );
}

fn resolutions(m: &Module) -> Vec<woomy_core::control_flow::Resolution> {
    let model = ModuleModel::new("module.test", m);
    let code: BTreeMap<_, _> = (0..m.sections[1].size / 4)
        .filter_map(|n| {
            let pc = 0x1000 + n * 4;
            model.instruction(pc).map(|d| (pc, d))
        })
        .collect();
    let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
    woomy_core::control_flow::resolve(&cfg, &code, &facts(m), &model)
}
#[test]
fn proven_ctr_and_lr_targets_have_evidence() {
    for (spr, branch) in [
        (0x7c6903a6, 0x4e800420),
        (0x7c6903a6, 0x4e800421),
        (0x7c6803a6, 0x4e800020),
    ] {
        let m = module(&[0x38601010, spr, branch, 0x60000000, 0x4e800020]);
        let r = resolutions(&m);
        assert_eq!(r[0].status, "RESOLVED");
        assert_eq!(
            r[0].targets,
            vec![woomy_core::control_flow::ControlTarget::Static {
                module: "module.test".into(),
                address: 0x1010
            }]
        );
        assert!(r[0].evidence.contains("Instruction"));
    }
}
#[test]
fn non_executable_unknown_and_ambiguous_indirects_are_not_guessed() {
    let m = module(&[0x38602000, 0x7c6903a6, 0x4e800420]);
    assert_eq!(resolutions(&m)[0].status, "UNRESOLVED");
    let m = module(&[0x4e800420]);
    assert_eq!(resolutions(&m)[0].status, "UNRESOLVED");
    let m = module(&[
        0x4182000c, 0x3860101c, 0x48000008, 0x38601020, 0x7c6903a6, 0x4e800420, 0, 0x4e800020,
        0x4e800020,
    ]);
    let r = resolutions(&m);
    assert_eq!(r[0].status, "AMBIGUOUS");
    assert_eq!(r[0].targets.len(), 2);
}
#[test]
fn imported_indirect_resolves_identity_without_a_runtime_address() {
    let mut m = module(&[0x3c600000, 0x60630000, 0x7c6903a6, 0x4e800421, 0x4e800020]);
    m.relocations = vec![relocation(0x1002, 5, 2), relocation(0x1006, 4, 2)];
    let r = resolutions(&m);
    assert_eq!(r[0].status, "RESOLVED");
    assert_eq!(
        r[0].targets,
        vec![woomy_core::control_flow::ControlTarget::Imported {
            library: "coreinit".into(),
            symbol: "OSReport".into()
        }]
    );
    m.imports[0].code = false;
    assert_eq!(resolutions(&m)[0].status, "UNRESOLVED");
}

#[test]
fn unresolved_d_form_relocation_preserves_sequential_flow_but_not_values() {
    let mut m = module(&[0x34630000, 0x4e800020]); // addic. r3,r3,lo(unknown)
    m.relocations = vec![relocation(0x1002, 4, 0)];
    let d = ModuleModel::new("m", &m).instruction(0x1000).unwrap();
    assert_eq!(d.control_flow, Flow::Next);
    assert_eq!(d.status, "RELOCATION_REQUIRED");
    assert!(matches!(d.semantics, woomy_ppc::Semantics::Unknown));
    assert_eq!(value(&facts(&m), 0x1000, Register::Gpr(3)), Value::Unknown);
}

#[test]
fn symbol_addend_can_cross_allocated_sections_but_not_leave_module() {
    let mut m = module(&[0x48000001]);
    m.symbols[1].value = 0x1000;
    let mut r = relocation(0x1000, 1, 1);
    r.addend = Some(0x1000);
    let model = ModuleModel::new("m", &m);
    assert_eq!(
        model.relocated_target(&r),
        Some(Target::Address {
            module: "m".into(),
            section: 2,
            offset: 0
        })
    );
    r.addend = Some(0x3000);
    assert!(model.relocated_target(&r).is_none());
}

fn switch_module() -> Module {
    let mut m = module(&[
        0x28030002, // cmplwi r3,2
        0x4181002c, // bgt default (1030)
        0x38802000, // li r4,2000
        0x5465103a, // rlwinm r5,r3,2,0,29
        0x7cc4282e, // lwzx r6,r4,r5
        0x7cc903a6, // mtctr r6
        0x4e800420, // bctr
        0, 0x4e800020, 0x4e800020, 0x4e800020, 0, 0x4e800020,
    ]);
    m.sections[2].flags = 2;
    m.sections[2].size = 12;
    m.sections[2].data = [0x1020u32, 0x1024, 0x1028]
        .into_iter()
        .flat_map(u32::to_be_bytes)
        .collect();
    m
}
fn tables(m: &Module) -> Vec<woomy_core::jump_tables::JumpTable> {
    let model = ModuleModel::new("module.test", m);
    let code: BTreeMap<_, _> = (0..m.sections[1].size / 4)
        .map(|n| {
            let pc = 0x1000 + n * 4;
            (pc, model.instruction(pc).unwrap())
        })
        .collect();
    let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
    woomy_core::jump_tables::discover(0x1000, &cfg, &code, &facts(m), &model)
}
#[test]
fn bounded_switch_and_default_are_verified_and_feed_cfg() {
    let m = switch_module();
    let t = tables(&m);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].status, "VERIFIED");
    assert_eq!(t[0].bound, Some(3));
    assert_eq!(t[0].default_target, Some(0x1030));
    assert_eq!(t[0].entries, vec![(0, 0x1020), (1, 0x1024), (2, 0x1028)]);
    let model = ModuleModel::new("m", &m);
    let code = (0..m.sections[1].size / 4)
        .map(|n| {
            let pc = 0x1000 + n * 4;
            (pc, model.instruction(pc).unwrap())
        })
        .collect();
    let edges = BTreeMap::from([(0x1018, BTreeSet::from([0x1020, 0x1024, 0x1028]))]);
    let cfg =
        woomy_ppc::build_cfg_with_targets(0x1000, &code, &BTreeSet::from([0x1000]), 100, &edges);
    assert!(cfg.blocks.iter().any(|b| b.start == 0x1024));
    assert_eq!(
        cfg.edges
            .iter()
            .filter(|e| e.kind == "IndirectProven")
            .count(),
        3
    );
}
#[test]
fn malformed_truncated_and_non_executable_tables_are_rejected_without_partial_entries() {
    for bad in [0x1021u32, 0x2000, 0xfffffff0] {
        let mut m = switch_module();
        m.sections[2].data[4..8].copy_from_slice(&bad.to_be_bytes());
        let t = tables(&m);
        assert_eq!(t[0].status, "REJECTED");
        assert!(t[0].entries.is_empty());
    }
    let mut m = switch_module();
    m.sections[2].data.truncate(11);
    assert_eq!(tables(&m)[0].status, "REJECTED");
}
#[test]
fn duplicate_switch_targets_preserve_case_indices() {
    let mut m = switch_module();
    m.sections[2].data[4..8].copy_from_slice(&0x1020u32.to_be_bytes());
    let t = tables(&m);
    assert_eq!(t[0].status, "VERIFIED");
    assert_eq!(t[0].entries, vec![(0, 0x1020), (1, 0x1020), (2, 0x1028)]);
}
#[test]
fn missing_bound_and_mutable_tables_never_scan_for_targets() {
    let mut m = switch_module();
    m.sections[1].data[..4].copy_from_slice(&0x60000000u32.to_be_bytes());
    let t = tables(&m);
    assert_eq!(t[0].status, "CANDIDATE");
    assert!(t[0].bound.is_none());
    assert!(t[0].entries.is_empty());
    let mut m = switch_module();
    m.sections[2].flags = 3;
    let t = tables(&m);
    assert_eq!(t[0].status, "PARTIAL");
    assert!(t[0].entries.is_empty());
}
