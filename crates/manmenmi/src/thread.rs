//! Guest fields are encoded individually; Rust layout is never a Cafe layout.
use super::*;
pub const THREAD_SIZE: u32 = 0x6a0;
pub const CONTEXT_SIZE: u32 = 0x320;
pub const THREAD_SOURCE: &str =
    "https://github.com/devkitPro/wut/blob/master/include/coreinit/thread.h";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadChunk {
    pub offset: u32,
    pub bytes: Vec<u8>,
    pub provenance: ValueOrigin,
    pub evidence: Evidence,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuestField {
    pub name: String,
    pub offset: u32,
    pub bytes: Vec<u8>,
    pub layout_evidence: Evidence,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CafeThreadObject {
    pub address: Known<Address32>,
    pub chunks: Vec<ThreadChunk>,
    pub fields: Vec<GuestField>,
    pub stack: Option<Known<StackBounds>>,
    pub initialization: String,
}
impl CafeThreadObject {
    pub fn range(&self) -> GuestRange {
        GuestRange {
            start: self.address.value,
            length: THREAD_SIZE,
        }
    }
    pub fn model_zeroed(
        address: Address32,
        stack: StackBounds,
        entry: Address32,
        id: u16,
    ) -> Result<Self> {
        stack.range()?;
        let mut bytes = vec![0; THREAD_SIZE as usize];
        let mut fields = vec![];
        for (name, offset, encoded) in [
            (
                "context.tag",
                0x0,
                0x4f53_436f_6e74_7874u64.to_be_bytes().to_vec(),
            ),
            ("tag", 0x320, 0x7448_7244u32.to_be_bytes().to_vec()),
            ("state", 0x324, vec![2]),
            ("id", 0x326, id.to_be_bytes().to_vec()),
            ("stackStart", 0x394, stack.high.get().to_be_bytes().to_vec()),
            ("stackEnd", 0x398, stack.low.get().to_be_bytes().to_vec()),
            ("entryPoint", 0x39c, entry.get().to_be_bytes().to_vec()),
        ] {
            let end = offset as usize + encoded.len();
            bytes
                .get_mut(offset as usize..end)
                .ok_or("THREAD_FIELD_RANGE")?
                .copy_from_slice(&encoded);
            fields.push(GuestField { name:name.into(),offset,bytes:encoded,layout_evidence:Evidence { class:EvidenceClass::PublicImplementationObservation,source:THREAD_SOURCE.into(),detail:"WUT checked offsets; field values belong solely to this model, not a captured Cafe thread".into() }});
        }
        let result=Self { address:Known::model(address,"Dedicated deterministic guest runtime address; not a Cafe process address"),
            chunks:vec![ThreadChunk { offset:0,bytes,provenance:ValueOrigin::RuntimeProvided,evidence:Evidence::model("All 0x6A0 model storage bytes explicitly initialized to zero before listed fields; other field meanings remain unknown") }],fields,
            stack:Some(Known::model(stack,"Explicit bounded model stack")),initialization:"EXPLICIT_MODEL_ZERO_FILL_THEN_FIELDS".into() };
        result.validate(Domain::CafeBootstrapModel)?;
        Ok(result)
    }
    pub fn captured(
        address: Known<Address32>,
        chunks: Vec<ThreadChunk>,
        stack: Option<Known<StackBounds>>,
    ) -> Result<Self> {
        let result = Self {
            address,
            chunks,
            fields: vec![],
            stack,
            initialization: "EXACT_CAPTURED_CHUNKS_UNKNOWN_GAPS_UNMAPPED".into(),
        };
        result.validate(Domain::GameFunctionReferenceContext)?;
        Ok(result)
    }
    pub fn validate(&self, domain: Domain) -> Result<()> {
        if !self.address.valid_for(domain)
            || self.address.value.get() == 0
            || self.address.value.get() & 7 != 0
            || !self.range().valid()
            || self.chunks.is_empty()
            || self.chunks.len() > 64
            || self.fields.len() > 64
            || self.initialization.is_empty()
            || self.initialization.len() > 4096
        {
            return Err("INVALID_THREAD_OBJECT");
        }
        if domain == Domain::GameFunctionReferenceContext
            && self.initialization != "EXACT_CAPTURED_CHUNKS_UNKNOWN_GAPS_UNMAPPED"
        {
            return Err("THREAD_DOMAIN_PROVENANCE");
        }
        if let Some(stack) = &self.stack {
            if !stack.valid_for(domain) {
                return Err("THREAD_STACK_PROVENANCE");
            }
            stack.value.range()?;
        }
        for (i, c) in self.chunks.iter().enumerate() {
            let end = c.offset as u64 + c.bytes.len() as u64;
            let known = Known {
                value: (),
                provenance: c.provenance,
                evidence: c.evidence.clone(),
            };
            if c.bytes.is_empty()
                || end > THREAD_SIZE as u64
                || !known.valid_for(domain)
                || self.chunks[..i].iter().any(|p| {
                    (p.offset as u64) < end
                        && (c.offset as u64) < p.offset as u64 + p.bytes.len() as u64
                })
            {
                return Err("THREAD_CHUNK_RANGE_OVERLAP_OR_PROVENANCE");
            }
        }
        for f in &self.fields {
            if f.name.is_empty()
                || f.name.len() > 128
                || ![1, 2, 4, 8].contains(&f.bytes.len())
                || !f.layout_evidence.valid()
                || !self.chunks.iter().any(|c| {
                    f.offset >= c.offset
                        && (f.offset - c.offset) as usize <= c.bytes.len()
                        && c.bytes.get(
                            (f.offset - c.offset) as usize
                                ..(f.offset - c.offset) as usize + f.bytes.len(),
                        ) == Some(f.bytes.as_slice())
                })
            {
                return Err("THREAD_FIELD_PROOF_MISMATCH");
            }
        }
        Ok(())
    }
    pub fn regions(&self) -> Result<Vec<Region>> {
        self.chunks
            .iter()
            .map(|c| {
                Ok(Region {
                    base: self.address.value.offset(c.offset)?.get(),
                    permissions: READ | WRITE,
                    kind: 4,
                    provenance: format!("{:?}:OSThread:{}", c.provenance, self.initialization),
                    bytes: c.bytes.clone(),
                })
            })
            .collect()
    }
    /// Initialization is atomic on failure and is not a guest instruction write.
    pub fn materialize(&self, memory: &mut GuestMemory, domain: Domain) -> Result<()> {
        self.validate(domain)?;
        if !memory.valid() {
            return Err("RUNTIME_MEMORY_LAYOUT_OR_BUDGET");
        }
        let placements: Vec<_> = self
            .chunks
            .iter()
            .map(|c| {
                memory
                    .locate(
                        self.address.value.offset(c.offset)?.get(),
                        c.bytes.len(),
                        WRITE,
                    )
                    .ok_or("THREAD_MAPPING_NOT_WRITABLE")
            })
            .collect::<Result<_>>()?;
        for (c, (i, n)) in self.chunks.iter().zip(placements) {
            memory.regions[i].bytes[n..n + c.bytes.len()].copy_from_slice(&c.bytes);
        }
        Ok(())
    }
}
