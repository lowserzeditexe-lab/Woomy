//! Entirely synthetic machine words; no game bytes or recovered source names.
use std::collections::{BTreeMap, BTreeSet};
use woomy_core::semantic::{analyze, CandidateStatus, SemanticReport, SemanticType};
fn function(words: &[u32]) -> woomy_ir::Function {
    let instructions: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let pc = 0x1000 + i as u32 * 4;
            (pc, woomy_ppc::decode(pc, *w))
        })
        .collect();
    let cfg = woomy_ppc::build_cfg(0x1000, &instructions, &BTreeSet::from([0x1000]), 128);
    woomy_ir::lift(
        "synthetic.1000",
        "synthetic",
        &"a".repeat(64),
        0x1000,
        &cfg,
        &instructions,
    )
}
#[test]
fn repeated_fields_keep_unknown_types_and_exact_ir_evidence() {
    let f = function(&[0x80830010, 0x80a30024, 0x90c30030, 0x4e800020]);
    let before = serde_json::to_vec(&f).unwrap();
    let report = analyze(&f).unwrap();
    assert_eq!(before, serde_json::to_vec(&f).unwrap());
    assert_eq!(report.aggregates.len(), 1);
    assert_eq!(
        report.aggregates[0]
            .fields
            .iter()
            .map(|f| f.offset)
            .collect::<Vec<_>>(),
        vec![0x10, 0x24, 0x30]
    );
    assert!(report.aggregates[0]
        .fields
        .iter()
        .all(|f| f.ty == SemanticType::Unknown && f.width_bytes == 4));
    assert_eq!(report.aggregates[0].extent, None);
    for e in &report.evidence {
        assert!(f.blocks.iter().any(|b| b.id == e.block
            && b.instructions
                .iter()
                .any(|i| i.id == e.operation && i.source.address == e.pc)));
    }
    for op in &report.operations {
        assert!(!op.candidate.evidence.is_empty());
        assert!(op
            .candidate
            .evidence
            .iter()
            .all(|id| report.evidence.get(*id as usize).is_some()));
    }
    assert_eq!(report, analyze(&f).unwrap());
    assert_eq!(
        serde_json::from_slice::<SemanticReport>(&serde_json::to_vec(&report).unwrap()).unwrap(),
        report
    );
}
#[test]
fn mixed_comparison_interpretations_are_not_silently_selected() {
    let r = analyze(&function(&[0x2c030000, 0x28030000, 0x4e800020])).unwrap();
    assert!(r
        .types
        .iter()
        .any(|t| t.candidate.status == CandidateStatus::Conflicting
            && t.alternatives.contains(&SemanticType::SignedInteger)
            && t.alternatives.contains(&SemanticType::UnsignedInteger)));
    assert!(r
        .types
        .iter()
        .all(|t| t.candidate.confidence_basis_points.is_none()));
}
#[test]
fn stack_and_loaded_unknown_bases_do_not_become_source_structures() {
    let stack = analyze(&function(&[0x80810010, 0x80a10024, 0x4e800020])).unwrap();
    assert!(stack.aggregates.is_empty());
    let changed = analyze(&function(&[0x80630000, 0x80830010, 0x80a30024, 0x4e800020])).unwrap();
    assert!(changed.aggregates.is_empty());
    assert_eq!(changed.metrics["memory_operations"], 3);
}
#[test]
fn overlapping_widths_remain_conflicting_hypotheses() {
    let r = analyze(&function(&[0x80830010, 0xa0a30012, 0x4e800020])).unwrap();
    assert_eq!(r.aggregates.len(), 1);
    assert_eq!(
        r.aggregates[0].candidate.status,
        CandidateStatus::Conflicting
    );
    assert!(r.aggregates[0]
        .fields
        .iter()
        .all(|f| f.candidate.status == CandidateStatus::Conflicting));
}
#[test]
fn unknown_instructions_break_register_propagation() {
    let r = analyze(&function(&[0x80830010, 0xffffffff, 0x80a30024, 0x4e800020])).unwrap();
    assert!(r.aggregates.is_empty());
    assert_eq!(r.metrics["virtual_calls_resolved"], 0);
    assert_eq!(r.metrics["signatures_proven"], 0);
}
#[test]
fn semantic_identity_changes_when_machine_ir_changes() {
    let a = analyze(&function(&[0x80830010, 0x80a30024, 0x4e800020])).unwrap();
    let b = analyze(&function(&[0x80830014, 0x80a30024, 0x4e800020])).unwrap();
    assert_ne!(a.ir_hash, b.ir_hash);
    let mut bad = function(&[0x4e800020]);
    bad.ir_version = 0;
    assert!(analyze(&bad).is_err());
}

#[test]
fn cfg_straight_line_preserves_base_and_definition_provenance() {
    let f = function(&[0x38830010, 0x48000004, 0x80a40000, 0x80c40008, 0x4e800020]);
    let r = analyze(&f).unwrap();
    assert_eq!(r.aggregates.len(), 1);
    assert_eq!(
        r.aggregates[0]
            .fields
            .iter()
            .map(|f| f.offset)
            .collect::<Vec<_>>(),
        vec![16, 24]
    );
    assert!(r
        .inter_block_facts
        .iter()
        .any(|f| f.register == woomy_ppc::Register::Gpr(4) && !f.definitions.is_empty()));
    for fact in &r.inter_block_facts {
        for anchor in &fact.definitions {
            assert!(f.blocks.iter().any(|b| b.id == anchor.block
                && b.instructions
                    .iter()
                    .any(|i| i.id == anchor.operation && i.source.address == anchor.pc)));
        }
    }
    assert_eq!(r, analyze(&f).unwrap());
}
#[test]
fn diamond_agreement_conflict_and_unknown_are_distinct() {
    let words = [
        0x2c050000, 0x4182000c, 0x38830010, 0x48000008, 0x38830010, 0x80c40000, 0x80e40008,
        0x4e800020,
    ];
    let same = analyze(&function(&words)).unwrap();
    assert_eq!(same.aggregates.len(), 1);
    let mut different = words;
    different[4] = 0x38830020;
    let different = analyze(&function(&different)).unwrap();
    assert!(different.aggregates.is_empty());
    assert!(different
        .inter_block_facts
        .iter()
        .any(|f| matches!(f.value, woomy_core::semantic::Expr::Conflict(_))));
    let mut missing = words;
    missing[4] = 0x80830000;
    let missing = analyze(&function(&missing)).unwrap();
    assert!(missing.aggregates.is_empty());
    assert!(!missing
        .inter_block_facts
        .iter()
        .any(|f| f.block == 0x1014 && f.register == woomy_ppc::Register::Gpr(4)));
}
#[test]
fn loop_fixed_point_and_opaque_call_kill() {
    let r = analyze(&function(&[
        0x38830010, 0x48000004, 0x80a40000, 0x80c40008, 0x2c070000, 0x4082fff4, 0x4e800020,
    ]))
    .unwrap();
    assert_eq!(r.aggregates.len(), 1);
    assert!(r.metrics["cfg_iterations"].as_u64().unwrap() < 256);
    let call = analyze(&function(&[
        0x38830010, 0x48000009, 0x80a40000, 0x80c40008, 0x4e800020,
    ]))
    .unwrap();
    assert!(call.aggregates.is_empty());
}
#[test]
fn alias_uncertainty_and_vtable_pattern_cannot_create_object_identity() {
    let r = analyze(&function(&[
        0x81830000, 0x90830000, 0x818c0034, 0x7d8903a6, 0x4e800421, 0x4e800020,
    ]))
    .unwrap();
    assert!(r.objects.is_empty() && r.vtables.is_empty() && r.classes.is_empty());
    assert_eq!(r.metrics["virtual_calls_resolved"], 0);
    assert!(!r
        .types
        .iter()
        .any(|t| t.alternatives.iter().any(|t| matches!(
            t,
            SemanticType::FunctionPointer { .. } | SemanticType::ClassPointer { .. }
        ))));
    let scalar = analyze(&function(&[0x38601000, 0x4e800020])).unwrap();
    assert!(scalar.types.is_empty());
}

#[test]
fn table_proof_requires_relocations_function_starts_and_read_only_storage() {
    use woomy_core::semantic_tables::{scan, EntryEvidence};
    use woomy_ir::runtime::{GuestMemory, Region, EXECUTE, READ, WRITE};
    let mut memory = GuestMemory {
        regions: vec![
            Region {
                base: 0x1000,
                permissions: READ | EXECUTE,
                kind: 0,
                provenance: "synthetic".into(),
                bytes: vec![0; 16],
            },
            Region {
                base: 0x2000,
                permissions: READ,
                kind: 1,
                provenance: "synthetic".into(),
                bytes: [0x1000u32.to_be_bytes(), 0x1008u32.to_be_bytes()].concat(),
            },
        ],
    };
    let functions = BTreeMap::from([
        (0x1000, "synthetic.a".into()),
        (0x1008, "synthetic.b".into()),
    ]);
    let entries = vec![
        EntryEvidence {
            address: 0x2000,
            relocation_index: 0,
            relocation_target: Some(0x1000),
            applied_local_addr32: true,
        },
        EntryEvidence {
            address: 0x2004,
            relocation_index: 1,
            relocation_target: Some(0x1008),
            applied_local_addr32: true,
        },
    ];
    let tables = scan(&memory, &entries, &functions).unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].status, "HYPOTHESIS");
    assert_eq!(tables[0].entries.len(), 2);
    memory.regions[1].permissions |= WRITE;
    assert_eq!(
        scan(&memory, &entries, &functions).unwrap()[0].unresolved_entries,
        2
    );
    memory.regions[1].permissions = READ;
    memory.regions[1].bytes[4..].copy_from_slice(&0x1004u32.to_be_bytes());
    assert_eq!(
        scan(&memory, &entries, &functions).unwrap()[0].unresolved_entries,
        1
    );
    let mut unproven = entries;
    for e in &mut unproven {
        e.applied_local_addr32 = false;
    }
    assert!(scan(&memory, &unproven, &functions).unwrap().is_empty());
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]
    #[test]
    fn base_offset_candidates_preserve_independent_immediate_offsets(left in 0u16..8192, right in 0u16..8192) {
        let a=u32::from(left)*4;let b=u32::from(right)*4;
        let r=analyze(&function(&[0x80830000|a,0x80a30000|b,0x4e800020])).unwrap();
        proptest::prop_assert_eq!(r.aggregates.len(),usize::from(a!=b));
        if a!=b {
            let got:BTreeSet<_>=r.aggregates[0].fields.iter().map(|f|f.offset).collect();
            proptest::prop_assert_eq!(got,BTreeSet::from([a as i64,b as i64]));
            proptest::prop_assert!(r.aggregates[0].fields.iter().all(|f|f.ty==SemanticType::Unknown));
        }
    }
}
