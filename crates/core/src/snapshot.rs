//! Woomy-owned bounded JSON snapshot. Origin is a declaration, not authentication.
use crate::{cafe::CafeGuestImage, context::InputProvenance, Error, ProjectService, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use woomy_ir::{
    execution::Machine,
    runtime::{GuestMemory, ImageContextVersions, Region, EXECUTE},
    Function,
};
pub const SNAPSHOT_VERSION: u32 = 1;
pub const REFERENCE_REPLAY_VERSION: u32 = 1;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Origin {
    Synthetic,
    CemuReference,
    WiiuReference,
    ExternalReference,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registers {
    pub gpr: [u32; 32],
    pub fpr: [u64; 32],
    pub cr: [u8; 8],
    pub lr: u32,
    pub ctr: u32,
    pub xer: u32,
    pub fpscr: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRegion {
    pub base: u32,
    pub permissions: u32,
    pub kind: u32,
    pub provenance: InputProvenance,
    pub source: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSnapshot {
    pub format_version: u32,
    pub versions: ImageContextVersions,
    pub module: String,
    pub module_sha256: String,
    pub image_hash: String,
    pub function: String,
    pub function_entry: u32,
    pub cia: u32,
    pub origin: Origin,
    pub source: String,
    pub registers: Registers,
    pub register_provenance: InputProvenance,
    pub regions: Vec<SnapshotRegion>,
    pub expected_exit_sha256: Option<String>,
}
fn invalid(why: &str) -> Error {
    Error::Analysis(format!("INVALID_SNAPSHOT: {why}"))
}
fn hash_valid(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
impl ReferenceSnapshot {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(invalid("file byte budget"));
        }
        let snapshot: Self = serde_json::from_slice(bytes)?;
        snapshot.validate_structure()?;
        Ok(snapshot)
    }
    pub fn validate_structure(&self) -> Result<()> {
        if self.format_version != SNAPSHOT_VERSION || self.versions != ImageContextVersions::CURRENT
        {
            return Err(invalid("stale snapshot, loader or context version"));
        }
        if !hash_valid(&self.module_sha256)
            || !hash_valid(&self.image_hash)
            || self
                .expected_exit_sha256
                .as_ref()
                .is_some_and(|h| !hash_valid(h))
            || self.source.is_empty()
            || self.source.len() > 4096
            || self.module.is_empty()
            || self.module.len() > 4096
            || self.function.is_empty()
            || self.function.len() > 4096
        {
            return Err(invalid("identity/provenance/hash"));
        }
        if self.cia != self.function_entry
            || self.cia & 3 != 0
            || self.registers.cr.iter().any(|v| *v > 15)
            || self.regions.len() > 256
            || self.regions.iter().map(|r| r.bytes.len()).sum::<usize>() > 1048576
        {
            return Err(invalid("entry/register/memory bounds"));
        }
        if self.register_provenance == InputProvenance::Unknown
            || self.regions.iter().any(|r| {
                r.provenance == InputProvenance::Unknown
                    || r.source.is_empty()
                    || r.source.len() > 4096
                    || !hash_valid(&r.sha256)
                    || crate::hash_bytes(&r.bytes) != r.sha256
            })
        {
            return Err(invalid("unknown state or region hash mismatch"));
        }
        if self.origin != Origin::Synthetic
            && (self.register_provenance != InputProvenance::ReferenceSnapshot
                || self
                    .regions
                    .iter()
                    .any(|r| r.provenance != InputProvenance::ReferenceSnapshot))
        {
            return Err(invalid(
                "reference capture requires reference provenance for every supplied value",
            ));
        }
        let memory = self.guest_memory();
        if !memory.valid() || memory.locate(self.cia, 4, EXECUTE).is_none() {
            return Err(invalid("overlap/range/permissions/executable entry"));
        }
        Ok(())
    }
    fn guest_memory(&self) -> GuestMemory {
        GuestMemory {
            regions: self
                .regions
                .iter()
                .map(|r| Region {
                    base: r.base,
                    permissions: r.permissions,
                    kind: r.kind,
                    provenance: format!("{:?}:{}", r.provenance, r.source),
                    bytes: r.bytes.clone(),
                })
                .collect(),
        }
    }
    pub fn validate_for(&self, image: &CafeGuestImage, f: &Function) -> Result<()> {
        self.validate_structure()?;
        image.verify()?;
        if self.module != image.module()
            || self.module_sha256 != image.module_hash()
            || self.module != f.module
            || self.module_sha256 != f.module_hash
            || self.image_hash != image.hash()
            || self.function != f.id
            || self.function_entry != f.entry
            || !woomy_ir::verify(f).structural_valid
        {
            return Err(invalid("module/function/image identity mismatch"));
        }
        let memory = self.guest_memory();
        for b in &f.blocks {
            for s in &b.sources {
                let expected = image.slice(s.address, 4)?;
                let (i, n) = memory
                    .locate(s.address, 4, EXECUTE)
                    .ok_or_else(|| invalid("missing executable source"))?;
                if memory.regions[i].bytes[n..n + 4] != expected.bytes {
                    return Err(invalid(
                        "snapshot executable bytes differ from verified image",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn machine(&self) -> Result<Machine> {
        self.validate_structure()?;
        let r = &self.registers;
        let mut m = Machine::new(0, vec![]);
        m.gpr = r.gpr;
        m.fpr = r.fpr;
        m.cr = r.cr;
        m.lr = r.lr;
        m.ctr = r.ctr;
        m.xer = r.xer;
        m.fpscr = r.fpscr;
        m.guest_memory = self.guest_memory();
        Ok(m)
    }
    pub fn metadata(&self) -> Value {
        json!({"format_version":self.format_version,"versions":self.versions,"module":self.module,"module_hash":self.module_sha256,"image_hash":self.image_hash,"function":self.function,"entry":self.function_entry,"cia":self.cia,"origin":self.origin,"source":self.source,"regions":self.regions.len(),"bytes":self.regions.iter().map(|r|r.bytes.len()).sum::<usize>(),"validation_domain":if self.origin==Origin::Synthetic {"GAME_FUNCTION_SYNTHETIC_DOMAIN"}else{"GAME_FUNCTION_REFERENCE_CONTEXT"},"authenticity":"EXTERNAL_ORIGIN_DECLARATION_NOT_AUTHENTICATED","execution_ready":false})
    }
}
impl ProjectService {
    pub fn replay_snapshot(&self, path: &str) -> Result<Value> {
        let bytes = crate::project::read_bounded(&self.output.path(path)?, MAX_SNAPSHOT_BYTES)?;
        let snapshot = ReferenceSnapshot::parse(&bytes)?;
        let function = self.read_ir(&snapshot.function)?;
        let image = self.guest_image(Some(&function.module))?;
        snapshot.validate_for(&image, &function)?;
        let (program, code, _, _) = crate::bootstrap::entry_program(&image, function, None)?;
        if !program.source_barriers.is_empty() {
            return Err(invalid("unresolved executable relocation"));
        }
        let initial = snapshot.machine()?;
        let ppc = woomy_ir::execution::execute_ppc_program(&program, &code, initial.clone(), 512);
        let ir = woomy_ir::execution::execute_ir_program(&program, initial.clone(), 512);
        let mut native = None;
        let mut native_reason = None;
        match woomy_codegen::generate_program(&program) {
            Ok(generated) => {
                if let Some(source) = generated.source {
                    let directory = self.output.path("generated/replay")?;
                    std::fs::create_dir_all(&directory)?;
                    self.output.path("generated/replay")?;
                    let compiler = woomy_codegen::compiler::NativeCompiler::detect()
                        .map_err(Error::Analysis)?;
                    let build = compiler.build(&source, &directory)?;
                    let binary = build.executable.ok_or_else(|| {
                        Error::Analysis(format!("REPLAY_COMPILER_ERROR: {:?}", build.record))
                    })?;
                    let temp = tempfile::tempdir_in(&directory)?;
                    let exe = temp.path().join("replay.exe");
                    std::fs::write(&exe, binary)?;
                    native = woomy_codegen::validation::run(&exe, &[(initial.clone(), 512)])?
                        .executions
                        .into_iter()
                        .next();
                } else {
                    native_reason = Some("CODEGEN_UNSUPPORTED".to_owned());
                }
            }
            Err(reason) => native_reason = Some(reason),
        }
        let completed = matches!(
            ppc.exit,
            woomy_ir::execution::Exit::Returned { .. } | woomy_ir::execution::Exit::Trapped { .. }
        ) && matches!(
            ir.exit,
            woomy_ir::execution::Exit::Returned { .. } | woomy_ir::execution::Exit::Trapped { .. }
        );
        let same = ppc == ir && native.as_ref().is_some_and(|n| n == &ppc);
        let digest = crate::hash_bytes(&serde_json::to_vec(&ppc)?);
        let expected = snapshot.expected_exit_sha256.as_ref().map(|h| h == &digest);
        let mismatch =
            ppc != ir || native.as_ref().is_some_and(|n| n != &ppc) || expected == Some(false);
        let summary = json!({"replay_version":REFERENCE_REPLAY_VERSION,"runtime_abi_version":woomy_ir::runtime::RUNTIME_ABI_VERSION,"snapshot":snapshot.metadata(),"snapshot_sha256":crate::hash_bytes(&bytes),"status":if mismatch {"REPLAY_MISMATCH"}else if completed && same {"REPLAY_EXECUTION_AGREEMENT"}else{"INCONCLUSIVE"},"ppc_ir_equal":ppc==ir,"ppc_native_equal":native.as_ref().map(|n|n==&ppc),"native_unavailable":native_reason,"exit_sha256":digest,"expected_exit_hash_equal":expected,"real_game_context_match":false,"reference_authenticity":"NOT_ATTESTED_BY_ORIGIN_LABEL_OR_DIGEST","limit_steps":512});
        let report = json!({"summary":summary,"initial":initial,"program":program,"ppc":ppc,"ir":ir,"native":native});
        let destination = format!("reports/replay-{}.json", crate::hash_bytes(&bytes));
        self.output
            .write_atomic(&destination, &serde_json::to_vec_pretty(&report)?)?;
        let mut result = summary;
        result["report"] = json!(destination);
        Ok(result)
    }
    pub fn inspect_snapshot(&self, path: &str, validate: bool) -> Result<Value> {
        let bytes = crate::project::read_bounded(&self.output.path(path)?, MAX_SNAPSHOT_BYTES)?;
        let snapshot = ReferenceSnapshot::parse(&bytes)?;
        let mut value = snapshot.metadata();
        if validate {
            let ir = self.read_ir(&snapshot.function)?;
            let image = self.guest_image(Some(&ir.module))?;
            snapshot.validate_for(&image, &ir)?;
            value["validation"] = json!("STRUCTURALLY_VALID_FOR_CURRENT_IMAGE");
            self.persist_product(
                "SNAPSHOT",
                &crate::hash_bytes(&bytes),
                &ir.module,
                image.hash(),
                &value,
            )?;
        }
        Ok(value)
    }
}
