use std::collections::{BTreeMap, BTreeSet};
use woomy_ppc::dataflow::{self, Atom, Definition, Value};
use woomy_ppc::{build_cfg, decode, Register};

fn run(words: &[u32]) -> dataflow::Dataflow {
    let code: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(n, w)| {
            let pc = 0x1000 + n as u32 * 4;
            (pc, decode(pc, *w))
        })
        .collect();
    let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 1024);
    dataflow::analyze(0x1000, &cfg, &code, &(), || false)
}
fn written(d: &dataflow::Dataflow, pc: u32, r: Register) -> Value {
    d.writes
        .iter()
        .find(|f| f.address == pc && f.register == r)
        .unwrap()
        .fact
        .value
        .clone()
}
fn dform(op: u32, rt: u32, ra: u32, imm: u16) -> u32 {
    (op << 26) | (rt << 21) | (ra << 16) | imm as u32
}
fn spr(rt: u32, spr: u32, write: bool) -> u32 {
    (31 << 26)
        | (rt << 21)
        | ((spr & 31) << 16)
        | ((spr >> 5) << 11)
        | ((if write { 467 } else { 339 }) << 1)
}

#[test]
fn straight_constants_addresses_copies_and_reaching_definitions() {
    let d = run(&[
        dform(15, 3, 0, 0x1234),
        dform(24, 3, 3, 0xabcd),
        dform(15, 4, 0, 0x1235),
        dform(14, 4, 4, 0xabcd),
        0x7c651b78,
        0x4e800020,
    ]);
    assert!(d.converged);
    assert_eq!(
        written(&d, 0x1004, Register::Gpr(3)),
        Value::constant(0x1234abcd)
    );
    assert_eq!(
        written(&d, 0x100c, Register::Gpr(4)),
        Value::constant(0x1234abcd)
    );
    assert_eq!(
        written(&d, 0x1010, Register::Gpr(5)),
        Value::constant(0x1234abcd)
    );
    assert_eq!(
        d.used(0x1010, Register::Gpr(3)).unwrap().definitions,
        BTreeSet::from([Definition::Instruction(0x1004)])
    );
}
#[test]
fn conflicting_join_retains_both_definitions_and_unknown_path_is_top() {
    let d = run(&[
        0x4182000c, 0x38600001, 0x48000008, 0x38600002, 0x7c641b78, 0x4e800020,
    ]);
    let f = d.used(0x1010, Register::Gpr(3)).unwrap();
    assert_eq!(
        f.value,
        Value::Conflicting(BTreeSet::from([Atom::Constant(1), Atom::Constant(2)]))
    );
    assert_eq!(
        f.definitions,
        BTreeSet::from([
            Definition::Instruction(0x1004),
            Definition::Instruction(0x100c)
        ])
    );
    let d = run(&[
        0x4182000c, 0x38600001, 0x48000008, 0x60000000, 0x7c641b78, 0x4e800020,
    ]);
    assert_eq!(
        d.used(0x1010, Register::Gpr(3)).unwrap().value,
        Value::Unknown
    );
}
#[test]
fn loop_widens_and_is_deterministic() {
    let words = [0x38600000, 0x38630001, 0x4182fffc, 0x4e800020];
    let a = run(&words);
    let b = run(&words);
    assert!(a.converged);
    assert_eq!(a.inputs, b.inputs);
    assert_eq!(a.steps, b.steps);
    assert_eq!(a.inputs[&0x1004].value(Register::Gpr(3)), Value::Unknown);
}
#[test]
fn calls_kill_values_and_have_explicit_reaching_definitions() {
    let d = run(&[0x38600007, 0x48000101, 0x7c641b78, 0x4e800020]);
    let f = d.used(0x1008, Register::Gpr(3)).unwrap();
    assert_eq!(f.value, Value::Unknown);
    assert_eq!(f.definitions, BTreeSet::from([Definition::Call(0x1004)]));
}
#[test]
fn unknown_semantics_are_a_barrier_even_when_fallthrough_is_known() {
    let mut code = BTreeMap::from([
        (0x1000, decode(0x1000, 0x38600007)),
        (0x1004, decode(0x1004, 0)),
        (0x1008, decode(0x1008, 0x7c641b78)),
    ]);
    code.get_mut(&0x1004).unwrap().control_flow = woomy_ppc::Flow::Next;
    let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 10);
    let d = dataflow::analyze(0x1000, &cfg, &code, &(), || false);
    let f = d.used(0x1008, Register::Gpr(3)).unwrap();
    assert_eq!(f.value, Value::Unknown);
    assert_eq!(f.definitions, BTreeSet::from([Definition::Unknown(0x1004)]));
}
#[test]
fn lr_ctr_and_pc_materialization_preserve_pre_branch_values() {
    let d = run(&[
        0x38601020,
        spr(3, 9, true),
        spr(4, 9, false),
        spr(4, 8, true),
        0x4e800020,
    ]);
    assert_eq!(
        d.used(0x1010, Register::Lr).unwrap().value,
        Value::constant(0x1020)
    );
    assert_eq!(
        written(&d, 0x1008, Register::Gpr(4)),
        Value::constant(0x1020)
    );
    let d = run(&[0x48000005, spr(3, 8, false), 0x4e800020]);
    assert_eq!(
        written(&d, 0x1004, Register::Gpr(3)),
        Value::constant(0x1004)
    );
    let d = run(&[0x38601020, spr(3, 9, true), 0x4e800421, 0x4e800020]);
    assert_eq!(
        d.used(0x1008, Register::Ctr).unwrap().value,
        Value::constant(0x1020)
    );
    assert_eq!(d.used(0x100c, Register::Lr).unwrap().value, Value::Unknown);
}
#[test]
fn rotate_mask_and_wrapping_arithmetic() {
    assert_eq!(dataflow::mask32(0, 31), u32::MAX);
    assert_eq!(dataflow::mask32(28, 3), 0xf000000f);
    let rotate = (21 << 26) | (3 << 21) | (4 << 16) | (2 << 11) | (29 << 1);
    let d = run(&[
        0x3860ffff,
        0x38800002,
        (31 << 26) | (5 << 21) | (3 << 16) | (4 << 11) | (266 << 1),
        rotate,
        0x4e800020,
    ]);
    assert_eq!(written(&d, 0x1008, Register::Gpr(5)), Value::constant(1));
    assert_eq!(
        written(&d, 0x100c, Register::Gpr(4)),
        Value::constant(0xfffffffc)
    );
}
#[test]
fn bounded_cancellation_never_exposes_unstable_facts() {
    let code = BTreeMap::from([(0, decode(0, 0x48000000))]);
    let cfg = build_cfg(0, &code, &BTreeSet::from([0]), 10);
    let d = dataflow::analyze(0, &cfg, &code, &(), || true);
    assert!(!d.converged);
    assert!(d.truncated);
    assert!(d.uses.is_empty());
}
use proptest::prelude::*;
proptest! {
    #[test]
    fn constant_construction_matches_32_bit_arithmetic(hi in any::<u16>(),lo in any::<u16>()) {
        let d=run(&[dform(15,3,0,hi),dform(24,3,3,lo),0x4e800020]);
        prop_assert_eq!(written(&d,0x1004,Register::Gpr(3)),Value::constant(((hi as u32)<<16)|lo as u32));
    }
}

#[test]
fn controlled_stack_forwarding_requires_contract_and_kills_aliases() {
    struct Controlled;
    impl dataflow::ValueModel for Controlled {
        fn controlled_stack(&self) -> bool {
            true
        }
    }
    let base = [
        0x9421ffe0, 0x38601234, 0x90610008, 0x80810008, 0x7c8903a6, 0x4e800420,
    ];
    let analyze = |words: &[u32], controlled: bool| {
        let code: BTreeMap<_, _> = words
            .iter()
            .enumerate()
            .map(|(n, w)| {
                let pc = 0x1000 + n as u32 * 4;
                let mut decoded = decode(pc, *w);
                if *w == 0 {
                    decoded.control_flow = woomy_ppc::Flow::Next;
                }
                (pc, decoded)
            })
            .collect();
        let cfg = build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 1024);
        if controlled {
            dataflow::analyze(0x1000, &cfg, &code, &Controlled, || false)
        } else {
            dataflow::analyze(0x1000, &cfg, &code, &(), || false)
        }
    };
    assert_eq!(
        analyze(&base, true)
            .used(0x1014, Register::Ctr)
            .unwrap()
            .value,
        Value::constant(0x1234)
    );
    assert_eq!(
        analyze(&base, false)
            .used(0x1014, Register::Ctr)
            .unwrap()
            .value,
        Value::Unknown
    );
    for barrier in [0x90650000, 0x98610009, 0x48000101, 0u32] {
        let words = [
            base[0], base[1], base[2], barrier, base[3], base[4], base[5],
        ];
        assert_eq!(
            analyze(&words, true)
                .used(0x1018, Register::Ctr)
                .unwrap()
                .value,
            Value::Unknown
        );
    }
}
