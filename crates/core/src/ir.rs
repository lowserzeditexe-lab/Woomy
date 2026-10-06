use crate::{Error, ProjectService, Result};
use rusqlite::{params, Transaction};
use serde_json::{json, Value};

pub(crate) fn persist(tx: &Transaction<'_>, mut function: woomy_ir::Function) -> Result<()> {
    let mut artifact = serde_json::to_string(&function)?;
    if artifact.len() > 16 * 1024 * 1024 {
        function.blocks.clear();
        function.status = "LIFT_LIMIT".into();
        function
            .limitations
            .push("Serialized IR exceeds 16 MiB per-function budget".into());
        artifact = serde_json::to_string(&function)?;
    }
    let report = woomy_ir::verify(&function);
    tx.execute(
        "INSERT INTO ir_functions VALUES(?1,?2,?3,?4,?5)",
        params![
            function.id,
            woomy_ir::IR_VERSION,
            function.module_hash,
            function.status,
            artifact
        ],
    )?;
    tx.execute(
        "INSERT INTO ir_verifications VALUES(?1,?2,?3,?4,?5)",
        params![
            function.id,
            woomy_ir::IR_VERSION,
            report.status,
            report.unknown_operations,
            serde_json::to_string(&report)?
        ],
    )?;
    Ok(())
}
impl ProjectService {
    pub(crate) fn read_ir(&self, function: &str) -> Result<woomy_ir::Function> {
        self.refresh_module_hashes()?;
        let metadata = self.require_function(function)?;
        let module = metadata["module"]
            .as_str()
            .ok_or_else(|| Error::Analysis("IR module missing".into()))?;
        let (artifact,hash,version):(String,String,u32)=self.db.query_row("SELECT i.artifact,m.hash,m.ir_version FROM ir_functions i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE i.function=?1 AND m.status!='STALE'",[function],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if artifact.len() > 16 * 1024 * 1024 {
            return Err(Error::Analysis("IR artifact exceeds budget".into()));
        }
        let ir: woomy_ir::Function = serde_json::from_str(&artifact)?;
        if ir.module != module
            || ir.id != function
            || ir.module_hash != hash
            || ir.ir_version != woomy_ir::IR_VERSION
            || version != woomy_ir::IR_VERSION
        {
            return Err(Error::Analysis(
                "IR stale or identity mismatch; analyze again".into(),
            ));
        }
        Ok(ir)
    }
    pub fn inspect_ir(&self, function: &str, limit: usize, offset: usize) -> Result<Value> {
        if limit == 0 || limit > 1000 || offset > 10_000_000 {
            return Err(Error::Config("IR query budget exceeded".into()));
        }
        let mut ir = self.read_ir(function)?;
        let verification = woomy_ir::verify(&ir);
        let count = ir.blocks.len();
        let blocks: Vec<_> = ir.blocks.into_iter().skip(offset).take(limit).collect();
        ir.blocks = blocks;
        Ok(
            json!({"function":ir,"verification":verification,"pagination":{"unit":"blocks","limit":limit,"offset":offset,"total":count,"has_more":offset.saturating_add(limit)<count},"scope":"Verification covers full stored function, not the paginated display; execution equivalence is not validated"}),
        )
    }
    pub fn verify_ir(&self, function: &str) -> Result<Value> {
        let ir = self.read_ir(function)?;
        let mut report = woomy_ir::verify(&ir);
        let count: usize = self.db.query_row(
            "SELECT count(*) FROM function_instructions WHERE function=?1",
            [function],
            |r| r.get(0),
        )?;
        if count != ir.blocks.iter().map(|b| b.sources.len()).sum::<usize>() {
            report.structural_valid = false;
            report.status = "INVALID".into();
            report.diagnostics.push(woomy_ir::Diagnostic {
                code: "DATABASE_COVERAGE".into(),
                block: None,
                instruction: None,
                message: "IR does not cover exactly the analyzed PPC membership".into(),
            });
        }
        // Check source words against the deterministic database as well as the
        // self-contained artifact. JSON alone is not an authenticity proof.
        for block in &ir.blocks {
            for source in &block.sources {
                let matches:bool=self.db.query_row("SELECT EXISTS(SELECT 1 FROM instructions i JOIN function_instructions fi ON fi.address=i.address JOIN functions f ON f.id=fi.function AND f.module=i.module WHERE f.id=?1 AND i.address=?2 AND i.raw=?3)",params![function,source.address,source.raw],|r|r.get(0))?;
                if !matches {
                    report.structural_valid = false;
                    report.status = "INVALID".into();
                    if report.diagnostics.len() < 1024 {
                        report.diagnostics.push(woomy_ir::Diagnostic {
                            code: "DATABASE_PROVENANCE".into(),
                            block: Some(block.id),
                            instruction: None,
                            message: "Source word is not a member of the analyzed function".into(),
                        });
                    }
                }
            }
        }
        self.db.execute("UPDATE ir_verifications SET status=?2,unknown_operations=?3,report=?4 WHERE function=?1",params![function,report.status,report.unknown_operations,serde_json::to_string(&report)?])?;
        Ok(json!({"function":function,"verification":report}))
    }
}
