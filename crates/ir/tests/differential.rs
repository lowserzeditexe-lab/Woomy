use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{execution::*, *};
fn fixture(words: &[u32]) -> (Function, BTreeMap<u32, u32>) {
    let raw: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(n, w)| (0x1000 + n as u32 * 4, *w))
        .collect();
    let code = raw
        .iter()
        .map(|(pc, w)| (*pc, woomy_ppc::decode(*pc, *w)))
        .collect();
    let cfg = woomy_ppc::build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 1000);
    (lift("f", "m", &"1".repeat(64), 0x1000, &cfg, &code), raw)
}
fn machine() -> Machine {
    let mut s = Machine::new(0x2000, vec![0; 64]);
    s.lr = 0xdead_beef;
    s.ctr = 0x12345678;
    s.xer = 0xffffffff;
    s.cr = [15; 8];
    s.fpscr = 0x55aa;
    s.fpr = [0x7ff8000000001234; 32];
    s
}

#[test]
fn signed_offsets_aliasing_big_endian_and_all_untouched_state_match() {
    let (f, code) = fixture(&[0x9064fffc, 0x80a4fffc, 0x7ca32a14, 0x4e800020]);
    let mut s = machine();
    s.gpr[3] = 0x89abcdef;
    s.gpr[4] = 0x2004;
    let r = compare(&f, &code, s, 100);
    assert_eq!(r.status, "MATCH", "{r:?}");
    assert_eq!(&r.ir.state.memory[..4], &[0x89, 0xab, 0xcd, 0xef]);
    assert_eq!(
        r.ir.state.writes,
        vec![MemoryWrite {
            address: 0x2000,
            bytes: vec![0x89, 0xab, 0xcd, 0xef]
        }]
    );
    assert_eq!(r.ir.state.gpr[5], 0x13579bde);
    assert_eq!(r.ir.state.fpscr, 0x55aa);
    assert_eq!(r.ir.state.fpr, [0x7ff8000000001234; 32]);
}
#[test]
fn pc_materialization_lr_restore_ctr_and_branch_trace_match() {
    let (f, code) = fixture(&[
        0x7c0802a6, 0x48000005, 0x7c6802a6, 0x7c6903a6, 0x7c0803a6, 0x48000008, 0x38600000,
        0x4e800020,
    ]);
    let r = compare(&f, &code, machine(), 100);
    assert_eq!(r.status, "MATCH", "{r:?}");
    assert_eq!(r.ir.state.gpr[3], 0x1008);
    assert_eq!(r.ir.state.ctr, 0x1008);
    assert_eq!(
        r.ir.trace,
        vec![0x1000, 0x1004, 0x1008, 0x100c, 0x1010, 0x1014, 0x101c]
    );
}
#[test]
fn structural_validity_does_not_hide_a_semantic_mismatch() {
    let (mut f, code) = fixture(&[0x38600001, 0x4e800020]);
    f.blocks[0].instructions[1].operation = Operation::Constant { bits: 2 };
    assert!(verify(&f).structural_valid);
    let r = compare(&f, &code, machine(), 100);
    assert_eq!(r.status, "MISMATCH");
    assert!(!r.state_equal);
    assert!(r.trace_equal);
}
#[test]
fn unsupported_memory_calls_and_loops_cannot_report_success() {
    for words in [
        vec![0x80600000, 0x4e800020],
        vec![0x48000011, 0x4e800020],
        vec![0],
        vec![0x48000000],
    ] {
        let (f, code) = fixture(&words);
        let r = compare(&f, &code, machine(), 16);
        assert_eq!(r.status, "INCONCLUSIVE");
        assert!(r.ppc.trace.len() <= 16 && r.ir.trace.len() <= 16);
    }
    let (f, code) = fixture(&[0x4e800020]);
    let mut s = machine();
    s.lr = 0x1000;
    let r = compare(&f, &code, s, 16);
    assert_eq!(r.ir.exit, Exit::Limit);
    assert_eq!(r.ppc.exit, Exit::Limit);
    let (mut f, code) = fixture(&[0x38600001, 0x4e800020]);
    f.blocks[0].instructions[0].result.as_mut().unwrap().ty = Type::Ptr;
    assert_eq!(compare(&f, &code, machine(), 100).ir.exit, Exit::InvalidIr);
}
#[test]
fn unconditional_trap_has_no_fallthrough() {
    let (f, code) = fixture(&[0x7fe00008, 0x38600001]);
    let r = compare(&f, &code, machine(), 100);
    assert_eq!(r.status, "MATCH");
    assert_eq!(r.ir.exit, Exit::Trapped { address: 0x1000 });
}
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(512))]
    #[test]
    fn randomized_scalar_pipeline_matches(a: u32,b:u32,imm:u16,shift in 0u32..32,begin in 0u32..32,end in 0u32..32) {
        let words=[(14<<26)|(3<<21)|(3<<16)|imm as u32,0x7c632214,0x7c632050,(21<<26)|(3<<21)|(5<<16)|(shift<<11)|(begin<<6)|(end<<1),(27<<26)|(5<<21)|(5<<16)|imm as u32,0x90a60000,0x80e60000,0x4e800020];
        let (f,code)=fixture(&words);let mut s=machine();s.gpr[3]=a;s.gpr[4]=b;s.gpr[6]=0x2000;
        let r=compare(&f,&code,s,100);proptest::prop_assert_eq!(r.status,"MATCH");
    }
}
