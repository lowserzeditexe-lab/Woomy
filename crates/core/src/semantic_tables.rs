//! Relocation-backed storage candidates. These never enter the machine call graph.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use woomy_ir::runtime::{GuestMemory, EXECUTE, READ, WRITE};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryEvidence {
    pub address: u32,
    pub relocation_index: usize,
    pub relocation_target: Option<u32>,
    pub applied_local_addr32: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerEntry {
    pub evidence: EntryEvidence,
    pub observed: Option<u32>,
    pub target_function: Option<String>,
    pub immutable_storage: bool,
    pub conflicts: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCandidate {
    pub id: String,
    pub address: u32,
    pub byte_length: u32,
    pub entry_width: u32,
    pub status: String,
    pub entries: Vec<PointerEntry>,
    pub unresolved_entries: usize,
    pub consumers: Vec<serde_json::Value>,
    pub call_sites: Vec<u32>,
    pub provenance: String,
}
pub fn scan(
    memory: &GuestMemory,
    entries: &[EntryEvidence],
    functions: &BTreeMap<u32, String>,
) -> Result<Vec<TableCandidate>, &'static str> {
    if !memory.valid_with_limit(256 * 1024 * 1024) || entries.len() > 524288 {
        return Err("SEMANTIC_TABLE_INPUT_LIMIT");
    }
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|e| e.address);
    if sorted.windows(2).any(|e| e[0].address == e[1].address) {
        return Err("SEMANTIC_TABLE_AMBIGUOUS_RELOCATION");
    }
    let mut groups: Vec<Vec<PointerEntry>> = vec![];
    for entry in sorted {
        let mut conflicts = vec![];
        let region = memory.locate(entry.address, 4, READ);
        let observed = region.map(|(i, n)| {
            u32::from_be_bytes(memory.regions[i].bytes[n..n + 4].try_into().unwrap())
        });
        let immutable = region.is_some_and(|(i, _)| memory.regions[i].permissions & WRITE == 0);
        if !immutable {
            conflicts.push("MUTABLE_OR_UNMAPPED_STORAGE".into());
        }
        if entry.address & 3 != 0 {
            conflicts.push("UNALIGNED_ENTRY".into());
        }
        if !entry.applied_local_addr32 || entry.relocation_target != observed {
            conflicts.push("NO_MATCHING_APPLIED_LOCAL_ADDR32".into());
        }
        let target = observed
            .filter(|target| memory.locate(*target, 4, EXECUTE).is_some())
            .and_then(|target| functions.get(&target).cloned());
        if target.is_none() {
            conflicts.push("NOT_A_KNOWN_EXECUTABLE_FUNCTION_START".into());
        }
        let pointer = PointerEntry {
            evidence: entry,
            observed,
            target_function: target,
            immutable_storage: immutable,
            conflicts,
        };
        let contiguous = groups.last().and_then(|g| g.last()).is_some_and(|last| {
            last.evidence.address.checked_add(4) == Some(pointer.evidence.address)
        }) && groups.last().is_some_and(|g| g.len() < 256);
        if contiguous {
            groups.last_mut().unwrap().push(pointer);
        } else {
            groups.push(vec![pointer]);
        }
    }
    let mut tables = vec![];
    for entries in groups {
        // Raw executable-looking integers never seed candidates. At least one
        // applied relocation must name a known executable function start.
        if !entries.iter().any(|e| {
            e.target_function.is_some()
                && e.evidence.applied_local_addr32
                && e.evidence.relocation_target == e.observed
        }) {
            continue;
        }
        let address = entries[0].evidence.address;
        let unresolved = entries.iter().filter(|e| !e.conflicts.is_empty()).count();
        tables.push(TableCandidate {id:format!("table_candidate_{address:08X}"),address,byte_length:entries.len() as u32*4,entry_width:4,status:if unresolved==0 {"HYPOTHESIS"}else{"REJECTED_OR_CONFLICTING"}.into(),unresolved_entries:unresolved,entries,consumers:vec![],call_sites:vec![],provenance:"EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP + WOOMY_INFERRED; contiguous relocation run bounded to 256 entries, not proven source array boundaries; read-only image storage, no runtime mutation/alias proof; no vtable or machine-target promotion".into()});
        if tables.len() > 16384 {
            return Err("SEMANTIC_TABLE_CANDIDATE_LIMIT");
        }
    }
    Ok(tables)
}
