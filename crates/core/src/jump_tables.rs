//! A deliberately narrow switch proof: unsigned compare/bgt default, scaled index,
//! lwzx from immutable words, mtctr, bctr. No unbounded memory scanning.
use crate::module_model::ModuleModel;
use serde::Serialize;
use std::collections::BTreeMap;
use woomy_ppc::dataflow::{Atom, Dataflow, Definition, Target, Value};
use woomy_ppc::{Cfg, DecodedInstruction, Flow, Register, Semantics};

#[derive(Debug, Clone, Serialize)]
pub struct TableEvidence {
    pub kind: &'static str,
    pub source: Option<u32>,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct JumpTable {
    pub site: u32,
    pub address: u32,
    pub section: u32,
    pub status: &'static str,
    pub bound: Option<u32>,
    pub default_target: Option<u32>,
    pub entries: Vec<(u32, u32)>,
    pub evidence: Vec<TableEvidence>,
}
fn definition(facts: &Dataflow, pc: u32, r: Register) -> Option<u32> {
    let f = facts.used(pc, r)?;
    if f.definitions.len() != 1 {
        return None;
    }
    match f.definitions.first()? {
        Definition::Instruction(a) => Some(*a),
        _ => None,
    }
}
fn add_evidence(t: &mut JumpTable, kind: &'static str, source: Option<u32>, detail: &str) {
    t.evidence.push(TableEvidence {
        kind,
        source,
        detail: detail.into(),
    });
}
pub fn discover(
    entry: u32,
    cfg: &Cfg,
    code: &BTreeMap<u32, DecodedInstruction>,
    facts: &Dataflow,
    model: &ModuleModel<'_>,
) -> Vec<JumpTable> {
    let mut tables = Vec::new();
    if !facts.converged {
        return tables;
    }
    for block in &cfg.blocks {
        let site = block.end;
        let Some(branch) = code.get(&site) else {
            continue;
        };
        if branch.opcode != "bctr"
            || branch.control_flow
                != (Flow::Indirect {
                    link: false,
                    conditional: false,
                })
        {
            continue;
        }
        let Some(mtpc) = definition(facts, site, Register::Ctr) else {
            continue;
        };
        let Some(mt) = code.get(&mtpc).filter(|d| d.opcode == "mtspr") else {
            continue;
        };
        let source = Register::Gpr(((mt.raw >> 21) & 31) as u8);
        let Some(loadpc) = definition(facts, mtpc, source) else {
            continue;
        };
        let Some(load) = code
            .get(&loadpc)
            .filter(|d| d.opcode == "lwzx" && d.status == "KNOWN")
        else {
            continue;
        };
        if !block.instructions.contains(&mtpc)
            || !block.instructions.contains(&loadpc)
            || loadpc >= mtpc
            || mtpc >= site
        {
            continue;
        }
        let base = Register::Gpr(((load.raw >> 16) & 31) as u8);
        if base == Register::Gpr(0) {
            continue;
        }
        let address = match facts.used(loadpc, base).map(|f| &f.value) {
            Some(Value::Known(Atom::Constant(n))) => Some(*n),
            Some(Value::Known(Atom::Target(t))) => model.address(t),
            _ => None,
        };
        let Some(address) = address else {
            continue;
        };
        let Some(Target::Address { section, .. }) = model.logical_address(address) else {
            continue;
        };
        let mut table = JumpTable {
            site,
            address,
            section,
            status: "CANDIDATE",
            bound: None,
            default_target: None,
            entries: Vec::new(),
            evidence: Vec::new(),
        };
        add_evidence(
            &mut table,
            "INDEXED_LOAD",
            Some(loadpc),
            "A propagated base address reaches lwzx, mtctr and bctr in one block",
        );
        let index = Register::Gpr(((load.raw >> 11) & 31) as u8);
        let scale = definition(facts, loadpc, index)
            .and_then(|pc| code.get(&pc))
            .filter(|d| {
                d.opcode == "rlwinm"
                    && d.status == "KNOWN"
                    && (d.raw >> 11) & 31 == 2
                    && (d.raw >> 6) & 31 == 0
                    && (d.raw >> 1) & 31 == 29
                    && block.instructions.contains(&d.address)
                    && d.address < loadpc
            });
        let incoming: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.to == Some(block.start))
            .collect();
        let guard = if incoming.len() == 1 && incoming[0].kind == "ConditionalFalse" {
            cfg.blocks.iter().find(|b| b.start == incoming[0].from)
        } else {
            None
        };
        let bound = scale.zip(guard).and_then(|(scale, guard)| {
            // An entry block also has an implicit external predecessor.
            if entry == block.start || guard.instructions.len() < 2 {
                return None;
            }
            let cmp = code.get(&guard.instructions[guard.instructions.len() - 2])?;
            let bc = code.get(&guard.end)?;
            let source_index = Register::Gpr(((scale.raw >> 21) & 31) as u8);
            if cmp.opcode != "cmpli"
                || cmp.status != "KNOWN"
                || Register::Gpr(((cmp.raw >> 16) & 31) as u8) != source_index
                || bc.opcode != "bc"
                || bc.status != "KNOWN"
                || (bc.raw >> 21) & 30 != 12
                || (bc.raw >> 16) & 31 != ((cmp.raw >> 23) & 7) * 4 + 1
            {
                return None;
            }
            let Flow::Branch {
                target,
                conditional: true,
            } = bc.control_flow
            else {
                return None;
            };
            if facts.used(cmp.address, source_index)?.definitions
                != facts.used(scale.address, source_index)?.definitions
            {
                return None;
            }
            if block.instructions.iter().take_while(|pc| **pc < scale.address).any(|pc| {
                code.get(pc).is_none_or(|d| !matches!(&d.semantics, Semantics::Known { defs, .. } if !defs.contains(&source_index)))
            }) {
                return None;
            }
            Some((cmp.raw & 0xffff, cmp.address, target))
        });
        let Some((maximum, cmp, default)) = bound else {
            add_evidence(
                &mut table,
                "UNKNOWN_BOUND",
                None,
                "No supported dominating unsigned compare/default guard; no entries read",
            );
            tables.push(table);
            continue;
        };
        let count = maximum + 1;
        table.bound = Some(count);
        table.default_target = Some(default);
        add_evidence(
            &mut table,
            "UNSIGNED_BOUND",
            Some(cmp),
            "cmplwi followed by bgt default bounds an unchanged index; rlwinm scales it by four",
        );
        if count > 4096 {
            table.status = "PARTIAL";
            add_evidence(
                &mut table,
                "BUDGET",
                None,
                "Proven bound exceeds 4096 entries; no entries read",
            );
            tables.push(table);
            continue;
        }
        let memory_tainted = facts
            .inputs
            .get(&block.start)
            .is_none_or(|s| s.memory_tainted)
            || block
                .instructions
                .iter()
                .take_while(|pc| **pc < loadpc)
                .any(|pc| {
                    code.get(pc).is_none_or(|d| {
                        !matches!(
                            &d.semantics,
                            Semantics::Known {
                                memory_write: false,
                                ..
                            }
                        )
                    })
                });
        let section_data = &model.module.sections[section as usize];
        if memory_tainted || section_data.kind != 1 || section_data.flags & 1 != 0 {
            table.status = "PARTIAL";
            add_evidence(
                &mut table,
                "MUTABILITY",
                None,
                "Table memory not proven immutable at this load; no entries read",
            );
            tables.push(table);
            continue;
        }
        let offset = (address - section_data.address) as usize;
        if address & 3 != 0
            || offset
                .checked_add(count as usize * 4)
                .is_none_or(|end| end > section_data.data.len() || end > section_data.size as usize)
        {
            table.status = "REJECTED";
            add_evidence(
                &mut table,
                "BOUNDS",
                None,
                "Proven table range is misaligned or truncated",
            );
            tables.push(table);
            continue;
        }
        table.status = "VERIFIED";
        for index in 0..count {
            let value = model.read_word(address + index * 4);
            let target = match &value {
                Value::Known(Atom::Constant(n)) => Some(*n),
                Value::Known(Atom::Target(t)) => model.address(t),
                _ => None,
            };
            let Some(target) = target.filter(|t| *t & 3 == 0 && model.module.word(*t).is_some())
            else {
                table.status = "REJECTED";
                table.entries.clear();
                add_evidence(
                    &mut table,
                    "INVALID_ENTRY",
                    Some(address + index * 4),
                    "Entry is unresolved, misaligned, or outside file-backed executable code",
                );
                break;
            };
            table.entries.push((index, target));
        }
        if table.status == "VERIFIED" {
            add_evidence(&mut table,"VALIDATED_ENTRIES",Some(address),"Every bounded big-endian word resolves to aligned executable code; duplicate targets retain their case indices");
        }
        tables.push(table);
    }
    tables
}
