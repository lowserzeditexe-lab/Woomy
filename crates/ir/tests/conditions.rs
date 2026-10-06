use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{
    execution::{compare, execute_ppc, Machine},
    Operation, Terminator,
};
fn fixture(words: &[u32]) -> (woomy_ir::Function, BTreeMap<u32, u32>) {
    let raw: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(i, w)| (0x1000 + i as u32 * 4, *w))
        .collect();
    let decoded = raw
        .iter()
        .map(|(pc, w)| (*pc, woomy_ppc::decode(*pc, *w)))
        .collect();
    let cfg = woomy_ppc::build_cfg(0x1000, &decoded, &BTreeSet::from([0x1000]), 100);
    (
        woomy_ir::lift("f", "m", &"0".repeat(64), 0x1000, &cfg, &decoded),
        raw,
    )
}
#[test]
fn every_bo_bi_ctr_and_cr_truth_combination_matches_with_independent_expectation() {
    for bo in 0..32 {
        for bi in 0..32 {
            for ctr in [0u32, 1, 2, u32::MAX] {
                for bit in [false, true] {
                    let (f, raw) = fixture(&[
                        (16 << 26) | (bo << 21) | (bi << 16) | 8,
                        0x38e00063,
                        0x4e800020,
                    ]);
                    let mut s = Machine::new(0, vec![]);
                    s.ctr = ctr;
                    s.lr = 0x80000003;
                    s.cr[(bi / 4) as usize] = if bit { 1 << (3 - bi % 4) } else { 0 };
                    let r = compare(&f, &raw, s, 10);
                    assert_eq!(r.status, "MATCH", "{bo} {bi} {ctr} {bit}: {r:?}");
                    let after = if bo & 4 == 0 {
                        ctr.wrapping_sub(1)
                    } else {
                        ctr
                    };
                    let take_count =
                        bo & 4 != 0 || (if bo & 2 != 0 { after == 0 } else { after != 0 });
                    let take_cr = bo & 16 != 0 || bit == (bo & 8 != 0);
                    assert_eq!(r.ir.state.ctr, after);
                    assert_eq!(
                        r.ir.state.gpr[7],
                        if take_count && take_cr { 0 } else { 99 }
                    );
                }
            }
        }
    }
}
#[test]
fn signed_unsigned_cr_fields_and_sticky_xer_flags_have_known_results() {
    for (word, a, b, xer, expected_gpr, expected_xer, expected_cr) in [
        (0x7c632615, 0x7fff_ffff, 1, 0, 0x8000_0000, 0xc000_0000, 9), // addo.
        (0x7c632615, 1, 2, 0xc000_0000, 3, 0x8000_0000, 5),           // clear OV, retain SO
        (0x7c632015, u32::MAX, 1, 0, 0, 0x2000_0000, 2),              // addc.
        (0x7c632011, 1, 0, 0, u32::MAX, 0, 8),                        // subfc.
        (0x3463ffff, 0, 0, 0xc000_0000, u32::MAX, 0xc000_0000, 9),    // addic.
    ] {
        let (f, raw) = fixture(&[word, 0x4e800020]);
        let mut s = Machine::new(0, vec![]);
        s.lr = 0x80000000;
        s.gpr[3] = a;
        s.gpr[4] = b;
        s.xer = xer;
        let r = compare(&f, &raw, s, 8);
        assert_eq!(r.status, "MATCH", "{word:x}: {r:?}");
        assert_eq!(r.ir.state.gpr[3], expected_gpr);
        assert_eq!(r.ir.state.xer, expected_xer);
        assert_eq!(r.ir.state.cr[0], expected_cr);
    }
    for (word, a, b, expected) in [
        (0x7c032000, 0x80000000, 1, 9),
        (0x7c032040, 0x80000000, 1, 5),
        (0x7c032000, 1, 1, 3),
        (0x7c032040, 0, 1, 9),
    ] {
        let (f, raw) = fixture(&[word, 0x4e800020]);
        let mut s = Machine::new(0, vec![]);
        s.lr = 0x80000000;
        s.gpr[3] = a;
        s.gpr[4] = b;
        s.xer = 0x80000000;
        let r = compare(&f, &raw, s, 8);
        assert_eq!(r.status, "MATCH");
        assert_eq!(r.ir.state.cr[0], expected);
    }
}
#[test]
fn conditional_verifier_rejects_wrong_boolean_and_cr_bit() {
    let (f, raw) = fixture(&[0x41820008, 0x38600001, 0x4e800020]);
    let mut bad = f.clone();
    if let Terminator::Branch { condition, .. } = &mut bad.blocks[0].terminator {
        condition.slot = 99;
    }
    assert!(!woomy_ir::verify(&bad).structural_valid);
    let mut bad = f;
    if let Operation::BranchCondition { cr_bit, .. } = &mut bad.blocks[0].instructions[0].operation
    {
        *cr_bit = Some(32);
    }
    assert!(!woomy_ir::verify(&bad).structural_valid);
    let mut s = Machine::new(0, vec![]);
    s.lr = 0x80000000;
    assert!(matches!(
        execute_ppc(&raw, 0x1000, s, 1).exit,
        woomy_ir::execution::Exit::Limit
    ));
}
#[test]
fn relocation_halfwords_sign_shift_adjustment_and_wrap_are_explicit() {
    let mut r = woomy_ir::Relocation {
        module: "m".into(),
        symbol: "s".into(),
        symbol_table: 1,
        symbol_index: 1,
        kind: 6,
        addend: 0,
        place: 0x1002,
        target: woomy_ppc::dataflow::Target::Address {
            module: "m".into(),
            section: 1,
            offset: 0,
        },
        local_address: Some(0xffff8000),
    };
    assert_eq!(r.immediate(true, 16), Some(0));
    r.kind = 5;
    assert_eq!(r.immediate(true, 16), Some(0xffff0000));
    r.kind = 4;
    assert_eq!(r.immediate(true, 0), Some(0xffff8000));
    assert_eq!(r.immediate(false, 0), Some(0x8000));
    r.local_address = Some(0x7fff);
    r.kind = 6;
    assert_eq!(r.immediate(false, 0), Some(0));
    r.local_address = Some(0x8000);
    assert_eq!(r.immediate(false, 0), Some(1));
    r.local_address = None;
    assert_eq!(r.immediate(false, 0), None);
}
