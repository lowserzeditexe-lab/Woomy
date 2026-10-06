//! Read-only dump inspection; writes derived metadata only through SafeOutput.
use serde_json::json;
use woomy_core::{context::ExecutionContextBuilder, ProjectService};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args_os()
        .nth(1)
        .ok_or("usage: inspect_contexts <project-root>")?;
    let p = ProjectService::open(std::path::Path::new(&root))?;
    let image = p.guest_image(None)?;
    let ids =
        p.db.prepare("SELECT function FROM ir_functions WHERE status='LIFTED' ORDER BY function")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
    let mut contexts = vec![];
    for id in ids {
        let artifact: String = p.db.query_row(
            "SELECT artifact FROM ir_functions WHERE function=?1",
            [&id],
            |r| r.get(0),
        )?;
        let f = serde_json::from_str(&artifact)?;
        let compiled =
            p.codegen_status(&id)?["build"]["status"] == "COMPILED_NOT_SEMANTICALLY_VALIDATED";
        let context = ExecutionContextBuilder::new(&image).analyze(&f, compiled);
        contexts.push(context);
    }
    let report = json!({"image":image.status(),"regions":image.regions(),"contexts":contexts,"relocation_blockers":image.relocations().iter().filter(|r|r.state!="APPLIED_PROVEN_LOCAL").collect::<Vec<_>>()});
    p.output.write_atomic(
        "reports/cafe-context-audit.json",
        &serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{}",
        json!({"image":image.status(),"contexts":contexts.len(),"register_only_synthetic_eligible":contexts.iter().filter(|c|c.register_only_synthetic_eligible).map(|c|&c.function).collect::<Vec<_>>(),"invalid_relocations":image.relocations().iter().filter(|r|r.state=="INVALID").collect::<Vec<_>>() })
    );
    Ok(())
}
