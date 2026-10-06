use crate::{
    cafe::CafeGuestImage, context::ExecutionContextBuilder, project::read_bounded, Error,
    ProjectService, Result,
};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use woomy_ir::{
    calls::{Contract, Target},
    runtime::ImageContextVersions,
    Function,
};

impl ProjectService {
    pub fn guest_image(&self, module: Option<&str>) -> Result<CafeGuestImage> {
        self.refresh_module_hashes()?;
        let rows=self.db.prepare("SELECT id,path,hash,kind FROM modules WHERE status!='STALE' AND (?1 IS NULL OR id=?1) ORDER BY id")?.query_map([module],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        if rows.len() != 1 {
            return Err(Error::Config("Select exactly one current module".into()));
        }
        let (id, path, hash, kind) = &rows[0];
        let bytes = read_bounded(&self.input(path)?, woomy_binary::MAX_MODULE_BYTES)?;
        if crate::hash_bytes(&bytes) != *hash {
            return Err(Error::Analysis("STALE_MODULE".into()));
        }
        let parsed = woomy_binary::parse(&bytes)?;
        let image = CafeGuestImage::load(id, hash, kind, &parsed, bytes.len())?;
        self.persist_product("IMAGE", id, id, image.hash(), &image.status())?;
        Ok(image)
    }
    pub(crate) fn persist_product(
        &self,
        kind: &str,
        subject: &str,
        module: &str,
        key: &str,
        value: &Value,
    ) -> Result<()> {
        self.db.execute("INSERT INTO analysis_products VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(kind,subject) DO UPDATE SET module=excluded.module,cache_key=excluded.cache_key,versions=excluded.versions,artifact=excluded.artifact",params![kind,subject,module,key,serde_json::to_string(&ImageContextVersions::CURRENT)?,serde_json::to_string(value)?])?;
        Ok(())
    }
    pub fn image_query(
        &self,
        command: &str,
        module: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Value> {
        if limit == 0 || limit > 1000 || offset > 10_000_000 {
            return Err(Error::Config("image query budget".into()));
        }
        if command == "indirect-audit" {
            return self.audit_image_indirects(module);
        }
        if command == "link-audit" {
            return self.link_audit(module);
        }
        let image = self.guest_image(module)?;
        match command {
            "status" => Ok(image.status()),
            "verify" => {
                image.verify()?;
                Ok(
                    json!({"image_hash":image.hash(),"verification":"STRUCTURALLY_VALID","execution_ready":false,"relocation_status":image.status()["relocations"]}),
                )
            }
            "regions" => Ok(
                json!({"image_hash":image.hash(),"items":image.regions().iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":image.regions().len(),"has_more":offset.saturating_add(limit)<image.regions().len()}),
            ),
            "relocations" => Ok(
                json!({"image_hash":image.hash(),"items":image.relocations().iter().skip(offset).take(limit).collect::<Vec<_>>(),"total":image.relocations().len(),"has_more":offset.saturating_add(limit)<image.relocations().len()}),
            ),
            _ => Err(Error::Config(
                "image status|regions|relocations|verify [module-id]".into(),
            )),
        }
    }
    pub fn execution_context(&self, id: &str) -> Result<Value> {
        let ir = self.read_ir(id)?;
        let image = self.guest_image(Some(&ir.module))?;
        let compiled =
            self.codegen_status(id)?["build"]["status"] == "COMPILED_NOT_SEMANTICALLY_VALIDATED";
        let context = ExecutionContextBuilder::new(&image).analyze(&ir, compiled);
        let value = serde_json::to_value(context)?;
        let key = crate::hash_bytes(&serde_json::to_vec(&(
            image.hash(),
            &ir,
            ImageContextVersions::CURRENT,
        ))?);
        self.persist_product("CONTEXT", id, &ir.module, &key, &value)?;
        Ok(value)
    }
    pub(crate) fn module_ir(&self, module: &str) -> Result<Vec<Function>> {
        let texts=self.db.prepare("SELECT i.artifact FROM ir_functions i JOIN functions f ON f.id=i.function WHERE f.module=?1 ORDER BY f.address")?.query_map([module],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        if texts.len() > 4096 || texts.iter().map(String::len).sum::<usize>() > 256 * 1024 * 1024 {
            return Err(Error::Analysis("closure IR budget".into()));
        }
        texts
            .iter()
            .map(|text| {
                if text.len() > 16 * 1024 * 1024 {
                    return Err(Error::Analysis("IR budget".into()));
                }
                Ok(serde_json::from_str(text)?)
            })
            .collect()
    }
    pub fn function_closure(&self, id: &str) -> Result<Value> {
        let root = self.read_ir(id)?;
        let image = self.guest_image(Some(&root.module))?;
        let functions = self.module_ir(&root.module)?;
        let summaries = woomy_ir::calls::summarize(&functions);
        let contracts: Vec<_> = functions
            .iter()
            .flat_map(|f| woomy_ir::calls::contracts(f, &functions, &summaries))
            .collect();
        let value = plan_closure(id, &functions, &contracts, &image)?;
        let key = crate::hash_bytes(&serde_json::to_vec(&(
            image.hash(),
            &functions,
            ImageContextVersions::CURRENT,
        ))?);
        self.persist_product("CLOSURE", id, &root.module, &key, &value)?;
        self.output.write_atomic("reports/required-imports.json",&serde_json::to_vec_pretty(&json!({"root":id,"image_hash":image.hash(),"imports":value["imports"],"scope":"Known explored call graph; unknown exits may hide dependencies"}))?)?;
        Ok(value)
    }
    pub fn import_catalog(&self) -> Result<Value> {
        self.refresh_module_hashes()?;
        let roots=self.db.prepare("SELECT f.id FROM functions f JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND f.address=m.entry_point ORDER BY f.id")?.query_map([],|r|r.get::<_,String>(0))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut statement=self.db.prepare("SELECT c.artifact FROM call_contracts c JOIN functions f ON f.id=c.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' AND c.target_kind='INTERNAL' ORDER BY c.function,c.site LIMIT 262145")?;
        for (index, row) in statement
            .query_map([], |r| r.get::<_, String>(0))?
            .enumerate()
        {
            let text = row?;
            if index >= 262144 || text.len() > 1024 * 1024 {
                return Err(Error::Analysis("import reachability budget".into()));
            }
            let contract: Contract = serde_json::from_str(&text)?;
            if let Target::InternalFunction {
                function: Some(target),
                ..
            } = contract.target
            {
                graph.entry(contract.caller).or_default().push(target);
            }
        }
        let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut queue: VecDeque<_> = roots
            .into_iter()
            .map(|root| (root.clone(), vec![root]))
            .collect();
        while let Some((id, path)) = queue.pop_front() {
            if paths.contains_key(&id) {
                continue;
            }
            if paths.len() >= 4096 {
                return Err(Error::Analysis(
                    "import reachability function budget".into(),
                ));
            }
            if let Some(callees) = graph.get(&id) {
                for callee in callees {
                    if !paths.contains_key(callee) {
                        let mut next = path.clone();
                        next.push(callee.clone());
                        queue.push_back((callee.clone(), next));
                    }
                }
            }
            paths.insert(id, path);
        }
        let mut groups: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
        let mut statement=self.db.prepare("SELECT r.library,r.symbol,r.function,r.site FROM runtime_imports r JOIN functions f ON f.id=r.function JOIN modules m ON m.id=f.module WHERE m.status!='STALE' ORDER BY r.library,r.symbol,r.function,r.site")?;
        for row in statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })? {
            let (library, symbol, caller, site) = row?;
            groups
                .entry((library, symbol))
                .or_default()
                .push(json!({"caller":caller,"site":site,"entry_reachable":paths.contains_key(&caller),"first_reachable_path":paths.get(&caller)}));
        }
        let libraries: BTreeSet<_> = groups.keys().map(|(l, _)| l.clone()).collect();
        let items:Vec<_>=groups.into_iter().map(|((library,symbol),sites)|json!({"library":library,"symbol":symbol,"callers":sites.iter().filter_map(|s|s["caller"].as_str()).collect::<BTreeSet<_>>(),"entry_reachable":sites.iter().any(|s|s["entry_reachable"]==true),"sites":sites,"effect_knowledge":"UNKNOWN","provider_available":false,"provider_scope":"GAME_FUNCTION_REFERENCE_CONTEXT","model_contract":manmenmi::import_contract(&woomy_ir::runtime::Import{library:library.clone(),symbol:symbol.clone()}),"implementation_status":"UNRESOLVED_IMPORT","entry_reachability_scope":"Known internal call graph from module entries; unknown exits may hide additional dependencies"})).collect();
        Ok(
            json!({"libraries":libraries,"unique_libraries":libraries.len(),"unique_symbols":items.len(),"call_sites":items.iter().map(|i|i["sites"].as_array().map_or(0,Vec::len)).sum::<usize>(),"items":items,"manmenmi":"M10_EXTERNAL_ADAPTER_IMPLEMENTED","model_contracts":2,"reference_validated_handlers":0}),
        )
    }
}

pub fn plan_closure(
    root: &str,
    functions: &[Function],
    contracts: &[Contract],
    image: &CafeGuestImage,
) -> Result<Value> {
    let by_id: BTreeMap<_, _> = functions.iter().map(|f| (f.id.as_str(), f)).collect();
    if !by_id.contains_key(root) {
        return Err(Error::Analysis("closure root missing".into()));
    }
    let mut queue = VecDeque::from([(root.to_owned(), vec![root.to_owned()])]);
    let mut seen = BTreeSet::new();
    let mut entries = vec![];
    let mut imports: BTreeMap<(String, String), Value> = BTreeMap::new();
    let mut indirects = vec![];
    let mut missing = vec![];
    let mut blockers = BTreeSet::new();
    let builder = ExecutionContextBuilder::new(image);
    while let Some((id, path)) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if seen.len() > 4096 {
            return Err(Error::Analysis("closure function budget".into()));
        }
        let Some(f) = by_id.get(id.as_str()) else {
            missing.push(id);
            blockers.insert("BLOCKED_PARTIAL_FUNCTION");
            continue;
        };
        let native = woomy_codegen::eligibility(f);
        let context = builder.analyze(f, false);
        if native.status != "ELIGIBLE" {
            blockers.insert("BLOCKED_PARTIAL_FUNCTION");
            blockers.insert("BLOCKED_UNSUPPORTED_IR");
        }
        if !context.actual_game_execution_ready {
            blockers.insert("BLOCKED_CONTEXT");
        }
        for b in &f.blocks {
            if matches!(
                b.terminator,
                woomy_ir::Terminator::UnresolvedIndirectBranch { .. }
                    | woomy_ir::Terminator::OpaqueTerminator { .. }
            ) || b
                .successors
                .iter()
                .any(|s| !matches!(s.destination, woomy_ir::Destination::Block(_)))
            {
                indirects.push(json!({"function":id,"site":b.terminator_source.address,"reason":"UNRESOLVED_TERMINATOR_OR_EXTERNAL_EXIT","first_path":path}));
                blockers.insert("BLOCKED_INDIRECT");
            }
        }
        entries.push(json!({"function":id,"entry":f.entry,"ir_status":f.status,"native_eligible":native.status=="ELIGIBLE","unsupported":native.diagnostics,"context_blockers":context.blockers,"memory":context.requirements.iter().filter(|r|r.width.is_some()).collect::<Vec<_>>(),"first_path":path}));
        for c in contracts.iter().filter(|c| c.caller == id) {
            match &c.target {
                Target::InternalFunction {
                    function: Some(callee),
                    ..
                } => {
                    if !seen.contains(callee) {
                        let mut next = path.clone();
                        next.push(callee.clone());
                        queue.push_back((callee.clone(), next));
                    }
                }
                Target::ImportedFunction { library, symbol } => {
                    blockers.insert("BLOCKED_UNKNOWN_IMPORT");
                    let item=imports.entry((library.clone(),symbol.clone())).or_insert_with(||json!({"library":library,"symbol":symbol,"sites":[],"first_reachable_path":path,"effect_knowledge":"UNKNOWN","runtime_requirement":"Cafe provider with justified semantics","implementation_status":"UNRESOLVED_IMPORT","provider_available":false,"provider_scope":"GAME_FUNCTION_REFERENCE_CONTEXT","model_contract":manmenmi::import_contract(&woomy_ir::runtime::Import{library:library.clone(),symbol:symbol.clone()})}));
                    item["sites"]
                        .as_array_mut()
                        .expect("owned array")
                        .push(json!({"caller":id,"site":c.site}));
                }
                _ => {
                    blockers.insert("BLOCKED_INDIRECT");
                    indirects.push(
                        json!({"function":id,"site":c.site,"target":c.target,"first_path":path}),
                    );
                }
            }
        }
    }
    let mut required_imports: Vec<_> = imports.into_values().collect();
    required_imports.sort_by_key(|item| {
        (
            item["first_reachable_path"]
                .as_array()
                .map_or(usize::MAX, Vec::len),
            std::cmp::Reverse(item["sites"].as_array().map_or(0, Vec::len)),
            item["library"].as_str().unwrap_or("").to_owned(),
            item["symbol"].as_str().unwrap_or("").to_owned(),
        )
    });
    for (rank, item) in required_imports.iter_mut().enumerate() {
        item["priority_rank"] = json!(rank + 1);
        item["priority_evidence"]=json!("Shortest known entry path, then call-site fan-out, then stable identity; no API semantics inferred");
    }
    let native = entries
        .iter()
        .filter(|v| v["native_eligible"] == true)
        .count();
    Ok(
        json!({"root":root,"versions":ImageContextVersions::CURRENT,"image_hash":image.hash(),"states":blockers,"closed":blockers.is_empty(),"known_functions":entries.len(),"native_eligible_functions":native,"partial_functions":entries.len()-native,"functions":entries,"imports":required_imports,"unresolved_control_flow":indirects,"missing_functions":missing,"context_incomplete_functions":entries.len(),"scope":"Known explored dependencies only; unknown exits may hide additional functions/imports. No startup execution attempted."}),
    )
}
