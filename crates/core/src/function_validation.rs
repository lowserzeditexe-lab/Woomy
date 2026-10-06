//! Explicit bounded validation of real function bytes in a synthetic input domain.
use crate::{
    cafe::CafeGuestImage, context::ExecutionContextBuilder, Error, ProjectService, Result,
};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use woomy_ir::{
    execution::Machine,
    runtime::{
        GuestImageIdentity, GuestMemory, ImageContextVersions, Limits, Program, ProviderKind,
        RUNTIME_ABI_VERSION,
    },
    Function,
};

pub fn register_only_program(
    image: &CafeGuestImage,
    f: Function,
) -> Result<(Program, BTreeMap<u32, u32>, GuestMemory)> {
    image.verify()?;
    if image
        .relocations()
        .iter()
        .any(|r| ["INVALID", "AMBIGUOUS"].contains(&r.state.as_str()))
    {
        return Err(Error::Analysis(
            "INVALID_IMAGE_RELOCATION: execution rejected".into(),
        ));
    }
    let context = ExecutionContextBuilder::new(image).analyze(&f, false);
    if !context.register_only_synthetic_eligible {
        return Err(Error::Analysis("CONTEXT_INCOMPLETE: synthetic register-only validation requires a supported closed function without memory/calls/unknown targets".into()));
    }
    let mut code = BTreeMap::new();
    let mut memory = GuestMemory::default();
    for block in &f.blocks {
        if block.sources.is_empty() {
            return Err(Error::Analysis("EMPTY_EXECUTABLE_BLOCK".into()));
        }
        let region = image.slice(block.id, block.sources.len() * 4)?;
        for source in &block.sources {
            let n = (source.address - block.id) as usize;
            let word = region
                .bytes
                .get(n..n + 4)
                .ok_or_else(|| Error::Analysis("SOURCE_OUTSIDE_BLOCK".into()))?;
            code.insert(
                source.address,
                u32::from_be_bytes(word.try_into().expect("word")),
            );
        }
        memory.regions.push(region);
    }
    if !memory.valid() {
        return Err(Error::Analysis("CONTEXT_MEMORY_BUDGET".into()));
    }
    let program = Program {
        runtime_context: None,
        source_barriers: BTreeMap::new(),
        image: Some(GuestImageIdentity {
            versions: ImageContextVersions::CURRENT,
            module: f.module.clone(),
            module_hash: f.module_hash.clone(),
            image_hash: image.hash().into(),
        }),
        version: RUNTIME_ABI_VERSION,
        entry: f.entry,
        functions: vec![f],
        imports: BTreeMap::new(),
        provider: ProviderKind::NoRuntime,
        limits: Limits {
            calls: 0,
            depth: 0,
            trace: 16,
        },
    };
    program.validate().map_err(Error::Analysis)?;
    Ok((program, code, memory))
}
fn corpus(memory: &GuestMemory) -> Vec<(Machine, usize)> {
    let mut seed = 0x62a7_c39du32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let edges = [
        0,
        1,
        u32::MAX,
        0x80000000,
        0x7fffffff,
        0x7fff,
        0x8000,
        0xffff,
        0x10000,
        0x55555555,
        0xaaaaaaaa,
        31,
        32,
        63,
        64,
    ];
    (0..512)
        .map(|i| {
            let mut m = Machine::new(0, vec![]);
            for r in &mut m.gpr {
                *r = if i < edges.len() { edges[i] } else { next() };
            }
            for r in &mut m.fpr {
                *r = ((next() as u64) << 32) | next() as u64;
            }
            for r in &mut m.cr {
                *r = (next() & 15) as u8;
            }
            m.lr = 0xffff0003;
            m.ctr = next();
            m.xer = next();
            m.fpscr = next();
            m.guest_memory = memory.clone();
            (m, 512)
        })
        .collect()
}
impl ProjectService {
    pub(crate) fn current_validation_runs(&self, ir: &Function) -> Result<Vec<Value>> {
        let expected_hash = crate::hash_bytes(&serde_json::to_vec(ir)?);
        let rows=self.db.prepare("SELECT summary,report_path,report_hash FROM function_validation_runs WHERE function=?1 AND context_version=?2 ORDER BY id")?.query_map(params![ir.id,woomy_ir::runtime::EXECUTION_CONTEXT_VERSION],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        let mut results = vec![];
        for (text, path, hash) in rows {
            if text.len() > 65536 {
                continue;
            }
            let value: Value = serde_json::from_str(&text)?;
            if value["runtime_abi_version"] != RUNTIME_ABI_VERSION
                || value["ir_hash"] != expected_hash
                || value["versions"] != serde_json::to_value(ImageContextVersions::CURRENT)?
            {
                continue;
            }
            let path = self.output.path(&path)?;
            if !path.is_file() {
                continue;
            }
            let report = crate::project::read_bounded(&path, 64 * 1024 * 1024)?;
            if crate::hash_bytes(&report) == hash {
                results.push(value);
            }
        }
        Ok(results)
    }
    pub fn validate_function_synthetic(&self, id: &str) -> Result<Value> {
        if self.verify_ir(id)?["verification"]["structural_valid"] != true {
            return Err(Error::Analysis("INVALID_IR".into()));
        }
        let f = self.read_ir(id)?;
        let image = self.guest_image(Some(&f.module))?;
        let (program, code, memory) = register_only_program(&image, f)?;
        let generated = woomy_codegen::generate_program(&program).map_err(Error::Analysis)?;
        let source = generated
            .source
            .ok_or_else(|| Error::Analysis("CODEGEN_UNSUPPORTED".into()))?;
        let directory = self.output.path("generated/validation")?;
        std::fs::create_dir_all(&directory)?;
        self.output.path("generated/validation")?;
        let compiler =
            woomy_codegen::compiler::NativeCompiler::detect().map_err(Error::Analysis)?;
        let build = compiler.build(&source, &directory)?;
        let executable = build
            .executable
            .ok_or_else(|| Error::Analysis(format!("COMPILER_ERROR: {:?}", build.record)))?;
        let key = &generated.manifest.cache_key;
        self.output.write_atomic(
            &format!("generated/validation/{key}.cpp"),
            source.as_bytes(),
        )?;
        self.output.write_atomic(
            &format!("generated/validation/{key}.build.json"),
            &serde_json::to_vec(&build.record)?,
        )?;
        self.output.write_atomic(
            &format!("generated/validation/{key}.manifest.json"),
            &serde_json::to_vec(&generated.manifest)?,
        )?;
        let temporary = tempfile::tempdir_in(self.output.path("generated/validation")?)?;
        let exe = temporary.path().join("validation.exe");
        std::fs::write(&exe, &executable)?;
        self.output
            .write_atomic(&format!("generated/validation/{key}.exe"), &executable)?;
        let inputs = corpus(&memory);
        let native = woomy_codegen::validation::run(&exe, &inputs)
            .map_err(|e| Error::Analysis(e.to_string()))?;
        let mut counts = BTreeMap::from([
            ("MATCH".to_owned(), 0usize),
            ("MISMATCH".into(), 0),
            ("INCONCLUSIVE".into(), 0),
        ]);
        let mut pairs = [0usize; 3];
        let mut cases = vec![];
        for (index, ((initial, budget), actual)) in
            inputs.into_iter().zip(native.executions).enumerate()
        {
            let comparison = woomy_codegen::validation::compare_program(
                &program,
                &code,
                initial.clone(),
                budget,
                actual,
            );
            *counts.entry(comparison.status.clone()).or_default() += 1;
            pairs[0] += usize::from(comparison.ppc_ir_equal);
            pairs[1] += usize::from(comparison.ir_native_equal);
            pairs[2] += usize::from(comparison.ppc_native_equal);
            cases.push(
                json!({"case":index,"initial":initial,"budget":budget,"comparison":comparison}),
            );
        }
        let state = if counts["MISMATCH"] > 0 {
            "MISMATCH"
        } else if counts["INCONCLUSIVE"] > 0 || cases.len() != 512 {
            "INCONCLUSIVE"
        } else {
            "DIFFERENTIAL_MATCH_SYNTHETIC_DOMAIN"
        };
        let summary = json!({"function":id,"domain":"GAME_FUNCTION_SYNTHETIC_DOMAIN","status":state,"initial_context_state":"EXECUTION_READY","final_context_state":"EXECUTED","image_hash":image.hash(),"ir_hash":crate::hash_bytes(&serde_json::to_vec(&program.functions[0])?),"versions":ImageContextVersions::CURRENT,"runtime_abi_version":RUNTIME_ABI_VERSION,"seed":0x62a7_c39du32,"cases":cases.len(),"results":counts,"ppc_ir_equal":pairs[0],"ir_native_equal":pairs[1],"ppc_native_equal":pairs[2],"actual_game_execution_ready":false,"real_game_context_match":false,"input_provenance":"ExplicitTestInput: all registers; LoaderDerived: registered executable blocks","control_constraint":"LR=0xFFFF0003 denotes explicit outer harness return, not a Cafe caller","report":format!("reports/validation-{key}.json")});
        let report = json!({"summary":summary,"program":program,"source_words":code,"manifest":generated.manifest,"build":build.record,"process":native.process,"cases":cases});
        let bytes = serde_json::to_vec(&report)?;
        let path = format!("reports/validation-{key}.json");
        self.output.write_atomic(&path, &bytes)?;
        self.db.execute("INSERT INTO function_validation_runs VALUES(?1,?2,?3,?4,'GAME_FUNCTION_SYNTHETIC_DOMAIN',?5,?6,?7,?8) ON CONFLICT(id) DO UPDATE SET status=excluded.status,report_hash=excluded.report_hash,summary=excluded.summary",params![key,id,image.hash(),woomy_ir::runtime::EXECUTION_CONTEXT_VERSION,state,path,crate::hash_bytes(&bytes),serde_json::to_string(&summary)?])?;
        Ok(summary)
    }
}
