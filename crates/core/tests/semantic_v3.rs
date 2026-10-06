//! Synthetic instructions and guest addresses only.
use std::collections::{BTreeMap, BTreeSet};
use woomy_core::{semantic_interprocedural as inter, semantic_values::*};
use woomy_ir::runtime::{GuestMemory, Region, EXECUTE, READ, WRITE};
use woomy_ppc::Register;
#[path = "../../../tests/support/mod.rs"]
mod support;
fn memory(write: bool) -> GuestMemory {
    let mut bytes = vec![0; 64];
    bytes[..4].copy_from_slice(&0x2000u32.to_be_bytes());
    GuestMemory {
        regions: vec![
            Region {
                base: 0x1000,
                permissions: READ | EXECUTE,
                kind: 0,
                provenance: "Synthetic".into(),
                bytes: vec![0; 0x2000],
            },
            Region {
                base: 0x4000,
                permissions: READ | if write { WRITE } else { 0 },
                kind: 2,
                provenance: "Synthetic".into(),
                bytes,
            },
        ],
    }
}
fn initial() -> BTreeMap<u32, InitialWord> {
    BTreeMap::from([(
        0x4000,
        InitialWord {
            address: 0x4000,
            target: 0x2000,
            evidence: "relocation:0".into(),
        },
    )])
}
fn address(n: u32) -> Fact {
    Fact::atom(Atom::Bits {
        bits: n,
        address_source: Some(n),
    })
    .at("synthetic_address")
}
fn function(entry: u32, words: &[u32]) -> woomy_ir::Function {
    let code: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let a = entry + i as u32 * 4;
            (a, woomy_ppc::decode(a, *w))
        })
        .collect();
    let cfg = woomy_ppc::build_cfg(entry, &code, &BTreeSet::from([entry]), 128);
    woomy_ir::lift(
        &format!("synthetic.{entry:08X}"),
        "synthetic",
        &"a".repeat(64),
        entry,
        &cfg,
        &code,
    )
}
#[test]
fn mutable_initialization_requires_entry_evidence() {
    let m = memory(true);
    let s = Storage::default();
    let init = initial();
    assert_eq!(
        s.load(Some(Location::Global(0x4000)), 4, &m, &init, false)
            .status(),
        "UNKNOWN"
    );
    assert_eq!(
        s.load(Some(Location::Global(0x4000)), 4, &m, &init, true)
            .alternatives,
        address(0x2000).alternatives
    );
    assert_eq!(
        s.load(
            Some(Location::Global(0x4000)),
            4,
            &memory(false),
            &init,
            false
        )
        .status(),
        "DEFINITELY_ONE"
    );
}
#[test]
fn storage_is_point_specific_and_alias_calls_invalidate() {
    let m = memory(true);
    let init = initial();
    let mut s = Storage::default();
    s.store(Some(Location::Global(0x4000)), 4, address(0x2004), &m);
    let read = s.load(Some(Location::Global(0x4000)), 4, &m, &init, false);
    assert_eq!(read.alternatives, address(0x2004).alternatives);
    s.store(Some(Location::Global(0x4000)), 4, address(0x2008), &m);
    assert_ne!(
        read,
        s.load(Some(Location::Global(0x4000)), 4, &m, &init, false)
    );
    s.store(None, 4, address(0x2000), &m);
    assert!(s
        .load(Some(Location::Global(0x4000)), 4, &m, &init, false)
        .unknown
        .contains("UNKNOWN_ALIAS"));
    s.invalidate("UNKNOWN_CALL_EFFECT");
    assert_eq!(
        s.load(Some(Location::Global(0x4000)), 4, &m, &init, false)
            .status(),
        "CLOBBERED"
    );
}
#[test]
fn conditional_writers_preserve_all_alternatives_and_unknown_path() {
    let m = memory(true);
    let init = initial();
    let mut a = Storage::default();
    let mut b = Storage::default();
    a.store(Some(Location::Global(0x4000)), 4, address(0x2000), &m);
    b.store(Some(Location::Global(0x4000)), 4, address(0x2004), &m);
    let joined = Storage::join(&[&a, &b], &m, &init, false);
    assert_eq!(
        joined
            .load(Some(Location::Global(0x4000)), 4, &m, &init, false)
            .status(),
        "FINITE_SET"
    );
    b = Storage::default();
    let joined = Storage::join(&[&a, &b], &m, &init, false);
    assert_eq!(
        joined
            .load(Some(Location::Global(0x4000)), 4, &m, &init, false)
            .status(),
        "UNKNOWN"
    );
}
#[test]
fn overlap_width_and_permission_fail_closed() {
    let m = memory(true);
    let init = initial();
    let mut s = Storage::default();
    s.store(Some(Location::Global(0x4000)), 4, address(0x2000), &m);
    s.store(Some(Location::Global(0x4002)), 2, Fact::bits(3), &m);
    assert_eq!(
        s.load(Some(Location::Global(0x4000)), 4, &m, &init, false)
            .status(),
        "UNKNOWN"
    );
    assert_eq!(
        s.load(Some(Location::Global(0x403f)), 4, &m, &init, false)
            .status(),
        "UNKNOWN"
    );
    s.store(
        Some(Location::Global(0x4000)),
        4,
        address(0x2000),
        &memory(false),
    );
    assert_eq!(
        s.load(Some(Location::Global(0x4000)), 4, &m, &init, false)
            .status(),
        "UNKNOWN"
    );
}
#[test]
fn argument_and_return_constraints_are_deterministic_and_versioned() {
    // Caller passes 7; callee observes incoming r3, then returns 9.
    let caller = function(0x1000, &[0x38600007, 0x48000ffd, 0x38830001, 0x4e800020]);
    let callee = function(0x2000, &[0x38830001, 0x38600009, 0x4e800020]);
    let fs = vec![caller, callee];
    let report = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    assert!(report.converged);
    assert_eq!(report.constraints.len(), 1);
    assert_eq!(report.constraints[0].register, Register::Gpr(3));
    assert_eq!(
        report.constraints[0].value.alternatives,
        Fact::bits(7).alternatives
    );
    let ret = &report.summaries["synthetic.00001000"].candidate_return;
    assert_eq!(ret.alternatives, Fact::bits(9).alternatives);
    assert_eq!(
        report,
        inter::analyze(&fs, &memory(true), &[], "image", false).unwrap()
    );
    assert_ne!(
        report.identity,
        inter::analyze(&fs, &memory(true), &[], "changed", false)
            .unwrap()
            .identity
    );
}
#[test]
fn executable_looking_integer_does_not_pass_function_pointer_gate() {
    let f = function(0x1000, &[0x39802000, 0x7d8903a6, 0x4e800421, 0x4e800020]);
    let target = function(0x2000, &[0x38600001, 0x4e800020]);
    let r = inter::analyze(&[f, target], &memory(true), &[], "image", false).unwrap();
    let sites = &r.summaries["synthetic.00001000"].indirects;
    assert_eq!(sites.len(), 1);
    assert_ne!(sites[0].classification, "MACHINE_PROVEN_SINGLE_TARGET");
    assert!(sites[0]
        .failed_requirements
        .contains(&"NO_FUNCTION_ADDRESS_PROVENANCE".into()));
}

fn relocate(f: &mut woomy_ir::Function, pc: u32, target: u32) {
    for i in f.blocks.iter_mut().flat_map(|b| &mut b.instructions) {
        if i.source.address == pc
            && matches!(i.operation,woomy_ir::Operation::Constant {bits} if bits==target as u64)
        {
            i.operation = woomy_ir::Operation::RelocationImmediate {
                relocation: woomy_ir::Relocation {
                    module: f.module.clone(),
                    symbol: "synthetic_address".into(),
                    symbol_table: 1,
                    symbol_index: 1,
                    kind: 4,
                    addend: 0,
                    place: pc + 2,
                    target: woomy_ppc::dataflow::Target::Address {
                        module: f.module.clone(),
                        section: 0,
                        offset: target,
                    },
                    local_address: Some(target),
                },
                signed: true,
                shift: 0,
            };
        }
    }
}
#[test]
fn relocation_store_reload_ctr_proves_only_exact_function_entry() {
    for target in [0x2000, 0x2004, 0x7000] {
        let mut f = function(
            0x1000,
            &[
                0x39800000 | target,
                0x38804000,
                0x91840000,
                0x81840000,
                0x7d8903a6,
                0x4e800421,
                0x4e800020,
            ],
        );
        relocate(&mut f, 0x1000, target);
        let dest = function(0x2000, &[0x38600001, 0x4e800020]);
        let r = inter::analyze(&[f, dest], &memory(true), &[], "image", false).unwrap();
        let site = &r.summaries["synthetic.00001000"].indirects[0];
        assert_eq!(
            site.classification == "MACHINE_PROVEN_SINGLE_TARGET",
            target == 0x2000,
            "{site:?}"
        );
        assert_eq!(
            r.summaries["synthetic.00001000"].accesses[1].reaching_status,
            "DEFINITELY_ONE"
        );
    }
}
#[test]
fn finite_table_indices_keep_unknowns_and_all_entries() {
    let mut m = memory(false);
    m.regions[1].bytes[4..8].copy_from_slice(&0x2004u32.to_be_bytes());
    let mut init = initial();
    init.insert(
        0x4004,
        InitialWord {
            address: 0x4004,
            target: 0x2004,
            evidence: "relocation:1".into(),
        },
    );
    let mut index = Fact::bits(0x4000);
    index.merge(&Fact::bits(0x4004));
    let s = Storage::default();
    let value = s.load_value(&index, 4, &m, &init, false);
    assert_eq!(value.status(), "FINITE_SET");
    assert_eq!(value.alternatives.len(), 2);
    index.merge(&Fact::unknown("UNKNOWN_INDEX"));
    assert_eq!(
        s.load_value(&index, 4, &m, &init, false).status(),
        "UNKNOWN"
    );
}
#[test]
fn unknown_call_and_alias_block_machine_promotion() {
    for words in [
        vec![
            0x39802000, 0x38804000, 0x91840000, 0x48002001, 0x81840000, 0x7d8903a6, 0x4e800421,
            0x4e800020,
        ],
        vec![
            0x39802000, 0x38804000, 0x91840000, 0x90a60000, 0x81840000, 0x7d8903a6, 0x4e800421,
            0x4e800020,
        ],
    ] {
        let mut f = function(0x1000, &words);
        relocate(&mut f, 0x1000, 0x2000);
        let dest = function(0x2000, &[0x38600001, 0x4e800020]);
        let r = inter::analyze(&[f, dest], &memory(true), &[], "image", false).unwrap();
        assert!(r.summaries["synthetic.00001000"]
            .indirects
            .iter()
            .all(|s| s.classification != "MACHINE_PROVEN_SINGLE_TARGET"));
    }
}
#[test]
fn vtable_pattern_requires_initialization_table_and_consumer_chain() {
    let weak = function(
        0x1000,
        &[0x81830000, 0x818c0004, 0x7d8903a6, 0x4e800421, 0x4e800020],
    );
    let mut strong = function(
        0x1000,
        &[
            0x39804000, 0x91830000, 0x81830000, 0x818c0004, 0x7d8903a6, 0x4e800421, 0x4e800020,
        ],
    );
    relocate(&mut strong, 0x1000, 0x4000);
    let mut m = memory(false);
    m.regions[1].bytes[4..8].copy_from_slice(&0x2100u32.to_be_bytes());
    let initial = vec![
        InitialWord {
            address: 0x4000,
            target: 0x2000,
            evidence: "relocation:0".into(),
        },
        InitialWord {
            address: 0x4004,
            target: 0x2100,
            evidence: "relocation:1".into(),
        },
    ];
    let entries = BTreeMap::from([
        (0x2000, "synthetic.00002000".into()),
        (0x2100, "synthetic.00002100".into()),
    ]);
    let es: Vec<_> = initial
        .iter()
        .enumerate()
        .map(|(i, w)| woomy_core::semantic_tables::EntryEvidence {
            address: w.address,
            relocation_index: i,
            relocation_target: Some(w.target),
            applied_local_addr32: true,
        })
        .collect();
    let tables = woomy_core::semantic_tables::scan(&m, &es, &entries).unwrap();
    for (f, count) in [(weak, 0), (strong, 1)] {
        let fs = vec![
            f,
            function(0x2000, &[0x38600001, 0x4e800020]),
            function(0x2100, &[0x38600002, 0x4e800020]),
        ];
        let r = inter::analyze(&fs, &m, &initial, "image", false).unwrap();
        let grouped =
            woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &m, &tables).unwrap();
        assert_eq!(
            grouped["vtables"].as_array().unwrap().len(),
            count,
            "{grouped}"
        );
        assert_eq!(grouped["virtual_calls"].as_array().unwrap().len(), count);
    }
}

fn bl(pc: u32, target: u32) -> u32 {
    0x48000001 | (target.wrapping_sub(pc) & 0x03fffffc)
}
#[test]
fn recursive_sccs_converge_without_creating_evidence() {
    let a = function(0x1000, &[bl(0x1000, 0x2000), 0x4e800020]);
    let b = function(0x2000, &[bl(0x2000, 0x1000), 0x4e800020]);
    let self_call = function(0x2100, &[bl(0x2100, 0x2100), 0x4e800020]);
    let fs = vec![a, b, self_call];
    let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    assert!(r.converged);
    assert!(r.iterations < 128);
    assert!(r.call_sccs.iter().any(|s| s.len() == 2));
    assert!(r
        .summaries
        .values()
        .all(|s| s.candidate_return.status() == "UNKNOWN"));
    let ids: BTreeSet<_> = r.nodes.iter().map(|n| n.id.clone()).collect();
    assert!(r
        .nodes
        .iter()
        .all(|n| n.inputs.iter().all(|p| ids.contains(p))));
    assert_eq!(
        r,
        inter::analyze(
            &fs.into_iter().rev().collect::<Vec<_>>(),
            &memory(true),
            &[],
            "image",
            false
        )
        .unwrap()
    );
}
#[test]
fn caller_disagreement_is_retained_and_address_returns_reach_ctr() {
    let mut callee = function(0x2000, &[0x38602100, 0x4e800020]);
    relocate(&mut callee, 0x2000, 0x2100);
    let caller = function(
        0x1000,
        &[bl(0x1000, 0x2000), 0x7c6903a6, 0x4e800421, 0x4e800020],
    );
    let target = function(0x2100, &[0x38600001, 0x4e800020]);
    let r = inter::analyze(
        &[caller, callee, target],
        &memory(true),
        &[],
        "image",
        false,
    )
    .unwrap();
    assert_eq!(
        r.summaries["synthetic.00001000"].indirects[0].targets,
        vec![0x2100]
    );
    assert_eq!(
        r.summaries["synthetic.00001000"].indirects[0].classification,
        "MACHINE_PROVEN_SINGLE_TARGET"
    );
    for value in [7, 8] {
        let a = function(0x1000, &[0x38600007, bl(0x1004, 0x2000), 0x4e800020]);
        let b = function(
            0x1100,
            &[0x38600000 | value, bl(0x1104, 0x2000), 0x4e800020],
        );
        let c = function(0x2000, &[0x38830001, 0x4e800020]);
        let fs = vec![a, b, c];
        let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
        let x =
            woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &memory(true), &[]).unwrap();
        assert_eq!(
            x["argument_values"]["synthetic.00002000:input:Gpr(3)"]["alternatives"]
                .as_array()
                .unwrap()
                .len(),
            if value == 7 { 1 } else { 2 }
        );
    }
}
#[test]
fn unrelated_pointer_origins_do_not_merge_equal_fields() {
    let a = function(0x1000, &[0x80830010, 0x80a30020, 0x4e800020]);
    let b = function(0x2000, &[0x80830010, 0x80a30020, 0x4e800020]);
    let fs = vec![a, b];
    let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    let x = woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &memory(true), &[]).unwrap();
    assert_eq!(x["aggregates"].as_array().unwrap().len(), 2);
    assert_ne!(
        x["aggregates"][0]["pointer_origin"],
        x["aggregates"][1]["pointer_origin"]
    );
}

#[test]
fn machine_promotion_preserves_complete_ppc_ir_state_and_indirect_trace_kind() {
    use woomy_ir::{
        execution::{execute_ir_program, execute_ppc_program, Machine},
        runtime::*,
    };
    let mut caller = function(
        0x1000,
        &[
            0x7fe802a6, 0x39802000, 0x7d8903a6, 0x4e800421, 0x7fe803a6, 0x4e800020,
        ],
    );
    relocate(&mut caller, 0x1004, 0x2000);
    let callee = function(0x2000, &[0x3860000b, 0x4e800020]);
    let fs = vec![caller, callee];
    let (lowered, proofs) =
        woomy_core::semantic_proof::promote(&fs, &memory(true), &[], "image").unwrap();
    assert_eq!(proofs.len(), 1);
    assert_ne!(fs, lowered);
    let code: BTreeMap<_, _> = fs
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.sources)
        .map(|s| (s.address, s.raw))
        .collect();
    let p = Program {
        image: None,
        runtime_context: None,
        source_barriers: BTreeMap::new(),
        version: RUNTIME_ABI_VERSION,
        entry: 0x1000,
        functions: lowered,
        imports: BTreeMap::new(),
        provider: ProviderKind::NoRuntime,
        limits: Limits::default(),
    };
    let mut state = Machine::new(0, vec![]);
    state.lr = 0xffff0003;
    state.guest_memory = memory(true);
    for (pc, word) in &code {
        let (r, o) = state.guest_memory.locate(*pc, 4, READ).unwrap();
        state.guest_memory.regions[r].bytes[o..o + 4].copy_from_slice(&word.to_be_bytes());
    }
    let ppc = execute_ppc_program(&p, &code, state.clone(), 64);
    let ir = execute_ir_program(&p, state.clone(), 64);
    assert_eq!(ppc.state, ir.state);
    assert_eq!(ppc.exit, ir.exit);
    assert_eq!(ppc.trace, ir.trace);
    assert_eq!(ir.state.gpr[3], 11);
    assert_eq!(ir.state.calls[0].kind, 1);
    let generated = woomy_codegen::generate_program(&p).unwrap();
    #[cfg(windows)]
    {
        let compiler = woomy_codegen::compiler::NativeCompiler::detect().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let build = compiler
            .build(&generated.source.unwrap(), temp.path())
            .unwrap();
        let exe = temp.path().join("promoted.exe");
        std::fs::write(&exe, build.executable.unwrap()).unwrap();
        let native = woomy_codegen::validation::run(&exe, &[(state, 64)])
            .unwrap()
            .executions
            .remove(0);
        assert_eq!(native.state, ppc.state);
        assert_eq!(native.trace, ppc.trace);
        assert_eq!(native.exit, ppc.exit);
    }
    #[cfg(not(windows))]
    assert!(generated.source.is_some());
}

#[test]
fn type_conflicts_and_shared_pointer_aggregate_constraints_are_explicit() {
    let mut a = function(0x1000, &[0x38604000, bl(0x1004, 0x2000), 0x4e800020]);
    relocate(&mut a, 0x1000, 0x4000);
    let b = function(0x1100, &[0x38600007, bl(0x1104, 0x2000), 0x4e800020]);
    let c = function(0x2000, &[0x80830010, 0x80a30020, 0x4e800020]);
    let fs = vec![a, b, c];
    let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    let x = woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &memory(true), &[]).unwrap();
    let ty = x["type_constraints"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["subject"] == "synthetic.00002000:input:Gpr(3)")
        .unwrap();
    assert_eq!(ty["conflicting"], true);
    assert!(ty["alternatives"].get("GuestPointer").is_some());
    assert!(ty["alternatives"].get("Integer").is_some());
    let caller = function(0x1000, &[bl(0x1000, 0x2000), 0x4e800020]);
    let callee = function(0x2000, &[0x80830010, 0x80a30020, 0x4e800020]);
    let fs = vec![caller, callee];
    let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    let x = woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &memory(true), &[]).unwrap();
    assert!(x["aggregates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["pointer_origin"] == "synthetic.00001000:input:Gpr(3)"
            && a["fields"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f.get("via_call").is_some())));
}
#[test]
fn callback_requires_pass_store_and_invocation_not_a_name() {
    let mut caller = function(0x1000, &[0x38602100, bl(0x1004, 0x2000), 0x4e800020]);
    relocate(&mut caller, 0x1000, 0x2100);
    let registration = function(
        0x2000,
        &[
            0x38804000, 0x90640000, 0x81840000, 0x7d8903a6, 0x4e800421, 0x4e800020,
        ],
    );
    let target = function(0x2100, &[0x38600001, 0x4e800020]);
    let fs = vec![caller, registration, target];
    let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    let x = woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &memory(true), &[]).unwrap();
    assert_eq!(x["callback_candidates"].as_array().unwrap().len(), 1);
    assert!(r.summaries["synthetic.00002000"]
        .indirects
        .iter()
        .all(|s| s.classification != "MACHINE_PROVEN_SINGLE_TARGET"));
}
#[test]
fn alternative_budget_is_absorbing_and_stale_ir_changes_identity() {
    let mut fact = Fact::default();
    for n in 0..100 {
        fact.merge(&Fact::bits(n));
    }
    assert!(fact.unknown.contains("VALUE_BUDGET_EXHAUSTED"));
    assert!(fact.alternatives.is_empty());
    fact.merge(&address(0x2000));
    assert!(fact.alternatives.is_empty());
    let a = inter::analyze(
        &[function(0x1000, &[0x38600001, 0x4e800020])],
        &memory(true),
        &[],
        "image",
        false,
    )
    .unwrap();
    let b = inter::analyze(
        &[function(0x1000, &[0x38600002, 0x4e800020])],
        &memory(true),
        &[],
        "image",
        false,
    )
    .unwrap();
    assert_ne!(a.identity, b.identity);
}

#[test]
fn finite_machine_targets_are_retained_without_unsupported_dispatch_promotion() {
    let mut f = function(
        0x1000,
        &[
            0x2c030000, 0x4182000c, 0x39802000, 0x48000008, 0x39802100, 0x7d8903a6, 0x4e800421,
            0x4e800020,
        ],
    );
    relocate(&mut f, 0x1008, 0x2000);
    relocate(&mut f, 0x1010, 0x2100);
    let fs = vec![
        f,
        function(0x2000, &[0x38600001, 0x4e800020]),
        function(0x2100, &[0x38600002, 0x4e800020]),
    ];
    let r = inter::analyze(&fs, &memory(true), &[], "image", false).unwrap();
    let site = &r.summaries["synthetic.00001000"].indirects[0];
    assert_eq!(site.targets, vec![0x2000, 0x2100]);
    assert_eq!(site.classification, "FINITE_TARGET_SET");
    assert!(
        woomy_core::semantic_proof::promote(&fs, &memory(true), &[], "image")
            .unwrap()
            .1
            .is_empty()
    );
}
#[test]
fn imported_unknown_effects_cannot_supply_a_return_or_preserve_storage() {
    let mut f = function(
        0x1000,
        &[
            0x39802000,
            0x38804000,
            0x91840000,
            bl(0x100c, 0x3000),
            0x81840000,
            0x7d8903a6,
            0x4e800421,
            0x4e800020,
        ],
    );
    relocate(&mut f, 0x1000, 0x2000);
    for i in f.blocks.iter_mut().flat_map(|b| &mut b.instructions) {
        if i.source.address == 0x100c {
            if let woomy_ir::Operation::Call { target, .. } = &mut i.operation {
                *target = woomy_ir::CallTarget::Imported {
                    library: "synthetic".into(),
                    symbol: "UnknownEffects".into(),
                };
            }
        }
    }
    let r = inter::analyze(
        &[f, function(0x2000, &[0x38600001, 0x4e800020])],
        &memory(true),
        &[],
        "image",
        false,
    )
    .unwrap();
    let s = &r.summaries["synthetic.00001000"];
    assert!(s.memory_unknown);
    assert_eq!(s.indirects[0].classification, "UNKNOWN_CALL_EFFECT");
    assert!(!s.indirects[0].failed_requirements.is_empty());
}

#[test]
fn project_promotes_fresh_proof_rebuilds_summaries_and_redrives_corrupt_products(
) -> woomy_core::Result<()> {
    let dir = tempfile::tempdir()?;
    for folder in ["code", "content", "meta"] {
        std::fs::create_dir(dir.path().join(folder))?;
    }
    let mut bytes = support::cafe_fixture();
    let text = u32::from_be_bytes(bytes[120..124].try_into().unwrap()) as usize;
    for (n, word) in [
        0x39800000u32,
        0x38802000,
        0x91840000,
        0x81840000,
        0x7d8903a6,
        0x4e800421,
        0x4e800020,
        0x60000000,
        0x38600001,
        0x4e800020,
    ]
    .into_iter()
    .enumerate()
    {
        support::put32(&mut bytes, text + n * 4, word);
    }
    let sh = 64 + 7 * 40;
    let offset = u32::from_be_bytes(bytes[sh + 16..sh + 20].try_into().unwrap()) as usize;
    support::put32(&mut bytes, sh + 28, 1);
    support::put32(&mut bytes, offset, 0x1002);
    support::put32(&mut bytes, offset + 4, 0x104);
    std::fs::write(dir.path().join("code/synthetic.rpx"), bytes)?;
    let mut project = woomy_core::ProjectService::create(dir.path())?;
    woomy_core::AnalysisService::run(&mut project, &std::sync::atomic::AtomicU8::new(0), |_| {})?;
    let before: usize = project.db.query_row(
        "SELECT count(*) FROM indirect_sites WHERE is_return=0 AND status='RESOLVED'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(before, 0);
    let summary = project.semantic_analysis(None)?;
    assert_eq!(summary["machine_promoted_targets"], 1);
    let audit: serde_json::Value = serde_json::from_slice(&std::fs::read(
        dir.path()
            .join(".woomy/reports/semantic-indirect-audit.json"),
    )?)?;
    let site = audit["sites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["pc"] == 4116)
        .unwrap();
    assert_eq!(site["classification"], "MACHINE_PROVEN_SINGLE_TARGET");
    assert!(site["missing_evidence"].is_null());
    let (fid, status, target): (String, String, u32) = project.db.query_row(
        "SELECT function,status,target FROM indirect_sites WHERE address=4116",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    assert_eq!(status, "RESOLVED");
    assert_eq!(target, 0x1020);
    assert!(project.function_summary(&fid).is_ok());
    let edges: usize = project.db.query_row(
        "SELECT count(*) FROM call_edges WHERE caller=?1 AND address=4116",
        [&fid],
        |r| r.get(0),
    )?;
    assert_eq!(edges, 1);
    project.db.execute("UPDATE analysis_products SET artifact='{}',versions='{}' WHERE kind='SEMANTIC_INTERPROCEDURAL'",[])?;
    assert_eq!(summary, project.semantic_analysis(None)?);
    let integrity: String = project
        .db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    assert_eq!(integrity, "ok");
    Ok(())
}

#[test]
fn narrow_storage_cannot_preserve_pointer_provenance_and_symbolic_offsets_wrap() {
    let m = memory(true);
    let mut s = Storage::default();
    s.store(Some(Location::Global(0x4000)), 1, address(0x2000), &m);
    let read = s.load(Some(Location::Global(0x4000)), 1, &m, &initial(), false);
    assert_eq!(read.alternatives, Fact::bits(0).alternatives);
    let base = |offset| Location::Symbolic {
        function: "synthetic".into(),
        register: Register::Gpr(3),
        offset,
    };
    s.store(Some(base(i32::MAX - 1)), 4, address(0x2000), &m);
    s.store(Some(base(i32::MIN)), 2, Fact::bits(1), &m);
    assert_eq!(
        s.load(Some(base(i32::MAX - 1)), 4, &m, &initial(), false)
            .status(),
        "UNKNOWN"
    );
}
