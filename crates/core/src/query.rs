use crate::{Error, ProjectService, Result};
use rusqlite::{params, types::ValueRef, ToSql};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Value>,
    pub limit: usize,
    pub offset: usize,
    pub has_more: bool,
}
impl ProjectService {
    pub fn status(&self) -> Result<Value> {
        self.refresh_module_hashes()?;
        let count = |sql: &str| -> Result<i64> { Ok(self.db.query_row(sql, [], |r| r.get(0))?) };
        let analyzed = count("SELECT count(*) FROM modules WHERE status='ANALYZED_PARTIAL'")? > 0;
        Ok(
            json!({"project":self.config.project.name,"game":self.metadata,"database":"OK","dump":self.dump_change_state()?,
            "files":count("SELECT count(*) FROM files")?,"modules":count("SELECT count(*) FROM modules")?,"stale_modules":count("SELECT count(*) FROM modules WHERE status='STALE'")?,
            "functions":count("SELECT count(*) FROM functions f JOIN modules m ON f.module=m.id WHERE m.status!='STALE'")?,
            "instructions":count("SELECT count(*) FROM instructions i JOIN modules m ON i.module=m.id WHERE m.status!='STALE'")?,
            "unknown_instructions":count("SELECT count(*) FROM instructions i JOIN modules m ON i.module=m.id WHERE m.status!='STALE' AND i.status IN ('UNKNOWN','INVALID')")?,
            "relocation_dependent_instructions":count("SELECT count(*) FROM instructions i JOIN modules m ON i.module=m.id WHERE m.status!='STALE' AND i.status IN ('RELOCATION_REQUIRED','RELOCATION_MODELED','IMPORT_REFERENCE')")?,
            "blocks":count("SELECT count(*) FROM basic_blocks b JOIN functions f ON b.function=f.id JOIN modules m ON f.module=m.id WHERE m.status!='STALE'")?,
            "unresolved_indirect":count("SELECT count(*) FROM indirect_sites i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND i.is_return=0 AND i.status!='RESOLVED'")?,"resolved_indirect":count("SELECT count(*) FROM indirect_sites i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND i.is_return=0 AND i.status='RESOLVED'")?,"interrupted_runs":count("SELECT count(*) FROM analysis_runs WHERE status IN ('INTERRUPTED','PAUSED','CANCELLED')")?,
            "rpx":if analyzed {"PARSED (derived Cafe image and relocation checks: image status)"}else{"NOT ANALYZED / STALE"},"ppc":if analyzed {"PARTIAL (reachable subset)"}else{"NOT ANALYZED / STALE"},"cfg":if analyzed {"PARTIAL (conservative)"}else{"NOT ANALYZED / STALE"},"dataflow":if analyzed {"PARTIAL (liveness, reaching definitions, scalar/symbolic values)"}else{"NOT ANALYZED / STALE"},
            "ir":if analyzed {"PARTIAL (typed machine IR; structural verification only)"} else {"NOT ANALYZED / STALE"},"ir_functions":count("SELECT count(*) FROM ir_functions i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE'")?,"ir_structurally_valid":count("SELECT count(*) FROM ir_verifications v JOIN functions f ON f.id=v.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND v.status='STRUCTURALLY_VALID'")?,"cpp":"PARTIAL (IR-only scalar backend; Windows x64 MSVC)","native":self.native_counts()?,"runtime":self.runtime_status()?,"ai":self.config.ai,
            "last_run":self.rows("SELECT * FROM analysis_runs ORDER BY id DESC LIMIT 1",&[])?.first()}),
        )
    }
    /// Only this service selects SQL; AI/user arguments are bound parameters.
    pub fn list(&self, kind: &str, filter: &str, limit: usize, offset: usize) -> Result<Page> {
        if limit == 0 || limit > 1000 || offset > 10_000_000 || filter.len() > 4096 {
            return Err(Error::Config("query budget exceeded".into()));
        }
        if [
            "evidence",
            "disasm",
            "cfg",
            "blocks",
            "callers",
            "callees",
            "dataflow",
            "values",
            "definitions",
            "dataflow_status",
            "indirect_targets",
            "ir_status",
        ]
        .contains(&kind)
            || (kind == "unresolved" && !filter.is_empty())
        {
            self.require_function(filter)?;
        }
        let sql=match kind {
            "ir_status"=>"SELECT i.function,i.ir_version,i.status,v.status AS verification,v.unknown_operations FROM ir_functions i JOIN ir_verifications v ON v.function=i.function WHERE i.function=?3 LIMIT ?1 OFFSET ?2",
            "values"=>"SELECT * FROM register_values WHERE function=?3 ORDER BY address,register LIMIT ?1 OFFSET ?2",
            "definitions"=>"SELECT * FROM reaching_definitions WHERE function=?3 ORDER BY use_address,register,definition_address,kind LIMIT ?1 OFFSET ?2",
            "dataflow_status"=>"SELECT * FROM dataflow_runs WHERE function=?3 LIMIT ?1 OFFSET ?2",
            "indirect"=>"SELECT i.* FROM indirect_sites i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND (?3='' OR i.function=?3) ORDER BY i.function,i.address LIMIT ?1 OFFSET ?2",
            "indirect_targets"=>"SELECT t.* FROM indirect_targets t JOIN functions f ON f.id=t.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND t.function=?3 ORDER BY site,ordinal LIMIT ?1 OFFSET ?2",
            "jump_tables"=>"SELECT j.* FROM jump_tables j JOIN modules m ON m.id=j.module WHERE m.status!='STALE' AND (?3='' OR j.function=?3) ORDER BY j.module,j.site LIMIT ?1 OFFSET ?2",
            "modules"=>"SELECT * FROM modules WHERE (?3='' OR id=?3) ORDER BY id LIMIT ?1 OFFSET ?2",
            "sections"=>"SELECT * FROM sections WHERE module=?3 ORDER BY idx LIMIT ?1 OFFSET ?2",
            "functions"=>"SELECT f.* FROM functions f JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND (?3='' OR instr(f.id,?3)>0 OR instr(coalesce(f.name,''),?3)>0) ORDER BY f.id LIMIT ?1 OFFSET ?2",
            "function"=>"SELECT f.* FROM functions f JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND f.id=?3 LIMIT ?1 OFFSET ?2",
            "evidence"=>"SELECT * FROM function_evidence WHERE function=?3 ORDER BY kind,source LIMIT ?1 OFFSET ?2",
            "disasm"=>"SELECT i.* FROM instructions i JOIN functions f ON f.module=i.module JOIN function_instructions fi ON fi.function=f.id AND fi.address=i.address JOIN modules m ON m.id=i.module WHERE f.id=?3 AND m.status!='STALE' ORDER BY i.address LIMIT ?1 OFFSET ?2",
            "cfg"=>"SELECT * FROM cfg_edges WHERE function=?3 ORDER BY source,kind,target LIMIT ?1 OFFSET ?2",
            "blocks"=>"SELECT * FROM basic_blocks WHERE function=?3 ORDER BY start LIMIT ?1 OFFSET ?2",
            "callers"=>"SELECT * FROM call_edges WHERE callee=?3 ORDER BY caller,address LIMIT ?1 OFFSET ?2",
            "callees"=>"SELECT * FROM call_sites WHERE function=?3 ORDER BY address LIMIT ?1 OFFSET ?2",
            "imports"=>"SELECT i.*,s.name AS symbol,s.address FROM imports i LEFT JOIN symbols s ON s.module=i.module AND s.section=i.section JOIN modules m ON m.id=i.module WHERE m.status!='STALE' AND (?3='' OR i.module=?3) ORDER BY i.module,i.section,s.idx LIMIT ?1 OFFSET ?2",
            "exports"=>"SELECT e.* FROM exports e JOIN modules m ON m.id=e.module WHERE m.status!='STALE' AND (?3='' OR e.module=?3) ORDER BY e.module,e.address,e.name LIMIT ?1 OFFSET ?2",
            "relocations"=>"SELECT r.*,s.name AS symbol_name FROM relocations r LEFT JOIN symbols s ON s.module=r.module AND s.symbol_table=r.symbol_table AND s.idx=r.symbol JOIN modules m ON m.id=r.module WHERE m.status!='STALE' AND r.module=?3 ORDER BY r.offset,r.section LIMIT ?1 OFFSET ?2",
            "xref_to"=>"SELECT x.* FROM cross_references x JOIN modules m ON m.id=x.module WHERE m.status!='STALE' AND target=CAST(?3 AS INTEGER) ORDER BY x.module,source LIMIT ?1 OFFSET ?2",
            "xref_from"=>"SELECT x.* FROM cross_references x JOIN modules m ON m.id=x.module WHERE m.status!='STALE' AND source=CAST(?3 AS INTEGER) ORDER BY x.module,target LIMIT ?1 OFFSET ?2",
            "unresolved"=>"SELECT i.* FROM indirect_sites i JOIN functions f ON f.id=i.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND i.status!='RESOLVED' AND i.is_return=0 AND (?3='' OR i.function=?3) ORDER BY i.function,i.address LIMIT ?1 OFFSET ?2",
            "dataflow"=>"SELECT * FROM block_liveness WHERE function=?3 ORDER BY block LIMIT ?1 OFFSET ?2",
            "function_xrefs"=>"SELECT x.* FROM cross_references x JOIN functions f ON f.module=x.module JOIN function_instructions fi ON fi.function=f.id AND fi.address=x.source WHERE f.id=?3 ORDER BY x.source,x.kind,x.target LIMIT ?1 OFFSET ?2",
            "problems"=>"SELECT * FROM problems WHERE (?3='' OR function=?3) ORDER BY id LIMIT ?1 OFFSET ?2",
            "unknown"=>"SELECT u.* FROM unknown_opcodes u JOIN modules m ON m.id=u.module WHERE m.status!='STALE' ORDER BY count DESC,u.module,opcode,extended_opcode LIMIT ?1 OFFSET ?2",
            "runs"=>"SELECT * FROM analysis_runs ORDER BY id DESC LIMIT ?1 OFFSET ?2",
            "findings"=>"SELECT * FROM ai_findings WHERE (?3='' OR CAST(id AS TEXT)=?3) ORDER BY id LIMIT ?1 OFFSET ?2",
            _=>return Err(Error::Config("unknown query".into()))
        };
        let mut stmt = self.db.prepare(sql)?;
        let param_count = stmt.parameter_count();
        let max = (limit + 1) as i64;
        let off = offset as i64;
        let params: Vec<&dyn ToSql> = if param_count == 3 {
            vec![&max, &off, &filter]
        } else {
            vec![&max, &off]
        };
        let mut rows = stmt.query(params.as_slice())?;
        let mut items = vec![];
        while let Some(row) = rows.next()? {
            items.push(row_value(row)?);
        }
        let has_more = items.len() > limit;
        items.truncate(limit);
        Ok(Page {
            items,
            limit,
            offset,
            has_more,
        })
    }
    pub fn rows(&self, sql: &str, parameters: &[&dyn ToSql]) -> Result<Vec<Value>> {
        let mut s = self.db.prepare(sql)?;
        let mut r = s.query(parameters)?;
        let mut out = vec![];
        while let Some(row) = r.next()? {
            out.push(row_value(row)?);
            if out.len() > 1001 {
                return Err(Error::Config("query exceeds row budget".into()));
            }
        }
        Ok(out)
    }
    pub fn require_function(&self, id: &str) -> Result<Value> {
        self.list("function", id, 1, 0)?
            .items
            .into_iter()
            .next()
            .ok_or_else(|| Error::Analysis("function not found or stale".into()))
    }
    pub fn interrupted(&self) -> Result<bool> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM analysis_runs WHERE status IN ('INTERRUPTED','PAUSED','CANCELLED') AND id=(SELECT max(id) FROM analysis_runs))",[],|r|r.get(0))?)
    }
    pub fn mark_cancelled(&self) -> Result<()> {
        self.db.execute(
            "UPDATE analysis_runs SET status='CANCELLED' WHERE status IN ('PAUSED','INTERRUPTED')",
            params![],
        )?;
        Ok(())
    }
}
fn row_value(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let mut result = serde_json::Map::new();
    for (i, name) in row.as_ref().column_names().iter().enumerate() {
        let mut value = match row.get_ref(i)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(n) => json!(n),
            ValueRef::Real(n) => json!(n),
            ValueRef::Text(s) => Value::String(String::from_utf8_lossy(s).into_owned()),
            ValueRef::Blob(_) => Value::String("<binary omitted>".into()),
        };
        if [
            "operands",
            "semantics",
            "control_flow",
            "live_in",
            "live_out",
            "value",
        ]
        .contains(name)
        {
            if let Some(text) = value.as_str() {
                if let Ok(parsed) = serde_json::from_str(text) {
                    value = parsed;
                }
            }
        }
        result.insert(name.to_string(), value);
    }
    Ok(Value::Object(result))
}
