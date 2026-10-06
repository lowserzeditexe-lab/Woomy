use crate::{
    hash_bytes,
    project::{read_bounded, ProjectService},
    Error, Result,
};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::atomic::{AtomicU8, Ordering},
    time::Instant,
};
use woomy_binary::{Module, RPX_PARSER_VERSION};
use woomy_ppc::{
    build_cfg_with_targets, liveness, DecodedInstruction, Flow, CFG_ANALYZER_VERSION,
    DATAFLOW_VERSION, PPC_DECODER_VERSION,
};

pub struct AnalysisService;
#[derive(Debug, Default, Serialize)]
pub struct AnalysisSummary {
    pub parsed_modules: usize,
    pub cached_modules: usize,
    pub failed_modules: usize,
    pub functions: usize,
    pub instructions: usize,
    pub unknown_instructions: usize,
    pub elapsed_seconds: f64,
}
#[derive(Debug, Clone)]
struct Evidence {
    kind: &'static str,
    source: u32,
}
struct Discovery {
    code: BTreeMap<u32, DecodedInstruction>,
    functions: BTreeMap<u32, Vec<Evidence>>,
    names: BTreeMap<u32, String>,
    truncated: bool,
    targets: BTreeMap<u32, BTreeMap<u32, BTreeSet<u32>>>,
    advanced_complete: bool,
}
impl AnalysisService {
    pub fn run(
        project: &mut ProjectService,
        cancel: &AtomicU8,
        mut progress: impl FnMut(&str),
    ) -> Result<AnalysisSummary> {
        let start = Instant::now();
        let mut summary = AnalysisSummary::default();
        project
            .db
            .execute("INSERT INTO analysis_runs(status) VALUES('RUNNING')", [])?;
        let run = project.db.last_insert_rowid();
        let result = (|| -> Result<()> {
            // Persist RUNNING before the scan so even an interruption during
            // hashing is recognized when the project is opened again.
            progress("[1/4] Scanning and hashing local dump...");
            project.scan(cancel, &mut progress)?;
            project.invalidate_versions()?;
            let files: Vec<(String, String, String)> = project
                .db
                .prepare(
                    "SELECT path,sha256,kind FROM files WHERE kind IN ('RPX','RPL') ORDER BY path",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<_, _>>()?;
            if files.is_empty() {
                return Err(Error::Analysis("no RPX/RPL files found".into()));
            }
            for (path, hash, kind) in files {
                if cancel.load(Ordering::Relaxed) != 0 {
                    return Err(Error::Cancelled);
                }
                let id = module_id(&path);
                let cached: Option<String> = project
                    .db
                    .query_row(
                        "SELECT status FROM modules WHERE id=?1 AND hash=?2",
                        params![id, hash],
                        |r| r.get(0),
                    )
                    .optional()?;
                if cached.as_deref() == Some("ANALYZED_PARTIAL") {
                    summary.cached_modules += 1;
                    progress(&format!("Cache valid: {path}"));
                    continue;
                }
                progress(&format!("[2/4] Parsing {path}..."));
                let data = read_bounded(&project.input(&path)?, woomy_binary::MAX_MODULE_BYTES)?;
                if hash_bytes(&data) != hash {
                    return Err(Error::Analysis("module changed after scan; retry".into()));
                }
                let module = match woomy_binary::parse(&data) {
                    Ok(m) => m,
                    Err(e) => {
                        summary.failed_modules += 1;
                        project.db.execute("INSERT INTO problems(category,summary,provenance) VALUES('PARSE',?1,'REAL')",[format!("{path}: {e}")])?;
                        progress(&format!("Parse failed: {path}: {e}"));
                        continue;
                    }
                };
                drop(data);
                progress(&format!("[3/4] Discovering reachable PPC: {path}"));
                let instruction_limit = match project.config.analysis.performance.as_str() {
                    "eco" => 100_000,
                    "maximum" => 500_000,
                    _ => 250_000,
                };
                let discovery = discover(&module, &id, instruction_limit, cancel, &mut progress)?;
                let unknown = discovery
                    .code
                    .values()
                    .filter(|d| d.status == "UNKNOWN" || d.status == "INVALID")
                    .count();
                progress(&format!("[4/4] CFG, calls, xrefs, conservative liveness: {} functions, {} instructions, {} unknown",discovery.functions.len(),discovery.code.len(),unknown));
                let tx = project.db.transaction()?;
                persist_module(&tx, &id, &path, &hash, &kind, &module)?;
                persist_analysis(&tx, &id, &module, &discovery, cancel, &mut progress)?;
                if cancel.load(Ordering::Relaxed) != 0 {
                    return Err(Error::Cancelled);
                }
                tx.execute(
                    "UPDATE modules SET status='ANALYZED_PARTIAL' WHERE id=?1",
                    [&id],
                )?;
                tx.execute(
                    "UPDATE analysis_runs SET modules_done=modules_done+1 WHERE id=?1",
                    [run],
                )?;
                tx.commit()?;
                summary.parsed_modules += 1;
                summary.functions += discovery.functions.len();
                summary.instructions += discovery.code.len();
                summary.unknown_instructions += unknown;
                project.log(&format!(
                    "module checkpoint: {} functions, {} instructions",
                    discovery.functions.len(),
                    discovery.code.len()
                ))?;
            }
            Ok(())
        })();
        let state = match &result {
            Err(Error::Cancelled) => {
                if cancel.load(Ordering::Relaxed) == 2 {
                    "PAUSED"
                } else {
                    "CANCELLED"
                }
            }
            Err(_) => "FAILED",
            Ok(_) => {
                if summary.failed_modules > 0 {
                    "PARTIAL"
                } else {
                    "COMPLETED"
                }
            }
        };
        project.db.execute(
            "UPDATE analysis_runs SET status=?1,finished=CURRENT_TIMESTAMP WHERE id=?2",
            params![state, run],
        )?;
        result?;
        summary.elapsed_seconds = start.elapsed().as_secs_f64();
        if summary.failed_modules > 0 {
            return Err(Error::Analysis(format!(
                "{} module(s) failed parsing; inspect problems",
                summary.failed_modules
            )));
        }
        Ok(summary)
    }
}
pub fn module_id(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("module.{clean}.{}", &hash_bytes(path.as_bytes())[..12])
}
pub fn function_id(module: &str, address: u32) -> String {
    format!("{module}.{address:08X}")
}
fn add_seed(
    m: &Module,
    seeds: &mut BTreeMap<u32, Vec<Evidence>>,
    address: u32,
    kind: &'static str,
    source: u32,
) {
    if m.executable_address(address) {
        let v = seeds.entry(address).or_default();
        if !v.iter().any(|e| e.kind == kind && e.source == source) {
            v.push(Evidence { kind, source });
        }
    }
}
fn discover(
    m: &Module,
    id: &str,
    limit: usize,
    cancel: &AtomicU8,
    progress: &mut impl FnMut(&str),
) -> Result<Discovery> {
    let mut functions = BTreeMap::new();
    let mut names = BTreeMap::new();
    add_seed(
        m,
        &mut functions,
        m.entry_point,
        "ENTRY_POINT",
        m.entry_point,
    );
    for e in &m.exports {
        if e.code {
            add_seed(m, &mut functions, e.address, "EXPORT", e.address);
            names.insert(e.address, e.name.clone());
        }
    }
    for s in &m.symbols {
        if s.info & 15 == 2 && s.section != 0 {
            add_seed(m, &mut functions, s.value, "SYMBOL", s.value);
            if !s.name.is_empty() {
                names.insert(s.value, s.name.clone());
            }
        }
    }
    let symbols: BTreeMap<_, _> = m.symbols.iter().map(|s| ((s.table, s.index), s)).collect();
    let model = crate::module_model::ModuleModel::new(id, m);
    for r in &m.relocations {
        if let Some(s) = symbols.get(&(r.symbol_table, r.symbol)) {
            if s.info & 15 == 2 && s.section != 0 && r.addend == Some(0) {
                add_seed(m, &mut functions, s.value, "RELOCATION", r.offset);
            }
        }
    }
    let mut queue: VecDeque<u32> = functions.keys().copied().collect();
    let mut queued: BTreeSet<u32> = functions.keys().copied().collect();
    let mut code = BTreeMap::new();
    let mut truncated = false;
    let mut indirect_targets: BTreeMap<u32, BTreeMap<u32, BTreeSet<u32>>> = BTreeMap::new();
    let mut advanced_complete = false;
    let mut edge_count = 0usize;
    for round in 0..16 {
        while let Some(pc) = queue.pop_front() {
            if cancel.load(Ordering::Relaxed) != 0 {
                return Err(Error::Cancelled);
            }
            if code.len() >= limit {
                truncated = true;
                break;
            }
            let Some(d) = model.instruction(pc) else {
                continue;
            };
            let mut targets = vec![];
            match d.control_flow {
                Flow::Next | Flow::ExternalCall | Flow::ConditionalTrap => {
                    if let Some(n) = pc.checked_add(4) {
                        targets.push(n);
                    }
                }
                Flow::Call { target, .. } => {
                    if pc.checked_add(4) != Some(target) {
                        add_seed(m, &mut functions, target, "DIRECT_CALL_TARGET", pc);
                    }
                    targets.push(target);
                    if let Some(n) = pc.checked_add(4) {
                        targets.push(n);
                    }
                }
                Flow::Branch {
                    target,
                    conditional,
                } => {
                    targets.push(target);
                    if conditional {
                        if let Some(n) = pc.checked_add(4) {
                            targets.push(n);
                        }
                    }
                }
                Flow::Indirect { link: true, .. }
                | Flow::Indirect {
                    conditional: true, ..
                }
                | Flow::Return { conditional: true } => {
                    if let Some(n) = pc.checked_add(4) {
                        targets.push(n);
                    }
                }
                _ => {}
            }
            code.insert(pc, d);
            for t in targets {
                if m.executable_address(t) && queued.insert(t) {
                    queue.push_back(t);
                }
            }
        }
        if truncated {
            break;
        }
        progress(&format!(
            "[3/4] Dataflow/indirect fixed point, round {}: {} functions",
            round + 1,
            functions.len()
        ));
        let entries: BTreeSet<_> = functions.keys().copied().collect();
        let mut changed = false;
        let mut memberships = 0usize;
        for entry in &entries {
            if cancel.load(Ordering::Relaxed) != 0 {
                return Err(Error::Cancelled);
            }
            let extra = indirect_targets.entry(*entry).or_default();
            let cfg = build_cfg_with_targets(*entry, &code, &entries, 16_384, extra);
            memberships += cfg
                .blocks
                .iter()
                .map(|b| b.instructions.len())
                .sum::<usize>();
            if memberships > 2_000_000 {
                truncated = true;
                break;
            }
            let facts = woomy_ppc::dataflow::analyze(*entry, &cfg, &code, &model, || {
                cancel.load(Ordering::Relaxed) != 0
            });
            let (resolutions, _) = advanced_results(*entry, &cfg, &code, &facts, &model);
            for resolution in resolutions {
                for target in resolution.targets {
                    if let crate::control_flow::ControlTarget::Static { address, .. } = target {
                        if extra.entry(resolution.address).or_default().insert(address) {
                            changed = true;
                            edge_count += 1;
                        }
                        if matches!(
                            code.get(&resolution.address).map(|d| d.control_flow),
                            Some(Flow::Indirect { link: true, .. })
                        ) {
                            let n = functions.len();
                            add_seed(
                                m,
                                &mut functions,
                                address,
                                "INDIRECT_CALL_TARGET",
                                resolution.address,
                            );
                            changed |= n != functions.len();
                        }
                        if queued.insert(address) {
                            queue.push_back(address);
                            changed = true;
                        }
                    }
                }
            }
            if edge_count > 262_144 {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
        if !changed {
            advanced_complete = true;
            break;
        }
    }
    // Seeds whose instructions exceeded the module budget remain visible as not decoded.
    Ok(Discovery {
        code,
        functions,
        names,
        truncated,
        targets: indirect_targets,
        advanced_complete,
    })
}
fn advanced_results(
    entry: u32,
    cfg: &woomy_ppc::Cfg,
    code: &BTreeMap<u32, DecodedInstruction>,
    facts: &woomy_ppc::dataflow::Dataflow,
    model: &crate::module_model::ModuleModel<'_>,
) -> (
    Vec<crate::control_flow::Resolution>,
    Vec<crate::jump_tables::JumpTable>,
) {
    let mut resolutions = crate::control_flow::resolve(cfg, code, facts, model);
    let tables = crate::jump_tables::discover(entry, cfg, code, facts, model);
    for table in tables.iter().filter(|t| t.status == "VERIFIED") {
        if let Some(r) = resolutions.iter_mut().find(|r| r.address == table.site) {
            r.targets = table
                .entries
                .iter()
                .map(|(_, address)| crate::control_flow::ControlTarget::Static {
                    module: model.id.into(),
                    address: *address,
                })
                .collect();
            r.targets.sort();
            r.targets.dedup();
            r.status = "RESOLVED";
            r.evidence = serde_json::json!({"rule":"BOUNDED_IMMUTABLE_SWITCH", "analysis_version":DATAFLOW_VERSION,"table_address":table.address,"bound":table.bound,"default_target":table.default_target,"proof":table.evidence}).to_string();
        }
    }
    (resolutions, tables)
}
fn persist_module(
    tx: &Transaction<'_>,
    id: &str,
    path: &str,
    hash: &str,
    kind: &str,
    m: &Module,
) -> Result<()> {
    tx.execute("DELETE FROM modules WHERE id=?1", [id])?;
    tx.execute("INSERT INTO modules(id,path,hash,kind,entry_point,parser_version,decoder_version,cfg_version,dataflow_version,ir_version,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'PARSING')",params![id,path,hash,kind,m.entry_point,RPX_PARSER_VERSION,PPC_DECODER_VERSION,CFG_ANALYZER_VERSION,DATAFLOW_VERSION,woomy_ir::IR_VERSION])?;
    let mut stmt = tx.prepare_cached("INSERT INTO sections VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
    for s in &m.sections {
        stmt.execute(params![
            id,
            s.index,
            s.name,
            s.kind,
            s.flags,
            s.address,
            s.size,
            s.offset,
            s.file_size
        ])?;
    }
    let mut stmt = tx.prepare_cached("INSERT INTO imports VALUES(?1,?2,?3,?4,?5,?6)")?;
    for i in &m.imports {
        stmt.execute(params![
            id,
            i.section,
            i.library,
            i.count,
            i.signature,
            i.code
        ])?;
    }
    let mut stmt = tx.prepare_cached("INSERT INTO exports VALUES(?1,?2,?3,?4,?5)")?;
    for e in &m.exports {
        stmt.execute(params![id, e.section, e.name, e.address, e.code])?;
    }
    let mut stmt = tx.prepare_cached("INSERT INTO symbols VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
    for s in &m.symbols {
        stmt.execute(params![
            id, s.table, s.index, s.name, s.value, s.size, s.info, s.section
        ])?;
    }
    let mut stmt = tx.prepare_cached("INSERT INTO relocations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
    for r in &m.relocations {
        stmt.execute(params![
            id,
            r.section,
            r.target_section,
            r.symbol_table,
            r.offset,
            r.kind,
            r.symbol,
            r.addend
        ])?;
    }
    Ok(())
}
fn persist_analysis(
    tx: &Transaction<'_>,
    id: &str,
    m: &Module,
    d: &Discovery,
    cancel: &AtomicU8,
    progress: &mut impl FnMut(&str),
) -> Result<()> {
    let mut stmt =
        tx.prepare_cached("INSERT INTO instructions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
    for i in d.code.values() {
        if cancel.load(Ordering::Relaxed) != 0 {
            return Err(Error::Cancelled);
        }
        stmt.execute(params![
            id,
            i.address,
            i.raw,
            i.opcode,
            i.status,
            i.classification,
            serde_json::to_string(&i.operands)?,
            serde_json::to_string(&i.semantics)?,
            serde_json::to_string(&i.control_flow)?
        ])?;
    }
    for (a, evidence) in &d.functions {
        let fid = function_id(id, *a);
        tx.execute(
            "INSERT INTO functions(id,module,address,name,status) VALUES(?1,?2,?3,?4,?5)",
            params![
                fid,
                id,
                a,
                d.names.get(a),
                if d.code.contains_key(a) {
                    "PARTIAL"
                } else {
                    "NOT_DECODED"
                }
            ],
        )?;
        for e in evidence {
            tx.execute(
                "INSERT INTO function_evidence VALUES(?1,?2,?3,?4)",
                params![
                    fid,
                    e.kind,
                    e.source,
                    if e.kind == "INDIRECT_CALL_TARGET" {
                        "INFERRED"
                    } else {
                        "REAL"
                    }
                ],
            )?;
        }
    }
    let module_hash: String =
        tx.query_row("SELECT hash FROM modules WHERE id=?1", [id], |r| r.get(0))?;
    let entries = d.functions.keys().copied().collect();
    let mut membership_count = 0usize;
    let model = crate::module_model::ModuleModel::new(id, m);
    let ir_relocations = d
        .code
        .keys()
        .filter_map(|pc| model.ir_relocation(*pc).map(|r| (*pc, r)))
        .collect();
    for (index, a) in d.functions.keys().enumerate() {
        if cancel.load(Ordering::Relaxed) != 0 {
            return Err(Error::Cancelled);
        }
        let fid = function_id(id, *a);
        if membership_count >= 2_000_000 {
            tx.execute(
                "UPDATE functions SET status='CFG_LIMIT' WHERE id=?1",
                [&fid],
            )?;
            if membership_count == 2_000_000 {
                tx.execute("INSERT INTO problems(module,category,summary,provenance) VALUES(?1,'LIMIT','Aggregate function membership budget reached','REAL')",[id])?;
                membership_count += 1;
            }
            continue;
        }
        let mut cfg = build_cfg_with_targets(
            *a,
            &d.code,
            &entries,
            16_384.min(2_000_000 - membership_count),
            d.targets.get(a).unwrap_or(&BTreeMap::new()),
        );
        let facts = woomy_ppc::dataflow::analyze(*a, &cfg, &d.code, &model, || {
            cancel.load(Ordering::Relaxed) != 0
        });
        let (mut resolutions, mut tables) = advanced_results(*a, &cfg, &d.code, &facts, &model);
        if !d.advanced_complete || cfg.truncated {
            for r in &mut resolutions {
                if !r.targets.is_empty() {
                    r.status = "PARTIAL";
                }
            }
            for t in &mut tables {
                if t.status == "VERIFIED" {
                    t.status = "PARTIAL";
                }
            }
        }
        for r in &resolutions {
            if let Some(block) = cfg.blocks.iter().find(|b| b.end == r.address) {
                for edge in cfg
                    .edges
                    .iter_mut()
                    .filter(|e| e.from == block.start && e.kind == "IndirectProven")
                {
                    if !r.targets.iter().any(|t| matches!(t,crate::control_flow::ControlTarget::Static{address,..} if edge.to==Some(*address))) {
                        edge.kind = "IndirectCandidate".into();
                    }
                }
                if r.targets
                    .iter()
                    .any(|t| matches!(t, crate::control_flow::ControlTarget::Imported { .. }))
                    && !matches!(
                        d.code[&r.address].control_flow,
                        Flow::Indirect { link: true, .. }
                    )
                {
                    cfg.edges.push(woomy_ppc::Edge {
                        from: block.start,
                        to: None,
                        kind: "ImportedTarget".into(),
                    });
                }
            }
            if r.status == "RESOLVED" {
                if let Some(block) = cfg.blocks.iter().find(|b| b.end == r.address) {
                    cfg.edges.retain(|e| {
                        !(e.from == block.start && e.to.is_none() && e.kind == "Indirect")
                    });
                }
            }
            if let Some(call) = cfg.calls.iter_mut().find(|c| c.address == r.address) {
                if r.targets.len() == 1 && r.status == "RESOLVED" {
                    match &r.targets[0] {
                        crate::control_flow::ControlTarget::Static { address, .. } => {
                            call.target = Some(*address);
                            call.kind = "IndirectResolved".into();
                        }
                        crate::control_flow::ControlTarget::Imported { .. } => {
                            call.kind = "Import".into();
                        }
                    }
                }
            }
        }
        membership_count += cfg
            .blocks
            .iter()
            .map(|b| b.instructions.len())
            .sum::<usize>();
        for b in &cfg.blocks {
            tx.execute(
                "INSERT INTO basic_blocks VALUES(?1,?2,?3)",
                params![fid, b.start, b.end],
            )?;
            let mut stmt = tx.prepare_cached("INSERT INTO function_instructions VALUES(?1,?2)")?;
            for pc in &b.instructions {
                stmt.execute(params![fid, pc])?;
            }
        }
        for e in &cfg.edges {
            tx.execute(
                "INSERT INTO cfg_edges VALUES(?1,?2,?3,?4)",
                params![fid, e.from, e.to, e.kind],
            )?;
        }
        for c in &cfg.calls {
            let evidence = if let Some(r) = resolutions.iter().find(|r| r.address == c.address) {
                r.evidence.clone()
            } else if c.kind == "Import" {
                serde_json::to_string(&d.code[&c.address].operands)?
            } else {
                "DECODED_INSTRUCTION".into()
            };
            tx.execute(
                "INSERT INTO call_sites VALUES(?1,?2,?3,?4,?5)",
                params![fid, c.address, c.kind, c.target, evidence],
            )?;
            if let Some(target) = c.target {
                if d.functions.contains_key(&target) {
                    tx.execute(
                        "INSERT INTO call_edges VALUES(?1,?2,?3)",
                        params![fid, function_id(id, target), c.address],
                    )?;
                }
            }
        }
        persist_advanced(tx, id, &fid, &resolutions, &tables, &d.code, &d.functions)?;
        for l in liveness(&cfg, &d.code) {
            tx.execute(
                "INSERT INTO block_liveness VALUES(?1,?2,?3,?4)",
                params![
                    fid,
                    l.block,
                    serde_json::to_string(&l.live_in)?,
                    serde_json::to_string(&l.live_out)?
                ],
            )?;
        }
        persist_dataflow(tx, &fid, &d.code, &facts)?;
        crate::ir::persist(
            tx,
            woomy_ir::lift_with_relocations(
                &fid,
                id,
                &module_hash,
                *a,
                &cfg,
                &d.code,
                &ir_relocations,
            ),
        )?;
        if cfg.truncated {
            tx.execute("INSERT INTO problems(module,function,category,summary,provenance) VALUES(?1,?2,'LIMIT','Function CFG instruction budget reached','REAL')",params![id,fid])?;
        }
        if index > 0 && index.is_multiple_of(500) {
            progress(&format!(
                "Processed {index}/{} functions",
                d.functions.len()
            ));
        }
    }
    crate::calls::persist(tx, id)?;
    let mut unknown: BTreeMap<(u32, u32), (u32, u32)> = BTreeMap::new();
    for i in d.code.values() {
        if i.status == "UNKNOWN" || i.status == "INVALID" {
            let e = unknown
                .entry((i.raw >> 26, (i.raw >> 1) & 1023))
                .or_insert((0, i.address));
            e.0 += 1;
        }
        let (target, kind) = match i.control_flow {
            Flow::ExternalCall => (None, "ImportReference"),
            Flow::Call { target, .. } => (Some(target), "Call"),
            Flow::Branch { target, .. } => (Some(target), "Branch"),
            _ => continue,
        };
        tx.execute(
            "INSERT INTO cross_references VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                i.address,
                target,
                kind,
                if i.status == "RELOCATION_MODELED" {
                    "INFERRED"
                } else {
                    "REAL"
                }
            ],
        )?;
    }
    for ((op, xo), (count, example)) in unknown {
        tx.execute(
            "INSERT INTO unknown_opcodes VALUES(?1,?2,?3,?4,?5)",
            params![id, op, xo, count, example],
        )?;
    }
    for r in &m.relocations {
        tx.execute(
            "INSERT INTO cross_references VALUES(?1,?2,NULL,'Relocation','REAL')",
            params![id, r.offset],
        )?;
    }
    for e in &m.exports {
        tx.execute(
            "INSERT INTO cross_references VALUES(?1,?2,?2,'ExportReference','REAL')",
            params![id, e.address],
        )?;
    }
    if d.truncated {
        tx.execute("INSERT INTO problems(module,category,summary,provenance) VALUES(?1,'LIMIT','Module reachable instruction budget reached','REAL')",[id])?;
    }
    if d.functions.is_empty() {
        tx.execute("INSERT INTO problems(module,category,summary,provenance) VALUES(?1,'DISCOVERY','No executable entry, export, symbol or relocation seed','UNKNOWN')",[id])?;
    }
    if !d.advanced_complete {
        tx.execute("INSERT INTO problems(module,category,summary,provenance) VALUES(?1,'LIMIT','Indirect discovery did not reach a fixed point within analysis budgets; resolutions remain partial','REAL')",[id])?;
    }
    tx.execute("INSERT INTO problems(module,category,summary,provenance) VALUES(?1,'LIMITATION','Logical relocation subset only; original bytes unchanged. Register facts depend on known CFG paths and inferred function boundaries. Calls and unknown effects clobber facts. Mutable memory and unsupported relocations remain unknown. Jump-table proofs support one bounded immutable pattern.','REAL')",[id])?;
    Ok(())
}

fn persist_advanced(
    tx: &Transaction<'_>,
    module: &str,
    function: &str,
    resolutions: &[crate::control_flow::Resolution],
    tables: &[crate::jump_tables::JumpTable],
    code: &BTreeMap<u32, DecodedInstruction>,
    functions: &BTreeMap<u32, Vec<Evidence>>,
) -> Result<()> {
    use crate::control_flow::ControlTarget;
    for r in resolutions {
        let target = match r.targets.as_slice() {
            [ControlTarget::Static { address, .. }] if r.status == "RESOLVED" => Some(*address),
            _ => None,
        };
        tx.execute("INSERT INTO indirect_sites(function,address,status,target,evidence,source_register,analysis_version,is_return) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", params![function,r.address,r.status,target,r.evidence,format!("{:?}",r.source),DATAFLOW_VERSION,r.is_return])?;
        for (ordinal, target) in r.targets.iter().enumerate() {
            let (kind, target_module, address, library, symbol) = match target {
                ControlTarget::Static { module, address } => {
                    ("STATIC", Some(module.as_str()), Some(*address), None, None)
                }
                ControlTarget::Imported { library, symbol } => (
                    "IMPORTED",
                    None,
                    None,
                    Some(library.as_str()),
                    Some(symbol.as_str()),
                ),
            };
            tx.execute(
                "INSERT INTO indirect_targets VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    function,
                    r.address,
                    ordinal,
                    kind,
                    target_module,
                    address,
                    library,
                    symbol,
                    r.evidence
                ],
            )?;
            tx.execute(
                "INSERT INTO cross_references VALUES(?1,?2,?3,?4,'INFERRED')",
                params![
                    module,
                    r.address,
                    address,
                    if address.is_some() {
                        "IndirectTarget"
                    } else {
                        "IndirectImport"
                    }
                ],
            )?;
            if matches!(
                code[&r.address].control_flow,
                Flow::Indirect { link: true, .. }
            ) {
                if let Some(a) = address.filter(|a| functions.contains_key(a)) {
                    tx.execute(
                        "INSERT OR IGNORE INTO call_edges VALUES(?1,?2,?3)",
                        params![function, function_id(module, a), r.address],
                    )?;
                }
            }
        }
    }
    for t in tables {
        let id = format!("{function}.table.{:08X}", t.site);
        tx.execute("INSERT INTO jump_tables(id,module,address,status,bound,evidence,function,site,section,default_target) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![id,module,t.address,t.status,t.bound,serde_json::to_string(&t.evidence)?,function,t.site,t.section,t.default_target])?;
        for (index, target) in &t.entries {
            tx.execute(
                "INSERT INTO jump_table_entries VALUES(?1,?2,?3)",
                params![id, index, target],
            )?;
        }
        for (ordinal, e) in t.evidence.iter().enumerate() {
            tx.execute(
                "INSERT INTO jump_table_evidence VALUES(?1,?2,?3,?4,?5)",
                params![id, ordinal, e.kind, e.source, e.detail],
            )?;
        }
    }
    Ok(())
}

fn persist_dataflow(
    tx: &Transaction<'_>,
    function: &str,
    code: &BTreeMap<u32, DecodedInstruction>,
    facts: &woomy_ppc::dataflow::Dataflow,
) -> Result<()> {
    use woomy_ppc::dataflow::{Atom, Definition, Value};
    tx.execute(
        "INSERT INTO dataflow_runs VALUES(?1,?2,?3,?4,?5)",
        params![
            function,
            DATAFLOW_VERSION,
            facts.converged,
            facts.steps,
            if facts.converged {
                "CONVERGED"
            } else {
                "PARTIAL_LIMIT"
            }
        ],
    )?;
    let mut values =
        tx.prepare_cached("INSERT INTO register_values VALUES(?1,?2,?3,?4,?5,'INFERRED',?6)")?;
    for fact in &facts.writes {
        let kind = match &fact.fact.value {
            Value::Unknown => "UNKNOWN",
            Value::Conflicting(_) => "CONFLICTING",
            Value::Known(Atom::Constant(_)) => "CONSTANT",
            Value::Known(Atom::Target(_)) => "SYMBOLIC_ADDRESS",
            Value::Known(Atom::Upper { .. }) => "RELOCATION_PART",
        };
        let rule = code
            .get(&fact.address)
            .map(|d| {
                let origin = match &fact.fact.value {
                    Value::Unknown => "UNKNOWN",
                    Value::Known(Atom::Target(_) | Atom::Upper { .. }) => "RELOCATION_DERIVED",
                    _ if ["addi", "addis"].contains(&d.opcode.as_str())
                        && (d.raw >> 16) & 31 == 0 =>
                    {
                        "RAW_IMMEDIATE"
                    }
                    _ if d.opcode == "or" && (d.raw >> 21) & 31 == (d.raw >> 11) & 31 => {
                        "REGISTER_COPY"
                    }
                    _ => "COMPUTED",
                };
                format!("{origin}:{}:{}", d.opcode, d.status)
            })
            .unwrap_or_else(|| "UNKNOWN".into());
        values.execute(params![
            function,
            fact.address,
            format!("{:?}", fact.register),
            kind,
            serde_json::to_string(&fact.fact.value)?,
            rule
        ])?;
    }
    let mut reaching =
        tx.prepare_cached("INSERT INTO reaching_definitions VALUES(?1,?2,?3,?4,?5)")?;
    for fact in &facts.uses {
        for def in &fact.fact.definitions {
            let (address, kind) = match def {
                Definition::Entry => (-1, "ENTRY"),
                Definition::Instruction(pc) => (*pc as i64, "INSTRUCTION"),
                Definition::Unknown(pc) => (*pc as i64, "UNKNOWN_EFFECT"),
                Definition::Call(pc) => (*pc as i64, "CALL_CLOBBER"),
                Definition::Many => (-1, "MANY"),
            };
            reaching.execute(params![
                function,
                fact.address,
                format!("{:?}", fact.register),
                address,
                kind
            ])?;
        }
    }
    Ok(())
}
