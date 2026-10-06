//! Synthetic instructions and guest addresses only.
use std::collections::{BTreeMap, BTreeSet};
use woomy_core::{semantic_interprocedural as inter, semantic_values::*};
use woomy_ir::runtime::{GuestMemory, Region, EXECUTE, READ, WRITE};
use woomy_ppc::Register;
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
fn bl(pc: u32, target: u32) -> u32 {
    0x48000001 | (target.wrapping_sub(pc) & 0x03fffffc)
}

use woomy_core::semantic_memory::{self as effects, Alias};
fn symbolic(reg: u8, offset: i32) -> Location {
    Location::Symbolic {
        function: "f".into(),
        register: Register::Gpr(reg),
        offset,
    }
}
fn analyze(fs: &[woomy_ir::Function]) -> inter::Report {
    inter::analyze(fs, &memory(true), &[], "image", false).unwrap()
}
fn across(callee: &[u32], arg: u32) -> Vec<woomy_ir::Function> {
    let mut caller = function(
        0x1000,
        &[
            0x7fe802a6,
            0x39802000,
            0x38804000,
            0x91840000,
            0x38600000 | arg,
            bl(0x1014, 0x1800),
            0x38804000,
            0x81840000,
            0x7d8903a6,
            0x4e800421,
            0x7fe803a6,
            0x4e800020,
        ],
    );
    relocate(&mut caller, 0x1004, 0x2000);
    vec![
        caller,
        function(0x1800, callee),
        function(0x2000, &[0x3860000b, 0x4e800020]),
    ]
}
#[test]
fn alias_ranges_and_unknown_origins_are_conservative() {
    use Alias::*;
    let g = Location::Global;
    for (a, aw, b, bw, expected) in [
        (Some(g(0x4000)), 4, Some(g(0x4000)), 4, MustAlias),
        (Some(g(0x4000)), 4, Some(g(0x4002)), 2, MayAlias),
        (Some(g(0x4000)), 4, Some(g(0x4004)), 4, ProvenDisjoint),
        (Some(symbolic(3, 0)), 4, Some(symbolic(4, 0)), 4, MayAlias),
        (Some(symbolic(3, 8)), 4, Some(symbolic(3, 10)), 4, MayAlias),
        (
            Some(symbolic(3, 8)),
            4,
            Some(symbolic(3, 12)),
            4,
            ProvenDisjoint,
        ),
        (None, 4, Some(g(0x4000)), 4, Unknown),
        (Some(g(u32::MAX)), 4, Some(g(0)), 4, Unknown),
        (
            Some(symbolic(3, i32::MAX - 1)),
            4,
            Some(symbolic(3, i32::MIN)),
            2,
            MayAlias,
        ),
    ] {
        assert_eq!(effects::alias(a.as_ref(), aw, b.as_ref(), bw), expected);
    }
}
#[test]
fn local_effects_include_origins_widths_and_source_witnesses() {
    for (words, reads, writes, complete) in [
        (vec![0x80a30010, 0x4e800020], 1, 0, true),
        (vec![0x90a30010, 0x4e800020], 0, 1, true),
        (
            vec![0x38804000, 0x80a40000, 0x90a40010, 0x4e800020],
            1,
            1,
            true,
        ),
        (vec![0x38804000, 0x90a40000, 0x4e800020], 0, 1, true),
        (vec![0x41820008, 0x90a30010, 0x4e800020], 0, 1, true),
        (vec![0x90a30010, 0x90c30020, 0x4e800020], 0, 2, true),
        (vec![0x90a30010, 0xb0c30012, 0x4e800020], 0, 2, true),
        (
            vec![0x98a30010, 0xb0c30012, 0x90e30014, 0x4e800020],
            0,
            3,
            true,
        ),
        (vec![0x7cc32214, 0x90a60000, 0x4e800020], 0, 1, true),
        (vec![0xffff_ffff, 0x90a30010, 0x4e800020], 0, 0, false),
    ] {
        let r = analyze(&[function(0x1800, &words)]);
        let m = &r.memory_effects["synthetic.00001800"];
        assert_eq!(m.complete, complete, "{m:?}");
        assert_eq!(m.effects.iter().filter(|e| !e.write).count(), reads);
        assert_eq!(m.effects.iter().filter(|e| e.write).count(), writes);
        assert!(m.effects.iter().all(|e| e.operation.is_some()
            && e.provenance.contains(&e.source)
            && (e.certainty.starts_with("MAY")
                || e.certainty.starts_with("MUST_ON_NORMAL_RETURN"))));
    }
}
#[test]
fn readonly_and_disjoint_calls_preserve_but_aliases_and_partial_calls_block() {
    for (words, arg, promotes) in [
        (vec![0x80a30010, 0x4e800020], 0x4000, true),
        (vec![0x80a30000, 0x80c50000, 0x4e800020], 0x4000, true),
        (vec![0x90a30010, 0x4e800020], 0x4010, true),
        (vec![0x90a30000, 0x4e800020], 0x4000, false),
        (vec![0xb0a30002, 0x4e800020], 0x4000, false),
        (vec![0x90a60000, 0x4e800020], 0x4000, false),
        (vec![0x7cc32214, 0x90a60000, 0x4e800020], 0x4000, false),
        (vec![0xffff_ffff, 0x4e800020], 0x4000, false),
        (vec![0x4e800421, 0x4e800020], 0x4000, false),
    ] {
        let fs = across(&words, arg);
        let r = analyze(&fs);
        let s = &r.summaries["synthetic.00001000"];
        assert!(r.converged);
        assert_eq!(
            s.indirects[0].classification == "MACHINE_PROVEN_SINGLE_TARGET",
            promotes,
            "{s:?}"
        );
        let (_, proof) =
            woomy_core::semantic_proof::promote(&fs, &memory(true), &[], "image").unwrap();
        assert_eq!(!proof.is_empty(), promotes);
        if promotes {
            assert!(s.call_effects[0].preserved > 0);
            assert!(proof[0]["memory_effects"].is_object());
        }
    }
}
#[test]
fn nested_calls_and_shifted_bases_compose_exact_ranges() {
    let a = function(0x1000, &[0x38604000, bl(0x1004, 0x1800), 0x4e800020]);
    let b = function(0x1800, &[0x38630010, bl(0x1804, 0x2000), 0x4e800020]);
    let c = function(0x2000, &[0x90a30020, 0x80c30024, 0x4e800020]);
    let r = analyze(&[a, b, c]);
    let m = &r.memory_effects["synthetic.00001000"];
    assert!(m.complete);
    assert!(m.effects.iter().any(|e| e.write
        && e.location == Some(Location::Global(0x4030))
        && e.via_calls.len() == 2));
    assert!(m
        .effects
        .iter()
        .any(|e| !e.write && e.location == Some(Location::Global(0x4034))));
}
#[test]
fn recursion_mutation_readonly_and_unknown_effects_converge_conservatively() {
    for op in [0x80a30010, 0x90a30010, 0xffff_ffff] {
        for mutual in [false, true] {
            let a = function(
                0x1800,
                &[
                    op,
                    bl(0x1804, if mutual { 0x2000 } else { 0x1800 }),
                    0x4e800020,
                ],
            );
            let mut fs = vec![a];
            if mutual {
                fs.push(function(0x2000, &[bl(0x2000, 0x1800), 0x4e800020]));
            }
            let r = analyze(&fs);
            assert!(r.converged, "{op:x}");
            let m = &r.memory_effects["synthetic.00001800"];
            assert_eq!(m.complete, op != 0xffff_ffff);
            assert!(m.effects.len() <= 1);
            fs.reverse();
            assert_eq!(r, analyze(&fs));
        }
    }
}
#[test]
fn finite_values_survive_readonly_but_never_become_single_target() {
    let mut f = function(
        0x1000,
        &[
            0x38804000,
            0x39802000,
            0x41820008,
            0x39802800,
            0x91840000,
            bl(0x1014, 0x1800),
            0x38804000,
            0x81840000,
            0x7d8903a6,
            0x4e800421,
            0x4e800020,
        ],
    );
    relocate(&mut f, 0x1004, 0x2000);
    relocate(&mut f, 0x100c, 0x2800);
    for write in [false, true] {
        let fs = vec![
            f.clone(),
            function(
                0x1800,
                if write {
                    &[0x90a40000, 0x4e800020]
                } else {
                    &[0x80a40000, 0x4e800020]
                },
            ),
            function(0x2000, &[0x4e800020]),
            function(0x2800, &[0x4e800020]),
        ];
        let r = analyze(&fs);
        let site = &r.summaries["synthetic.00001000"].indirects[0];
        assert_ne!(site.classification, "MACHINE_PROVEN_SINGLE_TARGET");
        if !write {
            assert_eq!(site.targets, vec![0x2000, 0x2800]);
        } else {
            assert!(!site.value.unknown.is_empty());
        }
    }
}
#[test]
fn aggregate_offsets_are_composed_without_merging_distinct_origins() {
    let a = function(0x1000, &[0x38630010, bl(0x1004, 0x1800), 0x4e800020]);
    let b = function(0x1100, &[0x38640010, bl(0x1104, 0x1800), 0x4e800020]);
    let c = function(0x1800, &[0x80a30020, 0x80c30024, 0x4e800020]);
    let fs = vec![a, b, c];
    let r = analyze(&fs);
    let x = woomy_core::semantic_reconstruction::reconstruct(&r, &fs, &memory(true), &[]).unwrap();
    for origin in [
        "synthetic.00001000:input:Gpr(3)",
        "synthetic.00001100:input:Gpr(4)",
    ] {
        let a = x["aggregates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["pointer_origin"] == origin)
            .unwrap();
        assert!(a["fields"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| [0x30, 0x34].contains(&f["offset"].as_i64().unwrap())));
    }
}
#[test]
fn stale_effect_policy_is_rejected_at_application() {
    let fs = across(&[0x80a30010, 0x4e800020], 0x4000);
    let r = analyze(&fs);
    let mut m = r.memory_effects["synthetic.00001800"].clone();
    m.version = 0;
    let mut storage = Storage::default();
    storage.store(
        Some(Location::Global(0x4000)),
        4,
        Fact::bits(3),
        &memory(true),
    );
    let applied = effects::apply(
        &r.summaries["synthetic.00001000"].calls[0],
        Some(&m),
        &mut storage,
    );
    assert!(!applied.complete);
    assert!(storage.words.is_empty());
}

fn differential_native(
    fs: Vec<woomy_ir::Function>,
    states: Vec<woomy_ir::execution::Machine>,
    promote: bool,
) {
    use woomy_ir::{execution::*, runtime::*};
    let code: BTreeMap<_, _> = fs
        .iter()
        .flat_map(|f| &f.blocks)
        .flat_map(|b| &b.sources)
        .map(|s| (s.address, s.raw))
        .collect();
    let functions = if promote {
        let (lowered, proof) =
            woomy_core::semantic_proof::promote(&fs, &memory(true), &[], "image").unwrap();
        assert_eq!(proof.len(), 1);
        lowered
    } else {
        fs
    };
    let p = Program {
        image: None,
        runtime_context: None,
        source_barriers: BTreeMap::new(),
        version: RUNTIME_ABI_VERSION,
        entry: 0x1000,
        functions,
        imports: BTreeMap::new(),
        provider: ProviderKind::NoRuntime,
        limits: Limits::default(),
    };
    let states: Vec<_> = states
        .into_iter()
        .map(|mut s| {
            s.guest_memory = memory(true);
            for (pc, word) in &code {
                let (r, o) = s.guest_memory.locate(*pc, 4, READ).unwrap();
                s.guest_memory.regions[r].bytes[o..o + 4].copy_from_slice(&word.to_be_bytes());
            }
            s
        })
        .collect();
    let expected: Vec<_> = states
        .iter()
        .map(|s| {
            let ppc = execute_ppc_program(&p, &code, s.clone(), 32);
            let ir = execute_ir_program(&p, s.clone(), 32);
            assert_eq!(ppc.state, ir.state);
            assert_eq!(ppc.trace, ir.trace);
            assert_eq!(ppc.exit, ir.exit);
            ppc
        })
        .collect();
    let generated = woomy_codegen::generate_program(&p).unwrap();
    #[cfg(windows)]
    {
        let compiler = woomy_codegen::compiler::NativeCompiler::detect().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let build = compiler
            .build(&generated.source.unwrap(), temp.path())
            .unwrap();
        let exe = temp.path().join("fixture.exe");
        std::fs::write(&exe, build.executable.unwrap()).unwrap();
        let cases: Vec<_> = states.into_iter().map(|s| (s, 32)).collect();
        let native = woomy_codegen::validation::run(&exe, &cases).unwrap();
        for (native, ppc) in native.executions.iter().zip(expected) {
            assert_eq!(native.state, ppc.state);
            assert_eq!(native.trace, ppc.trace);
            assert_eq!(native.exit, ppc.exit);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = expected;
        assert!(generated.source.is_some());
    }
}
#[test]
fn conditional_lr_branches_match_complete_ppc_ir_and_native_states() {
    for bo in [0, 4, 8, 12, 16, 18] {
        let f = function(
            0x1000,
            &[
                (19 << 26) | (bo << 21) | (2 << 16) | (16 << 1),
                0x38600007,
                0x4e800020,
            ],
        );
        assert!(matches!(
            f.blocks[0].terminator,
            woomy_ir::Terminator::ConditionalLrBranch { .. }
        ));
        let mut states = vec![];
        for cr in [0, 2] {
            for ctr in [0, 1, 2] {
                let mut s = woomy_ir::execution::Machine::new(0, vec![]);
                s.cr[0] = cr;
                s.ctr = ctr;
                s.lr = 0xffff0003;
                s.gpr[3] = 42;
                s.xer = 0xa0000000;
                s.fpr[3] = 0xdeadbeef01234567;
                states.push(s);
            }
        }
        differential_native(vec![f], states, false);
    }
}
#[test]
fn conditional_link_and_reserved_control_forms_remain_unsupported() {
    for raw in [0x4d820021, 0x4d828020, 0x4c000420] {
        let f = function(0x1000, &[raw, 0x4e800020]);
        assert!(!matches!(
            f.blocks[0].terminator,
            woomy_ir::Terminator::ConditionalLrBranch { .. }
        ));
        assert!(f
            .blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .any(|i| matches!(i.operation, woomy_ir::Operation::UnknownInstruction { .. })));
    }
}
#[test]
fn memory_preservation_proofs_match_full_native_execution() {
    for (callee, arg) in [
        (vec![0x80a30010, 0x4e800020], 0x4000),
        (vec![0x90a30010, 0x4e800020], 0x4010),
    ] {
        let fs = across(&callee, arg);
        let mut state = woomy_ir::execution::Machine::new(0, vec![]);
        state.lr = 0xffff0003;
        state.gpr[5] = 0x12345678;
        differential_native(fs, vec![state], true);
    }
}

#[test]
fn recursive_offset_expansion_widens_instead_of_proving_readonly() {
    let f = function(
        0x1800,
        &[0x90a30000, 0x38630010, bl(0x1808, 0x1800), 0x4e800020],
    );
    let r = analyze(&[f]);
    let summary = &r.memory_effects["synthetic.00001800"];
    assert!(r.converged);
    assert!(summary.exhausted);
    assert!(!summary.complete);
    assert!(summary.unknown_writes);
}
#[test]
fn returned_pointer_then_passed_pointer_preserves_effect_origin() {
    let a = function(
        0x1000,
        &[
            0x38604000,
            bl(0x1004, 0x1800),
            bl(0x1008, 0x2000),
            0x4e800020,
        ],
    );
    let b = function(0x1800, &[0x38630010, 0x4e800020]);
    let c = function(0x2000, &[0x90a30020, 0x4e800020]);
    let r = analyze(&[a, b, c]);
    let m = &r.memory_effects["synthetic.00001000"];
    assert!(m.complete);
    assert!(m
        .effects
        .iter()
        .any(|e| e.location == Some(Location::Global(0x4030))));
}
#[test]
fn escaping_values_are_composed_without_assuming_a_source_pointer_type() {
    let a = function(0x1000, &[0x38604000, bl(0x1004, 0x1800), 0x4e800020]);
    let b = function(0x1800, &[0x90830000, 0x4e800020]);
    let r = analyze(&[a, b]);
    let m = &r.memory_effects["synthetic.00001000"];
    assert!(m
        .escaping_values
        .values()
        .any(|v| v.alternatives.contains(&Atom::Incoming {
            function: "synthetic.00001000".into(),
            register: Register::Gpr(4),
            offset: 0
        })));
}
#[test]
fn mapped_nonexecutable_entry_and_raw_integer_do_not_promote() {
    for (target, provenance) in [(0x4000, true), (0x2004, true), (0x2000, false)] {
        let mut fs = across(&[0x80a30010, 0x4e800020], 0x4000);
        fs[0] = function(
            0x1000,
            &[0x39800000 | target, 0x7d8903a6, 0x4e800421, 0x4e800020],
        );
        if provenance {
            relocate(&mut fs[0], 0x1000, target);
        }
        if target == 0x4000 {
            fs.push(function(target, &[0x4e800020]));
        }
        let (_, proof) =
            woomy_core::semantic_proof::promote(&fs, &memory(true), &[], "image").unwrap();
        assert!(proof.is_empty());
    }
}
