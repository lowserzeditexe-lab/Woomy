use crate::{
    semantic_interprocedural::{Indirect, Report},
    semantic_memory::Alias,
    semantic_values::Location,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn metrics(r: &Report) -> Value {
    let calls: Vec<_> = r.summaries.values().flat_map(|s| &s.call_effects).collect();
    let effects: Vec<_> = r.memory_effects.values().flat_map(|s| &s.effects).collect();
    let queries: Vec<_> = calls.iter().flat_map(|c| &c.aliases).collect();
    json!({"functions_analyzed":r.summaries.len(),"complete_memory_summaries":r.memory_effects.values().filter(|m|m.complete).count(),"incomplete_memory_summaries":r.memory_effects.values().filter(|m|!m.complete).count(),
        "local_reads_examined":r.summaries.values().flat_map(|s|&s.accesses).filter(|a|!a.write).count(),"local_writes_examined":r.summaries.values().flat_map(|s|&s.accesses).filter(|a|a.write).count(),
        "must_effects_on_normal_return":effects.iter().filter(|e|e.certainty.starts_with("MUST_ON_NORMAL_RETURN")).count(),"may_effects":effects.iter().filter(|e|e.certainty.starts_with("MAY")).count(),"parametric_reads":effects.iter().filter(|e|!e.write).count(),"parametric_writes":effects.iter().filter(|e|e.write).count(),"exact_global_effects":effects.iter().filter(|e|matches!(e.location,Some(Location::Global(_)))).count(),"incoming_register_relative_effects":effects.iter().filter(|e|matches!(e.location,Some(Location::Symbolic{..}))).count(),"unknown_address_effects":effects.iter().filter(|e|e.location.is_none()).count(),"unknown_effect_reasons":r.memory_effects.values().map(|m|m.reasons.len()).sum::<usize>(),"effect_budget_exhausted_functions":r.memory_effects.values().filter(|m|m.exhausted).count(),
        "alias_queries":queries.len(),"must_alias":queries.iter().filter(|q|q.relationship==Alias::MustAlias).count(),"may_alias":queries.iter().filter(|q|q.relationship==Alias::MayAlias).count(),"proven_disjoint":queries.iter().filter(|q|q.relationship==Alias::ProvenDisjoint).count(),"unknown_alias":queries.iter().filter(|q|q.relationship==Alias::Unknown).count(),
        "direct_calls_with_composed_effects":calls.iter().filter(|c|c.callee.is_some() && !c.effects.is_empty()).count(),"calls_with_complete_effects":calls.iter().filter(|c|c.complete).count(),"calls_with_incomplete_effects":calls.iter().filter(|c|!c.complete).count(),"unknown_import_effects":calls.iter().filter(|c|c.classification=="EFFECT_UNKNOWN_IMPORT").count(),"unknown_indirect_effects":calls.iter().filter(|c|c.classification=="EFFECT_UNKNOWN_INDIRECT").count(),"facts_preserved_across_calls":calls.iter().map(|c|c.preserved).sum::<usize>(),"facts_invalidated_by_calls":calls.iter().map(|c|c.invalidated).sum::<usize>()})
}

pub fn audit(r: &Report, site: &Indirect) -> Value {
    let nodes: BTreeMap<_, _> = r.nodes.iter().map(|n| (&n.id, n)).collect();
    let mut seen = BTreeSet::new();
    let mut pending = site.value.definitions.iter().cloned().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if seen.len() > 4096 {
            return json!({"status":"AUDIT_PROVENANCE_BUDGET_EXHAUSTED","site":site.node});
        }
        if let Some(n) = nodes.get(&id) {
            pending.extend(n.inputs.iter().cloned());
        }
    }
    let mut calls = vec![];
    let mut blocking_calls = vec![];
    for s in r.summaries.values() {
        for call in &s.calls {
            if !seen.contains(&call.node) {
                continue;
            }
            let n = nodes.get(&call.node);
            let effect = s.call_effects.iter().find(|e| e.node == call.node);
            let causes: BTreeSet<_> = seen
                .iter()
                .filter_map(|id| nodes.get(id))
                .filter(|n| n.value.definitions.contains(&call.node))
                .flat_map(|n| {
                    n.value
                        .unknown
                        .iter()
                        .filter(|r| r.contains("CALL"))
                        .cloned()
                })
                .collect();
            if !causes.is_empty() {
                let classification = if causes.contains("CALL_REGISTER_WRITE") {
                    "EFFECT_REGISTER_CLOBBER"
                } else if causes.contains("UNKNOWN_CALLEE_RETURN") {
                    "EFFECT_RETURN_VALUE_UNKNOWN"
                } else {
                    effect
                        .map(|e| e.classification.as_str())
                        .unwrap_or("EFFECT_PROVENANCE_LOST")
                };
                blocking_calls.push(json!({"node":call.node,"function":s.function,"pc":n.map(|n|n.pc),"classification":classification,"reasons":causes,"target":call.target,"callee":call.callee,"written_or_clobbered_registers":call.callee.as_ref().and_then(|c|r.summaries.get(c)).map(|s|&s.possibly_modified)}));
            }
            calls.push(json!({"node":call.node,"function":s.function,"pc":n.map(|n|n.pc),"target":call.target,"callee":call.callee,"observed_arguments":call.arguments,"incoming_register_values_not_abi_arguments":call.inputs,"memory_effect":effect,"register_effect":"Use current machine summary; memory preservation alone does not preserve written registers","callee_uncertainties":call.callee.as_ref().and_then(|c|r.memory_effects.get(c)).map(|s|&s.reasons)}));
        }
    }
    let losses:Vec<_>=seen.iter().filter_map(|id|nodes.get(id)).filter(|n|!n.value.unknown.is_empty()).map(|n|json!({"node":n.id,"function":n.function,"block":n.block,"pc":n.pc,"operation":n.operation,"kind":n.kind,"unknown":n.value.unknown,"inputs":n.inputs})).collect();
    json!({"status":"AUDITED","site":site.node,"root_chain":seen,"calls":calls,"blocking_calls":blocking_calls,"losses":losses,"target_unknown":site.value.unknown,"scope":"Static provenance dependencies, not a fabricated dynamic path"})
}

pub fn storage_audit(r: &Report, address: u32, width: u8) -> Value {
    let location = Location::Global(address);
    let mut writes = vec![];
    let mut calls = vec![];
    for s in r.summaries.values() {
        for a in &s.accesses {
            if a.write {
                let relation = crate::semantic_memory::alias(
                    Some(&location),
                    width,
                    a.location.as_ref(),
                    a.width,
                );
                if relation != Alias::ProvenDisjoint {
                    writes.push(json!({"node":a.node,"function":s.function,"address":a.address,"width":a.width,"relationship":relation,"value":a.value}));
                }
            }
        }
        for c in &s.call_effects {
            let aliases:Vec<_>=c.effects.iter().filter(|e|e.write).filter_map(|e| {
                let relationship=crate::semantic_memory::alias(Some(&location),width,e.location.as_ref(),e.width);
                (relationship!=Alias::ProvenDisjoint).then(||json!({"source":e.source,"location":e.location,"width":e.width,"relationship":relationship,"via_calls":e.via_calls}))
            }).collect();
            if !c.complete || !aliases.is_empty() {
                calls.push(json!({"node":c.node,"function":s.function,"callee":c.callee,"classification":c.classification,"complete":c.complete,"aliases":aliases}));
            }
        }
    }
    json!({"address":address,"width":width,"possible_writes":writes,"possible_call_clobbers":calls,"scope":"May alias if reached; no claim that these functions execute before a particular read"})
}
