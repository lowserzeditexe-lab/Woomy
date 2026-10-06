//! Derived Cafe image at linked section addresses. This is not a captured process.
use crate::{hash_bytes, Error, Result};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use woomy_binary::{Module, Relocation, Symbol};
use woomy_ir::runtime::{GuestMemory, ImageContextVersions, Region, EXECUTE, READ, WRITE};

#[derive(Debug, Clone, Serialize)]
pub struct ImageRegion {
    pub section: u32,
    pub name: String,
    pub section_type: u32,
    pub flags: u32,
    pub start: u32,
    pub end: u64,
    pub alignment: u32,
    pub file_range: Option<(u32, u64)>,
    pub initialization: String,
    pub permissions: u32,
    pub initial_hash: String,
    pub relocated_hash: String,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct RelocationResult {
    pub index: usize,
    pub section: u32,
    pub target_section: u32,
    pub place: u32,
    pub kind: u8,
    pub symbol_table: u32,
    pub symbol_index: u32,
    pub symbol: Option<String>,
    pub addend: Option<i32>,
    pub target: Option<u32>,
    pub import: Option<(String, String)>,
    pub original: Vec<u8>,
    pub relocated: Vec<u8>,
    pub state: String,
    pub reason: String,
}
#[derive(Debug)]
pub struct CafeGuestImage {
    module: String,
    module_hash: String,
    module_kind: String,
    entry: u32,
    versions: ImageContextVersions,
    memory: GuestMemory,
    regions: Vec<ImageRegion>,
    relocations: Vec<RelocationResult>,
    excluded: Vec<Value>,
    unresolved_ranges: BTreeMap<u64, u64>,
    identity: String,
}
fn fail(reason: &str) -> Error {
    Error::Analysis(format!("CAFE_IMAGE_INVALID: {reason}"))
}
fn width(kind: u8) -> usize {
    if [3, 4, 5, 6].contains(&kind) {
        2
    } else {
        4
    }
}
impl CafeGuestImage {
    pub fn load(
        module: &str,
        module_hash: &str,
        kind: &str,
        parsed: &Module,
        file_length: usize,
    ) -> Result<Self> {
        if module.is_empty()
            || module.len() > 4096
            || module_hash.len() != 64
            || !module_hash.bytes().all(|b| b.is_ascii_hexdigit())
            || !["RPX", "RPL"].contains(&kind)
        {
            return Err(fail("identity"));
        }
        let mut image = Self {
            module: module.into(),
            module_hash: module_hash.into(),
            module_kind: kind.into(),
            entry: parsed.entry_point,
            versions: ImageContextVersions::CURRENT,
            memory: GuestMemory::default(),
            regions: vec![],
            relocations: vec![],
            excluded: vec![],
            unresolved_ranges: BTreeMap::new(),
            identity: String::new(),
        };
        let mut total = 0usize;
        let mut indices = BTreeSet::new();
        for s in &parsed.sections {
            if !indices.insert(s.index) {
                return Err(fail("duplicate section identity"));
            }
            if s.flags & 2 == 0 || s.size == 0 {
                continue;
            }
            if ![1, 8].contains(&s.kind) {
                image.excluded.push(json!({"section":s.index,"kind":s.kind,"address":s.address,"size":s.size,"reason":"LOADER_METADATA_OR_UNSUPPORTED_SECTION: not executable or ordinary program storage"}));
                continue;
            }
            if s.flags & !(0x08000000 | 7) != 0 {
                return Err(fail("unsupported allocated section flags"));
            }
            total = total
                .checked_add(s.size as usize)
                .ok_or_else(|| fail("size overflow"))?;
            let end = s.address as u64 + s.size as u64;
            if total > woomy_binary::MAX_MODULE_BYTES
                || s.size as usize > woomy_binary::MAX_SECTION_BYTES
                || end > 0x1_0000_0000
                || (s.alignment > 1
                    && (!s.alignment.is_power_of_two() || !s.address.is_multiple_of(s.alignment)))
            {
                return Err(fail("size, address or alignment"));
            }
            let (bytes, initialization, file_range) = if s.kind == 8 {
                if !s.data.is_empty() || s.flags & (4 | 0x08000000) != 0 {
                    return Err(fail("NOBITS metadata"));
                }
                (vec![0; s.size as usize], "ZeroInitializedByLoader", None)
            } else {
                let file_end = s.offset as u64 + s.file_size as u64;
                if s.data.len() != s.size as usize
                    || file_end > file_length as u64
                    || (s.flags & 0x08000000 == 0 && s.file_size != s.size)
                    || (s.flags & 0x08000000 != 0 && s.file_size < 4)
                {
                    return Err(fail("section byte/file range mismatch"));
                }
                (
                    s.data.clone(),
                    if s.flags & 0x08000000 != 0 {
                        "LoaderDerivedDecompressed"
                    } else {
                        "LoaderDerived"
                    },
                    Some((s.offset, file_end)),
                )
            };
            let permissions = READ
                | if s.flags & 1 != 0 { WRITE } else { 0 }
                | if s.flags & 4 != 0 { EXECUTE } else { 0 };
            let hash = hash_bytes(&bytes);
            image.regions.push(ImageRegion {section:s.index,name:s.name.clone(),section_type:s.kind,flags:s.flags,start:s.address,end,alignment:s.alignment,file_range,initialization:initialization.into(),permissions,initial_hash:hash.clone(),relocated_hash:hash,evidence:"PUBLIC_IMPLEMENTATION_OBSERVATION: wut-tools ELF flags/compression; PPC ELF NOBITS; Cemu zero-fill cross-check. WOOMY_INFERRED: identity placement at linked sh_addr; no live process address claim".into()});
            image.memory.regions.push(Region {
                base: s.address,
                permissions,
                kind: if permissions & EXECUTE != 0 {
                    0
                } else if permissions & WRITE != 0 {
                    2
                } else {
                    1
                },
                provenance: format!(
                    "{initialization}:{}:section:{}:linked-address",
                    image.module_hash, s.index
                ),
                bytes,
            });
        }
        if !image
            .memory
            .valid_with_limit(woomy_binary::MAX_MODULE_BYTES)
        {
            return Err(fail("overlap or guest memory layout"));
        }
        if image.memory.locate(image.entry, 4, EXECUTE).is_none() || image.entry & 3 != 0 {
            return Err(fail("entry is not executable"));
        }
        if parsed.relocations.len() > 2_000_000 {
            return Err(fail("relocation budget"));
        }
        let mut symbols: BTreeMap<(u32, u32), Vec<&Symbol>> = BTreeMap::new();
        for s in &parsed.symbols {
            symbols.entry((s.table, s.index)).or_default().push(s);
        }
        // Mark every member of each overlapping interval cluster before any write.
        let mut ranges: Vec<_> = parsed
            .relocations
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind != 0)
            .map(|(i, r)| (r.offset as u64, r.offset as u64 + width(r.kind) as u64, i))
            .collect();
        ranges.sort_unstable();
        let mut conflicts = BTreeSet::new();
        let mut previous = None;
        let mut end = 0;
        for (start, stop, i) in ranges {
            if start < end {
                if let Some(p) = previous {
                    conflicts.insert(p);
                }
                conflicts.insert(i);
            }
            if start >= end || stop > end {
                previous = Some(i);
                end = stop;
            }
        }
        for (index, r) in parsed.relocations.iter().enumerate() {
            let result = image.apply(index, r, parsed, &symbols, conflicts.contains(&index));
            if result.state != "APPLIED_PROVEN_LOCAL" {
                let end = r.offset as u64 + width(r.kind) as u64;
                image
                    .unresolved_ranges
                    .entry(r.offset as u64)
                    .and_modify(|old| *old = (*old).max(end))
                    .or_insert(end);
            }
            image.relocations.push(result);
        }
        for (meta, region) in image.regions.iter_mut().zip(&image.memory.regions) {
            meta.relocated_hash = hash_bytes(&region.bytes);
        }
        image.identity = image.compute_identity()?;
        image.verify()?;
        Ok(image)
    }
    fn apply(
        &mut self,
        index: usize,
        r: &Relocation,
        parsed: &Module,
        symbols: &BTreeMap<(u32, u32), Vec<&Symbol>>,
        conflict: bool,
    ) -> RelocationResult {
        let mut result = RelocationResult {
            index,
            section: r.section,
            target_section: r.target_section,
            place: r.offset,
            kind: r.kind,
            symbol_table: r.symbol_table,
            symbol_index: r.symbol,
            symbol: None,
            addend: r.addend,
            target: None,
            import: None,
            original: vec![],
            relocated: vec![],
            state: "INVALID".into(),
            reason: String::new(),
        };
        let action = (|| -> std::result::Result<(), (&str, &str)> {
            if r.kind == 0 {
                return Err((
                    "UNSUPPORTED",
                    "NONE has no applied field; explicitly retained",
                ));
            }
            let w = width(r.kind);
            let ri = self
                .regions
                .iter()
                .position(|s| s.section == r.target_section)
                .ok_or(("INVALID", "unmapped relocation section"))?;
            let region = &self.memory.regions[ri];
            let n = r
                .offset
                .checked_sub(region.base)
                .ok_or(("INVALID", "place precedes target section"))? as usize;
            let bytes = region
                .bytes
                .get(n..n + w)
                .ok_or(("INVALID", "place exceeds target section"))?;
            result.original = bytes.to_vec();
            if conflict {
                return Err(("AMBIGUOUS", "overlapping relocation fields"));
            }
            if !(r.offset as usize).is_multiple_of(w) {
                return Err(("INVALID", "relocation alignment"));
            }
            let candidates = symbols
                .get(&(r.symbol_table, r.symbol))
                .ok_or(("INVALID", "symbol missing"))?;
            if candidates.len() != 1 {
                return Err(("AMBIGUOUS", "symbol identity is not unique"));
            }
            let s = candidates[0];
            result.symbol = Some(s.name.clone());
            let imports: Vec<_> = parsed
                .imports
                .iter()
                .filter(|i| i.section == s.section as u32)
                .collect();
            if imports.len() > 1 {
                return Err(("AMBIGUOUS", "multiple import section identities"));
            }
            if let Some(import) = imports.first() {
                if s.name.is_empty() {
                    return Err(("INVALID", "unnamed imported symbol"));
                }
                result.import = Some((import.library.clone(), s.name.clone()));
                return Err((
                    "DEFERRED_IMPORT",
                    "ImportBindingRequired; no guest address assigned",
                ));
            }
            if ![1, 4, 5, 6, 10].contains(&r.kind) {
                return Err(("UNSUPPORTED", "relocation family"));
            }
            let addend = r.addend.ok_or(("UNSUPPORTED", "implicit addend"))?;
            if s.section == 0 || s.section >= 0xff00 {
                return Err((
                    "UNSUPPORTED",
                    "UNDEFINED_OR_SPECIAL_SYMBOL_BINDING_REQUIRED: no local address proof",
                ));
            }
            if !self.regions.iter().any(|x| {
                x.section == s.section as u32 && s.value >= x.start && (s.value as u64) < x.end
            }) {
                return Err(("INVALID", "symbol not owned by a mapped local section"));
            }
            let target = u32::try_from(s.value as i64 + addend as i64)
                .map_err(|_| ("INVALID", "S+A overflow"))?;
            if self.memory.locate(target, 1, READ).is_none() {
                return Err(("INVALID", "S+A outside mapped local storage"));
            }
            result.target = Some(target);
            let patched = match r.kind {
                1 => target.to_be_bytes().to_vec(),
                4 => (target as u16).to_be_bytes().to_vec(),
                5 => ((target >> 16) as u16).to_be_bytes().to_vec(),
                6 => (((target as u64 + 0x8000) >> 16) as u16)
                    .to_be_bytes()
                    .to_vec(),
                10 => {
                    let raw = u32::from_be_bytes(bytes.try_into().expect("checked width"));
                    let delta = target as i64 - r.offset as i64;
                    if raw >> 26 != 18
                        || raw & 2 != 0
                        || target & 3 != 0
                        || !(-0x0200_0000..0x0200_0000).contains(&delta)
                        || self.memory.locate(target, 4, EXECUTE).is_none()
                    {
                        return Err(("INVALID", "REL24 opcode/range/alignment/executable target"));
                    }
                    ((raw & !0x03ff_fffc) | ((delta as u32) & 0x03ff_fffc))
                        .to_be_bytes()
                        .to_vec()
                }
                _ => unreachable!(),
            };
            self.memory.regions[ri].bytes[n..n + w].copy_from_slice(&patched);
            result.relocated = patched;
            Ok(())
        })();
        match action {
            Ok(()) => {
                result.state = "APPLIED_PROVEN_LOCAL".into();
                result.reason =
                    "DOCUMENTED: PPC ELF table 4-8; checked local identity placement".into();
            }
            Err((state, why)) => {
                result.state = state.into();
                result.reason = why.into();
            }
        }
        result
    }
    fn compute_identity(&self) -> Result<String> {
        let mut h = Sha256::new();
        h.update(serde_json::to_vec(&(
            &self.module,
            &self.module_hash,
            &self.module_kind,
            self.entry,
            self.versions,
            &self.regions,
            &self.excluded,
        ))?);
        for r in &self.relocations {
            h.update(serde_json::to_vec(r)?);
            h.update([0]);
        }
        Ok(format!("{:x}", h.finalize()))
    }
    pub fn verify(&self) -> Result<()> {
        if self.versions != ImageContextVersions::CURRENT
            || !self.memory.valid_with_limit(woomy_binary::MAX_MODULE_BYTES)
            || self.memory.regions.len() != self.regions.len()
            || self.memory.locate(self.entry, 4, EXECUTE).is_none()
            || self.identity != self.compute_identity()?
        {
            return Err(fail("version, layout or image identity"));
        }
        for (s, r) in self.regions.iter().zip(&self.memory.regions) {
            if s.start != r.base
                || s.end != r.base as u64 + r.bytes.len() as u64
                || s.permissions != r.permissions
                || s.relocated_hash != hash_bytes(&r.bytes)
                || s.evidence.is_empty()
            {
                return Err(fail("region provenance or bytes"));
            }
        }
        for r in self
            .relocations
            .iter()
            .filter(|r| r.state == "APPLIED_PROVEN_LOCAL")
        {
            let (i, n) = self
                .memory
                .locate(r.place, r.relocated.len(), READ)
                .ok_or_else(|| fail("applied field outside image"))?;
            if self.regions[i].section != r.target_section
                || self.memory.regions[i].bytes[n..n + r.relocated.len()] != r.relocated
                || r.target
                    .is_none_or(|t| self.memory.locate(t, 1, READ).is_none())
            {
                return Err(fail("applied relocation evidence"));
            }
        }
        Ok(())
    }
    pub fn hash(&self) -> &str {
        &self.identity
    }
    pub fn module_hash(&self) -> &str {
        &self.module_hash
    }
    pub fn module(&self) -> &str {
        &self.module
    }
    pub fn entry(&self) -> u32 {
        self.entry
    }
    pub fn regions(&self) -> &[ImageRegion] {
        &self.regions
    }
    pub fn relocations(&self) -> &[RelocationResult] {
        &self.relocations
    }
    pub fn memory(&self) -> &GuestMemory {
        &self.memory
    }
    pub fn range_resolved(&self, address: u32, len: usize) -> bool {
        self.memory.locate(address, len, READ).is_some()
            && !self
                .unresolved_ranges
                .range((address as u64).saturating_sub(3)..address as u64 + len as u64)
                .any(|(_, end)| *end > address as u64)
    }
    pub fn slice(&self, address: u32, len: usize) -> Result<Region> {
        if !self.range_resolved(address, len) {
            return Err(fail("unmapped or unresolved relocation in requested range"));
        }
        let (i, n) = self
            .memory
            .locate(address, len, READ)
            .ok_or_else(|| fail("slice range"))?;
        let region = &self.memory.regions[i];
        Ok(Region {
            base: address,
            permissions: region.permissions,
            kind: region.kind,
            provenance: region.provenance.clone(),
            bytes: region.bytes[n..n + len].to_vec(),
        })
    }
    pub fn status(&self) -> Value {
        let mut counts = BTreeMap::new();
        for state in [
            "APPLIED_PROVEN_LOCAL",
            "DEFERRED_IMPORT",
            "UNSUPPORTED",
            "AMBIGUOUS",
            "INVALID",
        ] {
            counts.insert(state, 0usize);
        }
        for r in &self.relocations {
            *counts.entry(&r.state).or_default() += 1;
        }
        json!({"module":self.module,"module_hash":self.module_hash,"module_kind":self.module_kind,"entry":self.entry,"versions":self.versions,"image_hash":self.identity,"placement":"LINKED_SECTION_ADDRESSES","process_state":"UNKNOWN","mapped_sections":self.regions.len(),"mapped_bytes":self.memory.regions.iter().map(|r|r.bytes.len()).sum::<usize>(),"zero_filled_bytes":self.regions.iter().filter(|r|r.section_type==8).map(|r|r.end-r.start as u64).sum::<u64>(),"read_regions":self.regions.iter().filter(|r|r.permissions & READ!=0).count(),"write_regions":self.regions.iter().filter(|r|r.permissions & WRITE!=0).count(),"execute_regions":self.regions.iter().filter(|r|r.permissions & EXECUTE!=0).count(),"relocations":{"total":self.relocations.len(),"states":counts},"verification":"STRUCTURALLY_VALID","execution_ready":false,"excluded_allocated_sections":self.excluded})
    }
}
