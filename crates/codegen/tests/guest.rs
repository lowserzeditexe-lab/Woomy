#[path = "support/guest.rs"]
mod guest;
use woomy_codegen::generate_program;
#[cfg(windows)]
use woomy_codegen::{compiler::NativeCompiler, validation};
#[test]
#[cfg(windows)]
fn multi_function_native_preserves_complete_shared_state() {
    let compiler = NativeCompiler::detect().unwrap();
    let temp = tempfile::tempdir().unwrap();
    for name in guest::names() {
        let (p, code) = guest::fixture(name);
        let source = generate_program(&p).unwrap().source.unwrap();
        let build = compiler.build(&source, temp.path()).unwrap();
        assert!(build.executable.is_some(), "{name}: {:?}", build.record);
        let exe = temp.path().join(format!("{name}.exe"));
        std::fs::write(&exe, build.executable.unwrap()).unwrap();
        let inputs = guest::states(&p, &code, name, 512);
        for ((state, budget), native) in inputs.into_iter().zip(
            validation::run(&exe, &guest::states(&p, &code, name, 512))
                .unwrap()
                .executions,
        ) {
            let original = state.clone();
            let r = validation::compare_program(&p, &code, state, budget, native);
            assert_eq!(r.status, "MATCH", "{name}: {r:?}");
            assert_eq!(r.native.state.gpr[1], original.gpr[1]);
            assert_eq!(r.native.state.lr, original.lr);
            assert_eq!(r.native.state.fpscr, original.fpscr);
            assert_eq!(r.native.state.gpr[31], original.gpr[31]);
            if name == "direct" || name == "test_add" {
                assert_eq!(
                    r.native.state.gpr[3],
                    original.gpr[3].wrapping_add(original.gpr[4])
                );
            }
            if name == "memory_widths" {
                assert_eq!(r.native.state.gpr[7], original.gpr[3] & 255);
                assert_eq!(r.native.state.gpr[8], original.gpr[4] & 65535);
                assert_eq!(r.native.state.gpr[9], original.gpr[3]);
                assert_eq!(r.native.state.fpr[1], original.fpr[0]);
            }
        }
    }
}

#[test]
fn contracts_fixed_points_and_invalid_registrations_are_explicit() {
    use woomy_ir::{calls::*, runtime::*, Operation};
    let (p, _) = guest::fixture("nested");
    let summaries = summarize(&p.functions);
    assert_eq!(summaries, summarize(&p.functions));
    let a = &summaries[&p.functions[0].id];
    assert!(a.fixed_point && a.effects.memory_write);
    assert!(a
        .effects
        .registers_written
        .contains(&woomy_ppc::Register::Gpr(3)));
    assert!(a
        .registers_preserved
        .contains(&woomy_ppc::Register::Gpr(31)));
    assert!(!a.registers_preserved.contains(&woomy_ppc::Register::Gpr(1)));
    assert!(a
        .stack_observations
        .iter()
        .any(|s| s.entry_sp_offset == Some(-32)));
    assert!(a
        .stack_observations
        .iter()
        .any(|s| s.register == Some(woomy_ppc::Register::Lr) && s.operation == "STACK_STORE"));
    assert!(a.stack_observations.iter().any(
        |s| s.register == Some(woomy_ppc::Register::Lr) && s.operation == "REGISTER_FROM_STACK"
    ));
    assert_eq!(
        contracts(&p.functions[0], &p.functions, &summaries)[0]
            .effects
            .certainty,
        Certainty::Known
    );
    let (recursive, _) = guest::fixture("recursive");
    assert!(summarize(&recursive.functions)
        .values()
        .all(|s| s.fixed_point));
    let (import, _) = guest::fixture("test_add");
    let sums = summarize(&import.functions);
    assert!(sums.values().all(|s| s.registers_preserved.is_empty()));
    assert_eq!(
        contracts(&import.functions[0], &import.functions, &sums)[0]
            .effects
            .certainty,
        Certainty::Unknown
    );
    let mut bad = p.clone();
    bad.version = 0;
    assert!(generate_program(&bad).is_err());
    let mut bad = p.clone();
    bad.functions.push(bad.functions[0].clone());
    assert!(generate_program(&bad).is_err());
    let mut bad = p.clone();
    bad.functions.pop();
    assert!(generate_program(&bad).is_err());
    let mut bad = p.clone();
    bad.functions[1].ir_version = 0;
    assert!(generate_program(&bad).is_err());
    let mut bad = p.clone();
    bad.functions[1].blocks[0].instructions[0].operation = Operation::UnknownSideEffect {
        reason: "test".into(),
        effects: woomy_ir::Effects {
            uses: Default::default(),
            defs: Default::default(),
            memory: woomy_ir::MemoryEffect::None,
            details: vec![],
        },
    };
    assert!(generate_program(&bad).is_err());
    assert!(NoRuntime
        .resolve_import(&Import {
            library: "woomy_test".into(),
            symbol: "test_add".into()
        })
        .is_none());
    assert!(TestRuntime
        .resolve_import(&Import {
            library: "coreinit".into(),
            symbol: "test_add".into()
        })
        .is_none());
}
#[test]
#[cfg(windows)]
fn runtime_faults_limits_unknown_imports_and_corrupt_calls_are_never_matches() {
    use woomy_ir::{execution::Exit, runtime::*};
    let compiler = NativeCompiler::detect().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let (mut p, code) = guest::fixture("recursive");
    p.limits.depth = 3;
    let mut cases = guest::states(&p, &code, "recursive", 8);
    cases[0].0.gpr[3] = 10; // depth bound
    cases[1].0.gpr[1] = 0x3000; // stack underflow
    cases[2]
        .0
        .guest_memory
        .regions
        .last_mut()
        .unwrap()
        .permissions = READ; // readonly stack
    cases[3].0.guest_memory.regions[0].permissions = READ; // non executable
    cases[4].1 = 1; // instruction bound
    cases[5].0.gpr[1] = 0x31fd; // unaligned stack access
    cases[6].0.gpr[1] = 0xfffffff0; // wrapped address not mapped
    cases[7].0.gpr[3] = 100;
    let build = compiler
        .build(&generate_program(&p).unwrap().source.unwrap(), temp.path())
        .unwrap();
    let exe = temp.path().join("limits.exe");
    std::fs::write(&exe, build.executable.unwrap()).unwrap();
    for ((initial, budget), native) in cases
        .clone()
        .into_iter()
        .zip(validation::run(&exe, &cases).unwrap().executions)
    {
        let r = validation::compare_program(&p, &code, initial, budget, native);
        assert_eq!(r.status, "INCONCLUSIVE", "{r:?}");
        assert!(r.ppc_native_equal && r.ir_native_equal);
    }
    for (name, configure) in [
        ("test_add", 0),
        ("ctr_call", 1),
        ("direct", 2),
        ("direct", 3),
    ] {
        let (mut p, mut code) = guest::fixture(name);
        if configure == 0 {
            p.provider = ProviderKind::NoRuntime;
        }
        if configure == 1 {
            // The reference and IR both see an explicitly constructed unregistered CTR target.
            let i = p.functions[0].blocks[0]
                .instructions
                .iter_mut()
                .find(|i| matches!(i.operation, woomy_ir::Operation::Constant { bits: 4352 }))
                .unwrap();
            i.operation = woomy_ir::Operation::Constant { bits: 0x7000 };
            code.insert(0x100c, 0x39807000);
        }
        if configure == 2 {
            p.limits.calls = 0;
        }
        if configure == 3 {
            p.limits.trace = 0;
        }
        let source = generate_program(&p).unwrap().source.unwrap();
        let build = compiler.build(&source, temp.path()).unwrap();
        let exe = temp.path().join(format!("negative{configure}.exe"));
        std::fs::write(&exe, build.executable.unwrap()).unwrap();
        let inputs = guest::states(&p, &code, name, 1);
        let native = validation::run(&exe, &inputs).unwrap().executions.remove(0);
        let r = validation::compare_program(&p, &code, inputs[0].0.clone(), 512, native);
        assert_eq!(r.status, "INCONCLUSIVE", "{r:?}");
        assert!(r.ppc_native_equal && r.ir_native_equal);
    }
    let (p, code) = guest::fixture("multiple");
    let source = generate_program(&p).unwrap().source.unwrap();
    let corrupt = source.replacen("4352u, 0u, block", "4608u, 0u, block", 1);
    assert_ne!(source, corrupt);
    let build = compiler.build(&corrupt, temp.path()).unwrap();
    let exe = temp.path().join("corrupt-call.exe");
    std::fs::write(&exe, build.executable.unwrap()).unwrap();
    let inputs = guest::states(&p, &code, "multiple", 1);
    let r = validation::compare_program(
        &p,
        &code,
        inputs[0].0.clone(),
        512,
        validation::run(&exe, &inputs).unwrap().executions.remove(0),
    );
    assert_eq!(r.status, "MISMATCH");
    assert!(matches!(r.ppc.exit, Exit::Returned { .. }));
}
#[test]
fn region_boundaries_permissions_and_big_endian_known_answers() {
    use woomy_ir::{execution::Machine, runtime::*};
    let mut s = Machine::new(0, vec![]);
    s.guest_memory.regions.push(Region {
        base: 0xfffffff8,
        permissions: READ | WRITE,
        kind: 2,
        provenance: "TEST".into(),
        bytes: vec![0; 8],
    });
    assert!(s.guest_memory.valid());
    s.store_width(0xfffffff8, 0x0123456789abcdef, 8).unwrap();
    assert_eq!(s.load_width(0xfffffff8, 1), Some(1));
    assert_eq!(s.load_width(0xfffffffa, 2), Some(0x4567));
    assert_eq!(s.load_width(0xfffffffc, 4), Some(0x89abcdef));
    assert_eq!(s.load_width(0xfffffffc, 8), None);
    s.guest_memory.regions[0].permissions = READ;
    assert!(s.store_width(0xfffffff8, 0, 1).is_none());
    s.guest_memory
        .regions
        .push(s.guest_memory.regions[0].clone());
    assert!(!s.guest_memory.valid());
}
