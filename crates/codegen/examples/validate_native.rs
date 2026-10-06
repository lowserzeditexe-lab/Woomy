//! Reproducible synthetic fixtures only. Never reads or executes game functions.
#[path = "../tests/support/mod.rs"]
mod support;
use std::path::PathBuf;
use woomy_codegen::{compiler::NativeCompiler, generate, validation};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: validate_native <owned-output-directory>")?;
    std::fs::create_dir_all(&directory)?;
    let compiler = NativeCompiler::detect()?;
    let mut fixtures = Vec::new();
    let mut matched = 0;
    let mut total = 0;
    for (name, words) in support::programs() {
        let (ir, raw) = support::fixture_for(name, &words);
        let generated = generate(&ir);
        let source = generated.source.ok_or("CODEGEN_UNSUPPORTED")?;
        std::fs::write(directory.join(format!("{name}.cpp")), &source)?;
        let build = compiler.build(&source, &directory)?;
        std::fs::write(
            directory.join(format!("{name}.build.json")),
            serde_json::to_vec_pretty(&build.record)?,
        )?;
        let exe = directory.join(format!("{name}.exe"));
        std::fs::write(&exe, build.executable.ok_or("COMPILER_ERROR")?)?;
        let inputs = support::states(512);
        let native = validation::run(&exe, &inputs)?;
        let mut cases = Vec::new();
        for (index, ((initial, budget), native)) in
            inputs.into_iter().zip(native.executions).enumerate()
        {
            let report = validation::compare(&ir, &raw, initial.clone(), budget, native);
            total += 1;
            matched += usize::from(report.status == "MATCH");
            cases.push(serde_json::json!({"case":index,"initial":initial,"budget":budget,"comparison":report}));
        }
        fixtures.push(serde_json::json!({"fixture":name,"manifest":generated.manifest,"build":build.record,"process":native.process,"cases":cases}));
    }
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"scope":"Synthetic fixtures only; no game semantic validation","validation_version":woomy_codegen::NATIVE_VALIDATION_VERSION,"seed":0x76543210u32,"total":total,"matched":matched,"fixtures":fixtures}),
        )?,
    )?;
    println!(
        "Synthetic PPC == IR == Native: {matched}/{total} MATCH; {}",
        directory.join("report.json").display()
    );
    if matched != total {
        return Err("native semantic mismatch".into());
    }
    Ok(())
}
