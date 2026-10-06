mod support;
use woomy_codegen::{compiler::NativeCompiler, generate};
#[cfg(windows)]
use woomy_codegen::validation;

#[test]
fn deterministic_ir_only_emission_and_unsupported_diagnostics() {
    let (mut f, _) = support::fixture(&support::programs()[0].1);
    let a = generate(&f);
    assert_eq!(a.source, generate(&f).source);
    assert_eq!(a.manifest.status, "GENERATED");
    f.id = "../../evil; @args /Fe:dump.exe".into();
    let b = generate(&f);
    assert!(!b.source.unwrap().contains(&f.id));
    let (unknown, _) = support::fixture(&[0xffff_ffff]);
    let result = generate(&unknown);
    assert!(result.source.is_none());
    assert_eq!(result.manifest.status, "NOT_ELIGIBLE");
    assert!(!result.manifest.diagnostics.is_empty());
    let (call, _) = support::fixture(&[0x4800_0101, 0x4e80_0020]);
    assert!(generate(&call).source.is_none());
    f.blocks[0].instructions[0].source.raw ^= 1;
    assert!(generate(&f).source.is_none());
}
#[test]
fn missing_toolchain_is_explicit() {
    assert!(NativeCompiler::detect_target("missing-target")
        .err()
        .unwrap()
        .starts_with("TOOLCHAIN_NOT_FOUND"));
}

#[test]
#[cfg(windows)]
fn three_way_native_execution_512_seeded_cases_per_fixture() {
    let compiler =
        NativeCompiler::detect().expect("installed MSVC required for Windows native tests");
    let temp = tempfile::tempdir().unwrap();
    for (name, words) in support::programs() {
        let (f, raw) = support::fixture_for(name, &words);
        let g = generate(&f);
        assert!(g.source.is_some(), "{name}: {:?}", g.manifest.diagnostics);
        let build = compiler.build(&g.source.expect(name), temp.path()).unwrap();
        assert!(build.executable.is_some(), "{name}: {:?}", build.record);
        let exe = temp.path().join(format!("{name}.exe"));
        std::fs::write(&exe, build.executable.unwrap()).unwrap();
        let inputs = support::states(512);
        let native = validation::run(&exe, &inputs).unwrap();
        for ((state, budget), native) in inputs.into_iter().zip(native.executions) {
            let report = validation::compare(&f, &raw, state, budget, native);
            assert_eq!(report.status, "MATCH", "{name}: {report:?}");
        }
    }
}
#[test]
#[cfg(windows)]
fn compiler_errors_faults_and_deliberate_native_corruption_are_not_success() {
    let compiler = NativeCompiler::detect().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let bad = compiler.build("int main( {", temp.path()).unwrap();
    assert_eq!(bad.record.status, "COMPILER_ERROR");
    assert!(bad.executable.is_none());
    assert!(bad.record.link.is_none());
    assert!(!bad.record.compile.unwrap().stdout.is_empty());
    let (f, raw) = support::fixture(&support::programs()[1].1);
    let source = generate(&f).source.unwrap();
    let corrupt = source.replacen("4294967295u", "4294967294u", 1);
    assert_ne!(source, corrupt);
    let build = compiler.build(&corrupt, temp.path()).unwrap();
    let exe = temp.path().join("corrupt.exe");
    std::fs::write(&exe, build.executable.unwrap()).unwrap();
    let inputs = support::states(1);
    let native = validation::run(&exe, &inputs).unwrap().executions.remove(0);
    assert_eq!(
        validation::compare(&f, &raw, inputs[0].0.clone(), 128, native).status,
        "MISMATCH"
    );
    let build = compiler.build(&source, temp.path()).unwrap();
    let exe = temp.path().join("correct.exe");
    std::fs::write(&exe, build.executable.unwrap()).unwrap();
    let mut inputs = support::states(3);
    inputs[0].0.gpr[6] = 0x1fff;
    inputs[1].0.gpr[6] = 0x2040;
    inputs[2].1 = 1;
    for ((state, budget), native) in inputs
        .iter()
        .cloned()
        .zip(validation::run(&exe, &inputs).unwrap().executions)
    {
        let r = validation::compare(&f, &raw, state, budget, native);
        assert_eq!(r.status, "INCONCLUSIVE");
        assert!(r.ppc_native_equal && r.ir_native_equal);
    }
    let fault = compiler
        .build("int main(){return 23;}", temp.path())
        .unwrap();
    let exe = temp.path().join("fault.exe");
    std::fs::write(&exe, fault.executable.unwrap()).unwrap();
    assert!(validation::run(&exe, &inputs)
        .err()
        .unwrap()
        .to_string()
        .contains("NATIVE_FAULT"));
}
