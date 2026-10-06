//! Semantic grouping, type constraints, and conservative object/table evidence.
//! These hypotheses are deliberately not inputs to the machine proof gate.
use crate::{semantic_interprocedural::Report, semantic_values::*, Error, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{
    runtime::{GuestMemory, EXECUTE},
    Function,
};
use woomy_ppc::Register;

fn subject(function: &str, r: Register) -> String {
    format!("{function}:input:{r:?}")
}
fn pointer_targets(f: &Fact, m: &GuestMemory, entries: &BTreeMap<u32, String>) -> Vec<u32> {
    f.alternatives
        .iter()
        .filter_map(|a| match a {
            Atom::Bits {
                bits,
                address_source: Some(origin),
            } if bits == origin
                && bits & 3 == 0
                && entries.contains_key(bits)
                && m.locate(*bits, 4, EXECUTE).is_some() =>
            {
                Some(*bits)
            }
            _ => None,
        })
        .collect()
}
/// Evidence roots are sets: circulating a constraint never manufactures another witness.
pub fn reconstruct(
    report: &Report,
    functions: &[Function],
    memory: &GuestMemory,
    tables: &[crate::semantic_tables::TableCandidate],
) -> Result<Value> {
    let entries: BTreeMap<_, _> = functions.iter().map(|f| (f.entry, f.id.clone())).collect();
    let mut types = BTreeMap::<String, BTreeMap<String, BTreeSet<String>>>::new();
    let mut links = BTreeSet::<(String, String)>::new();
    let mut argument_values = BTreeMap::<String, Fact>::new();
    for f in functions {
        let local = crate::semantic::analyze(f)?;
        for ty in &local.typed_arguments {
            let key = format!(
                "{}:input:{}",
                f.id,
                ty.subject.trim_start_matches("incoming.")
            );
            for alt in &ty.alternatives {
                let kind = match alt {
                    crate::semantic::SemanticType::GuestPointer { .. } => "GuestPointer",
                    crate::semantic::SemanticType::SignedInteger => "SignedInteger",
                    crate::semantic::SemanticType::UnsignedInteger => "UnsignedInteger",
                    _ => "Unknown",
                };
                types
                    .entry(key.clone())
                    .or_default()
                    .entry(kind.into())
                    .or_default()
                    .extend(
                        ty.candidate
                            .evidence
                            .iter()
                            .map(|id| format!("{}:local_evidence:{id}", f.id)),
                    );
            }
        }
    }
    for c in &report.constraints {
        let dst = subject(&c.callee, c.register);
        argument_values
            .entry(dst.clone())
            .or_default()
            .merge(&c.value);
        for a in &c.value.alternatives {
            match a {
                Atom::Incoming {
                    function,
                    register,
                    offset: 0,
                } => {
                    let src = subject(function, *register);
                    links.insert((src.clone(), dst.clone()));
                    links.insert((dst.clone(), src));
                }
                Atom::Bits {
                    bits,
                    address_source,
                } => {
                    let kind =
                        if address_source == &Some(*bits) && memory.locate(*bits, 1, 0).is_some() {
                            if entries.contains_key(bits) {
                                "FunctionPointer"
                            } else {
                                "GuestPointer"
                            }
                        } else {
                            "Integer"
                        };
                    types
                        .entry(dst.clone())
                        .or_default()
                        .entry(kind.into())
                        .or_default()
                        .insert(c.site.clone());
                }
                _ => {}
            }
        }
    }
    let mut converged = false;
    let mut iterations = 0;
    for n in 0..128 {
        iterations = n + 1;
        let previous = types.clone();
        for (src, dst) in &links {
            if let Some(ts) = previous.get(src) {
                for (kind, roots) in ts {
                    types
                        .entry(dst.clone())
                        .or_default()
                        .entry(kind.clone())
                        .or_default()
                        .extend(roots.iter().cloned());
                }
            }
        }
        if types
            .values()
            .flat_map(|x| x.values())
            .map(|x| x.len())
            .sum::<usize>()
            > 262144
        {
            return Err(Error::Analysis(
                "SEMANTIC_TYPE_ORIGIN_BUDGET_EXHAUSTED".into(),
            ));
        }
        if types == previous {
            converged = true;
            break;
        }
    }
    let mut fields = BTreeMap::<String, Vec<Value>>::new();
    let mut pointers = vec![];
    let mut globals = BTreeMap::<u32, Vec<Value>>::new();
    for (fid, s) in &report.summaries {
        for a in &s.accesses {
            if let Some(location) = &a.location {
                match location {
                    Location::Global(address) => {
                        globals.entry(*address).or_default().push(json!({"function":fid,"node":a.node,"write":a.write,"width":a.width,"value":a.value,"reaching":a.reaching_status}));
                    }
                    Location::Symbolic {
                        function,
                        register,
                        offset,
                    } if *register != Register::Gpr(1) => {
                        let key = subject(function, *register);
                        fields.entry(key).or_default().push(json!({"function":fid,"node":a.node,"offset":offset,"width":a.width,"write":a.write,"field_type":"UNKNOWN","lifetime":"UNKNOWN"}));
                    }
                    _ => {}
                }
            }
            let targets = pointer_targets(&a.value, memory, &entries);
            if !targets.is_empty() {
                pointers.push(json!({"node":a.node,"storage":a.location,"targets":targets,"status":"HYPOTHESIS","unknown":a.value.unknown,"operation":if a.write{"FunctionPointerStore"}else{"FunctionPointerLoad"},"evidence":a.value.definitions}));
            }
        }
        for site in &s.indirects {
            let targets = pointer_targets(&site.value, memory, &entries);
            if !targets.is_empty() {
                pointers.push(json!({"node":site.node,"targets":targets,"status":"HYPOTHESIS","unknown":site.value.unknown,"operation":"IndirectCall","evidence":site.value.definitions}));
            }
        }
    }
    // Instantiate a callee's fields at an exactly identified caller pointer.
    // Multiple origins stay separate; equal offsets alone create no equivalence.
    let mut aggregate_converged = false;
    for _ in 0..128 {
        let previous = fields.clone();
        for c in &report.constraints {
            if c.value.unknown.is_empty() && c.value.alternatives.len() == 1 {
                if let Some(Atom::Incoming {
                    function,
                    register,
                    offset: base_offset,
                }) = c.value.alternatives.first()
                {
                    if let Some(fs) = previous.get(&subject(&c.callee, c.register)) {
                        let dest = fields.entry(subject(function, *register)).or_default();
                        for source in fs {
                            let mut field = source.clone();
                            let source_offset = source["offset"].as_i64().unwrap_or(0) as i32;
                            field["offset"] = json!(source_offset.wrapping_add(*base_offset));
                            field["via_call"] = json!(c.site);
                            let mut roots: BTreeSet<String> = field
                                .get("via_calls")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect();
                            roots.insert(c.site.clone());
                            if let Some(existing) = dest.iter_mut().find(|f| {
                                f["node"] == field["node"] && f["offset"] == field["offset"]
                            }) {
                                roots.extend(
                                    existing
                                        .get("via_calls")
                                        .and_then(Value::as_array)
                                        .into_iter()
                                        .flatten()
                                        .filter_map(Value::as_str)
                                        .map(str::to_owned),
                                );
                                existing["via_calls"] = json!(roots);
                            } else {
                                field["via_calls"] = json!(roots);
                                dest.push(field);
                            }
                        }
                    }
                }
            }
        }
        if fields.values().map(Vec::len).sum::<usize>() > 65536 {
            return Err(Error::Analysis(
                "AGGREGATE_PROPAGATION_BUDGET_EXHAUSTED".into(),
            ));
        }
        if fields == previous {
            aggregate_converged = true;
            break;
        }
    }
    let aggregates:Vec<_>=fields.into_iter().filter(|(_,v)|v.iter().filter_map(|f|f["offset"].as_i64()).collect::<BTreeSet<_>>().len()>=2).map(|(origin,fields)|json!({"id":format!("aggregate_{}",&crate::hash_bytes(origin.as_bytes())[..16]),"pointer_origin":origin,"fields":fields,"source_identity":"UNKNOWN","status":"HYPOTHESIS"})).collect();
    let mut table_values = vec![];
    for table in tables {
        for entry in &table.entries {
            globals.entry(entry.evidence.address).or_default();
        }
    }
    for t in tables {
        let mutable = t.entries.iter().any(|e| !e.immutable_storage);
        table_values.push(json!({"id":t.id,"address":t.address,"bytes":t.byte_length,"kind":if t.entries.len()==1 {"POINTER_STORAGE_WORD"}else if mutable{"MUTABLE_TABLE"}else{"IMMUTABLE_TABLE"},"status":"HYPOTHESIS_STORAGE_RUN","entries":t.entries,"accesses":globals.range(t.address..t.address.saturating_add(t.byte_length)).flat_map(|(_,a)|a.iter()).collect::<Vec<_>>(),"boundary_proof":"UNKNOWN","runtime_values":"SEE_POINT_SPECIFIC_REACHING_DEFINITIONS"}));
    }
    // Strong seed: an explicit object-field-zero store of a known table address,
    // plus executable entries and an actual indirect consumer of that table load.
    let mut vtables = vec![];
    let mut objects = vec![];
    let mut classes = vec![];
    let mut virtual_calls = vec![];
    let node_map: BTreeMap<_, _> = report.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    for s in report.summaries.values() {
        for init in s.accesses.iter().filter(|a| a.write && a.width == 4) {
            let Some(Location::Symbolic {
                function,
                register,
                offset: 0,
            }) = &init.location
            else {
                continue;
            };
            let Some(Location::Global(table_address)) = init.value.address() else {
                continue;
            };
            let Some(table) = tables.iter().find(|t| {
                t.address == table_address
                    && t.entries.len() >= 2
                    && t.entries
                        .iter()
                        .filter(|e| e.target_function.is_some() && e.evidence.applied_local_addr32)
                        .count()
                        >= 2
            }) else {
                continue;
            };
            if !init
                .value
                .alternatives
                .iter()
                .all(|a| matches!(a,Atom::Bits {bits,address_source:Some(o)} if bits==o))
            {
                continue;
            }
            let object_id = format!(
                "object_{}",
                &crate::hash_bytes(subject(function, *register).as_bytes())[..16]
            );
            let mut uses = vec![];
            for site in &s.indirects {
                let mut seen = BTreeSet::new();
                let mut pending = site.value.definitions.iter().cloned().collect::<Vec<_>>();
                while let Some(id) = pending.pop() {
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    if seen.len() > 4096 {
                        return Err(Error::Analysis("VTABLE_PROVENANCE_BUDGET_EXHAUSTED".into()));
                    }
                    if let Some(n) = node_map.get(id.as_str()) {
                        pending.extend(n.inputs.iter().cloned());
                    }
                }
                if !seen.contains(&init.node) {
                    continue;
                }
                for load in s
                    .accesses
                    .iter()
                    .filter(|a| !a.write && seen.contains(&a.node))
                {
                    let Some(Location::Global(address)) = load.location else {
                        continue;
                    };
                    if address >= table.address
                        && address < table.address.saturating_add(table.byte_length)
                        && (address - table.address) % 4 == 0
                        && !pointer_targets(&load.value, memory, &entries).is_empty()
                    {
                        let v = json!({"operation":"VirtualCallCandidate","node":site.node,"object":object_id,"vtable":table.id,"slot_offset":address-table.address,"targets":pointer_targets(&load.value,memory,&entries),"evidence_chain":seen,"unknown_alternatives":site.value.unknown,"status":"HYPOTHESIS"});
                        uses.push(v.clone());
                        virtual_calls.push(v);
                    }
                }
            }
            if uses.is_empty() {
                continue;
            }
            objects.push(json!({"id":object_id,"pointer_origin":subject(function,*register),"initialization":init.node,"source_identity":"UNKNOWN"}));
            vtables.push(json!({"id":table.id,"address":table.address,"object":object_id,"initialization":init.node,"entries":table.entries,"uses":uses,"status":"HYPOTHESIS"}));
            classes.push(json!({"id":format!("class_candidate_{}",&crate::hash_bytes(table.id.as_bytes())[..16]),"object":object_id,"vtable":table.id,"methods":table.entries.iter().filter_map(|e|e.target_function.as_ref()).collect::<Vec<_>>(),"source_name":"UNKNOWN","inheritance":"UNKNOWN"}));
        }
    }
    if aggregates.len() + objects.len() + vtables.len() > 16384 {
        return Err(Error::Analysis("SEMANTIC_OBJECT_BUDGET_EXHAUSTED".into()));
    }
    let mut callbacks = vec![];
    for c in &report.constraints {
        let targets = pointer_targets(&c.value, memory, &entries);
        if targets.is_empty() {
            continue;
        }
        let Some(callee) = report.summaries.get(&c.callee) else {
            continue;
        };
        let matches_input = |value: &Fact| {
            value.alternatives.iter().any(|a|matches!(a,Atom::Incoming {function,register,offset:0} if function==&c.callee && register==&c.register))
        };
        let stores: Vec<_> = callee
            .accesses
            .iter()
            .filter(|a| a.write && matches_input(&a.value))
            .map(|a| &a.node)
            .collect();
        let invocations: Vec<_> = callee
            .indirects
            .iter()
            .filter(|s| matches_input(&s.value))
            .map(|s| &s.node)
            .collect();
        if !stores.is_empty() && !invocations.is_empty() {
            callbacks.push(json!({"operation":"CallbackRegistrationCandidate","registration":c.site,"callee":c.callee,"register":c.register,"targets":targets,"stores":stores,"invocations":invocations,"lifetime":"UNKNOWN","status":"HYPOTHESIS","unknown_alternatives":c.value.unknown}));
        }
    }
    let returns:Vec<_>=report.summaries.values().map(|s|{
        let targets=pointer_targets(&s.candidate_return,memory,&entries);
        let kinds:BTreeSet<_>=s.candidate_return.alternatives.iter().map(|a|match a {Atom::Bits {bits,address_source:Some(o)} if bits==o && memory.locate(*bits,1,0).is_some()=>if targets.contains(bits){"FunctionPointer"}else{"GuestPointer"},Atom::Bits {..}=>"Integer",_=>"Unknown"}).collect();
        json!({"function":s.function,"candidate_value":s.candidate_return,"candidate_types":kinds,"source_return_type":"UNKNOWN","status":"HYPOTHESIS"})
    }).collect();
    let typed:Vec<_>=types.into_iter().map(|(subject,alternatives)|json!({"subject":subject,"conflicting":alternatives.len()>1,"alternatives":alternatives,"status":if converged{"HYPOTHESIS"}else{"INCOMPLETE_TYPE_FIXED_POINT"}})).collect();
    Ok(
        json!({"type_constraints":typed,"type_iterations":iterations,"type_converged":converged,"aggregate_converged":aggregate_converged,"argument_values":argument_values,"return_candidates":returns,"aggregates":aggregates,"global_storage":globals.into_iter().map(|(address,accesses)|json!({"address":address,"mutable":memory.locate(address,1,0).is_some_and(|(r,_)|memory.regions[r].permissions & woomy_ir::runtime::WRITE!=0),"accesses":accesses,"entry_value":"UNKNOWN_RUNTIME_ENTRY","relocation_initializations":tables.iter().flat_map(|t|&t.entries).filter(|e|e.evidence.address==address).collect::<Vec<_>>()})).collect::<Vec<_>>(),"function_pointers":pointers,"tables":table_values,"objects":objects,"vtables":vtables,"classes":classes,"virtual_calls":virtual_calls,"callback_candidates":callbacks,"callback_status":"ANALYZED_EXPLICIT_PASS_STORE_INVOKE_CHAINS","constructor_destructor_status":"NOT_IMPLEMENTED"}),
    )
}
