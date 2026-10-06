use crate::{project::read_bounded, ProjectService, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
impl ProjectService {
    /// Recompute evidence using loader facts, without silently changing historical sites.
    pub fn audit_image_indirects(&self, module: Option<&str>) -> Result<Value> {
        let image = self.guest_image(module)?;
        let path: String = self.db.query_row(
            "SELECT path FROM modules WHERE id=?1",
            [image.module()],
            |r| r.get(0),
        )?;
        let bytes = read_bounded(&self.input(&path)?, woomy_binary::MAX_MODULE_BYTES)?;
        if crate::hash_bytes(&bytes) != image.module_hash() {
            return Err(crate::Error::Analysis("STALE_MODULE".into()));
        }
        let module = woomy_binary::parse(&bytes)?;
        let model = crate::module_model::ModuleModel::with_image(image.module(), &module, &image)?;
        let pcs = self
            .db
            .prepare("SELECT address FROM instructions WHERE module=?1 ORDER BY address")?
            .query_map([image.module()], |r| r.get::<_, u32>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let code: BTreeMap<_, _> = pcs
            .into_iter()
            .filter_map(|pc| model.instruction(pc).map(|d| (pc, d)))
            .collect();
        let functions = self.module_ir(image.module())?;
        let entries: BTreeSet<_> = functions.iter().map(|f| f.entry).collect();
        let mut results = vec![];
        for f in &functions {
            let cfg = woomy_ppc::build_cfg(f.entry, &code, &entries, 16384);
            let facts = woomy_ppc::dataflow::analyze(f.entry, &cfg, &code, &model, || false);
            for resolution in crate::control_flow::resolve(&cfg, &code, &facts, &model)
                .into_iter()
                .filter(|r| !r.is_return)
            {
                results.push(
                    json!({"function":f.id,"resolution":resolution,"converged":facts.converged}),
                );
            }
        }
        let value = json!({"image_hash":image.hash(),"versions":woomy_ir::runtime::ImageContextVersions::CURRENT,"non_return_sites":results.len(),"resolved":results.iter().filter(|r|r["resolution"]["status"]=="RESOLVED").count(),"ambiguous":results.iter().filter(|r|r["resolution"]["status"]=="AMBIGUOUS").count(),"unresolved":results.iter().filter(|r|r["resolution"]["status"]=="UNRESOLVED").count(),"sites":results,"historical_sites_updated":false,"scope":"Bounded rerun over known instructions and function entries with immutable loader facts; no historical evidence overwritten"});
        self.output.write_atomic(
            "reports/cafe-indirect-audit.json",
            &serde_json::to_vec_pretty(&value)?,
        )?;
        Ok(value)
    }
}
