use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::*;
fn function(words: &[u32]) -> Function {
    let code: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let pc = 0x1000 + i as u32 * 4;
            (pc, woomy_ppc::decode(pc, *w))
        })
        .collect();
    let cfg = woomy_ppc::build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
    lift(
        "module.test.00001000",
        "module.test",
        &"0".repeat(64),
        0x1000,
        &cfg,
        &code,
    )
}
#[test]
fn scalar_lifting_is_typed_deterministic_and_preserves_every_source() {
    let words = [0x3c601234, 0x60635678, 0x3883ffff, 0x7c642050, 0x4e800020];
    let f = function(&words);
    assert_eq!(f, function(&words));
    assert_eq!(f.status, "LIFTED");
    let v = verify(&f);
    assert!(v.structural_valid, "{:?}", v.diagnostics);
    assert!(!v.executable_semantics_validated);
    assert_eq!(f.blocks[0].sources.len(), words.len());
    assert!(f.blocks[0]
        .instructions
        .iter()
        .all(|i| i.source.raw == words[((i.source.address - 0x1000) / 4) as usize]));
    let restored: Function = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
    assert_eq!(restored, f);
}
#[test]
fn memory_has_explicit_wrapping_address_big_endian_and_faults() {
    let mut f = function(&[0x8064fffc, 0x90650008, 0x4e800020]);
    assert!(verify(&f).structural_valid);
    let op = f.blocks[0]
        .instructions
        .iter_mut()
        .find(|i| matches!(i.operation, Operation::Load { .. }))
        .unwrap();
    let Operation::Load { width, endian, .. } = &mut op.operation else {
        unreachable!()
    };
    assert_eq!(*width, 4);
    assert_eq!(*endian, Endian::Big);
    *endian = Endian::Little;
    assert!(verify(&f).diagnostics.iter().any(|d| d.code == "MEMORY"));
}
#[test]
fn unsupported_unknown_and_relocation_semantics_are_never_dropped() {
    for word in [0u32, 0x7c602028, 0xc0640000, 0x7c630194] {
        let f = function(&[word, 0x4e800020]);
        let v = verify(&f);
        assert!(v.structural_valid, "{:?}", v.diagnostics);
        assert_eq!(f.status, "PARTIALLY_LIFTED");
        assert!(f.blocks[0]
            .instructions
            .iter()
            .any(|i| matches!(i.operation, Operation::UnknownInstruction { .. })));
        assert!(v.unknown_operations > 0);
    }
    let mut code = BTreeMap::from([
        (0x1000, woomy_ppc::decode(0x1000, 0x3c600000)),
        (0x1004, woomy_ppc::decode(0x1004, 0x4e800020)),
    ]);
    code.get_mut(&0x1000).unwrap().status = "RELOCATION_MODELED".into();
    let cfg = woomy_ppc::build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 10);
    let f = lift("f", "m", &"0".repeat(64), 0x1000, &cfg, &code);
    assert!(verify(&f).structural_valid);
    assert!(matches!(
        f.blocks[0].instructions[0].operation,
        Operation::UnknownInstruction { .. }
    ));
}
#[test]
fn conditional_control_is_explicit_and_indirect_calls_stay_partial() {
    let f = function(&[0x41820008, 0x38600001, 0x4e800020]);
    assert_eq!(f.status, "LIFTED");
    assert!(verify(&f).structural_valid);
    assert!(matches!(f.blocks[0].terminator, Terminator::Branch { .. }));
    for words in [
        vec![0x41820009, 0x38600001, 0x4e800020],
        vec![0x4e800421, 0x4e800020],
        vec![0x4e800420],
    ] {
        let f = function(&words);
        let v = verify(&f);
        assert!(v.structural_valid, "{:?}", v.diagnostics);
        assert_eq!(f.status, "PARTIALLY_LIFTED");
        assert!(v.unknown_operations > 0);
    }
    let f = function(&[0x48000011, 0x4e800020]);
    assert!(matches!(
        f.blocks[0].instructions[0].operation,
        Operation::Call {
            signature: Type::Unknown,
            ..
        }
    ));
    assert!(verify(&f).structural_valid);
}
#[test]
fn verifier_rejects_missing_values_wrong_types_and_duplicate_ids() {
    let f = function(&[0x38600001, 0x4e800020]);
    let mut bad = f.clone();
    let op = &mut bad.blocks[0].instructions[2].operation;
    let Operation::Binary { lhs, .. } = op else {
        panic!("expected arithmetic")
    };
    lhs.slot = 99;
    assert!(verify(&bad)
        .diagnostics
        .iter()
        .any(|d| d.code == "VALUE_REFERENCE"));
    let mut bad = f.clone();
    bad.blocks[0].instructions[0].result.as_mut().unwrap().ty = Type::I64;
    assert!(verify(&bad).diagnostics.iter().any(|d| d.code == "TYPE"));
    let mut bad = f.clone();
    bad.blocks[0].instructions[1].id = bad.blocks[0].instructions[0].id;
    assert!(verify(&bad)
        .diagnostics
        .iter()
        .any(|d| d.code == "INSTRUCTION_ID"));
}
#[test]
fn verifier_checks_cfg_provenance_and_coverage_claims() {
    let f = function(&[0x48000008, 0, 0x4e800020]);
    assert!(verify(&f).structural_valid);
    let mut bad = f.clone();
    bad.blocks[0].successors[0].destination = Destination::Block(0x9990);
    assert!(verify(&bad)
        .diagnostics
        .iter()
        .any(|d| d.code == "BRANCH_TARGET"));
    let mut bad = f;
    bad.blocks[0].terminator_source.raw ^= 1;
    assert!(verify(&bad)
        .diagnostics
        .iter()
        .any(|d| d.code == "PROVENANCE"));
    let mut bad = function(&[0]);
    bad.status = "LIFTED".into();
    assert!(verify(&bad)
        .diagnostics
        .iter()
        .any(|d| d.code == "COVERAGE"));
    bad.blocks[0].instructions.clear();
    assert!(verify(&bad)
        .diagnostics
        .iter()
        .any(|d| d.code == "DROPPED_UNKNOWN"));
}
