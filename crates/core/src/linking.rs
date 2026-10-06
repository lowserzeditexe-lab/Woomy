//! Read-only generic audit of undefined symbol references. No binding policy.
use crate::{project::read_bounded, Error, ProjectService, Result};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use woomy_binary::Module;
pub const LINK_AUDIT_VERSION: u32 = 1;

pub fn audit(module: &Module) -> Result<Value> {
    if module.symbols.len() > 65536 || module.relocations.len() > 2_000_000 {
        return Err(Error::Analysis("LINK_AUDIT_INPUT_LIMIT".into()));
    }
    let lookup: BTreeMap<_, _> = module
        .symbols
        .iter()
        .map(|s| ((s.table, s.index), s))
        .collect();
    if lookup.len() != module.symbols.len() {
        return Err(Error::Analysis(
            "LINK_AUDIT_DUPLICATE_SYMBOL_IDENTITY".into(),
        ));
    }
    let mut section_counts = BTreeMap::<u32, usize>::new();
    let positions: Vec<_> = module
        .relocations
        .iter()
        .map(|r| {
            let n = section_counts.entry(r.section).or_default();
            let result = *n;
            *n += 1;
            result
        })
        .collect();
    let symbols: Vec<_> = module.symbols.iter().filter(|s| s.section == 0).collect();
    if symbols.len() > 4096 {
        return Err(Error::Analysis("LINK_AUDIT_SYMBOL_LIMIT".into()));
    }
    let identities: std::collections::BTreeSet<_> =
        symbols.iter().map(|s| (s.table, s.index)).collect();
    let references: Vec<_> = module
        .relocations
        .iter()
        .enumerate()
        .filter(|(_, r)| identities.contains(&(r.symbol_table, r.symbol)))
        .collect();
    if references.len() > 4096 {
        return Err(Error::Analysis("LINK_AUDIT_REFERENCE_LIMIT".into()));
    }
    let mut families = BTreeMap::<u8, usize>::new();
    let attributes:Vec<_>=symbols.iter().map(|s| {
        let raw=module.sections.iter().find(|section|section.index==s.table).and_then(|section|{
            let offset=(s.index as usize).checked_mul(section.entry_size as usize)?;
            section.data.get(offset..offset.checked_add(16)?)
        });
        json!({"table":s.table,"index":s.index,"binding":s.info>>4,"type":s.info&15,"other":raw.map(|b|b[13]),"raw_record":raw})
    }).collect();
    let records: Vec<_> = references.iter().map(|(index,r)| {
        *families.entry(r.kind).or_default() += 1;
        let section=module.sections.iter().find(|s|s.index==r.section);
        let section_record=positions[*index];
        json!({"index":index,"section_record":section_record,"record":r,
            "relocation_section":section,"target_section":module.sections.iter().find(|s|s.index==r.target_section),
            "symbol_records":symbols.iter().filter(|s|s.table==r.symbol_table&&s.index==r.symbol).collect::<Vec<_>>(),
            "neighbors":module.relocations[index.saturating_sub(2)..(*index+3).min(module.relocations.len())].iter().map(|n|json!({"record":n,"symbol":lookup.get(&(n.symbol_table,n.symbol))})).collect::<Vec<_>>()})
    }).collect();
    let metadata:Vec<_>=module.sections.iter().filter(|s|s.kind==0x80000004).map(|s|json!({"section":s,"sha256":crate::hash_bytes(&s.data),"prefix_words_be":s.data.as_chunks::<4>().0.iter().take(24).map(|b|u32::from_be_bytes(*b)).collect::<Vec<_>>(),"interpretation":"PUBLIC_FILEINFO_LAYOUT_ONLY; no binding inferred from metadata"})).collect();
    Ok(
        json!({"version":LINK_AUDIT_VERSION,"evidence_domain":"EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP","resolution":"UNKNOWN","newly_resolved":0,
        "undefined_symbols":symbols,"symbol_attributes":attributes,"undefined_symbol_count":symbols.len(),"total_symbols":module.symbols.len(),"reference_count":records.len(),"total_relocations":module.relocations.len(),"families":families,"references":records,"fileinfo":metadata,"imports":module.imports,
        "policy":"Symbol names and initial zero values are not binding evidence. SHN_UNDEF is not SHN_ABS. No guest bytes changed.",
        "missing_evidence":["Justified Cafe binding rule applicable to the exact symbol attributes", "Authentic link map, producer documentation or reproducible reference loader observation", "Provider module identity and export proof if inter-module binding is alleged"],
        "public_behavior":"See docs/LINKING.md for pinned independent loader observations; these are not a retail Cafe reference"}),
    )
}
impl ProjectService {
    pub fn link_audit(&self, module: Option<&str>) -> Result<Value> {
        let image = self.guest_image(module)?;
        let path: String = self.db.query_row(
            "SELECT path FROM modules WHERE id=?1",
            [image.module()],
            |r| r.get(0),
        )?;
        let bytes = read_bounded(&self.input(&path)?, woomy_binary::MAX_MODULE_BYTES)?;
        if crate::hash_bytes(&bytes) != image.module_hash() {
            return Err(Error::Analysis("STALE_MODULE".into()));
        }
        let parsed = woomy_binary::parse(&bytes)?;
        let mut report = audit(&parsed)?;
        report["module"] = json!(image.module());
        report["module_hash"] = json!(image.module_hash());
        report["image_hash"] = json!(image.hash());
        report["available_modules"] = self
            .rows(
                "SELECT id,path,hash,kind FROM modules WHERE status!='STALE' ORDER BY id",
                &[],
            )?
            .into();
        let key = crate::hash_bytes(&serde_json::to_vec(&report)?);
        let serialized = serde_json::to_vec_pretty(&report)?;
        if serialized.len() > 16 * 1024 * 1024 {
            return Err(Error::Analysis("LINK_AUDIT_ARTIFACT_LIMIT".into()));
        }
        self.persist_product("LINK_AUDIT", image.module(), image.module(), &key, &report)?;
        let destination = format!("reports/link-audit-{}.json", image.module_hash());
        self.output.write_atomic(&destination, &serialized)?;
        report["report"] = json!(destination);
        Ok(report)
    }
}
