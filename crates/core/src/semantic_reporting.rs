use crate::semantic_interprocedural::Report;
use serde_json::{json, Value};
pub fn metrics(r: &Report, x: &Value) -> Value {
    let len = |key: &str| x[key].as_array().map_or(0, Vec::len);
    let summaries: Vec<_> = r.summaries.values().collect();
    json!({
        "functions_analyzed":summaries.len(),
        "functions_with_semantic_facts":summaries.iter().filter(|s|!s.candidate_arguments.is_empty() || !s.accesses.is_empty() || !s.candidate_return.alternatives.is_empty()).count(),
        "interprocedural_facts":r.constraints.len(),
        "type_conflicts":x["type_constraints"].as_array().into_iter().flatten().filter(|t|t["conflicting"]==true).count(),
        "functions_with_candidate_signatures":summaries.iter().filter(|s|!s.candidate_arguments.is_empty() || !s.candidate_return.alternatives.is_empty()).count(),
        "candidate_arguments":summaries.iter().map(|s|s.candidate_arguments.len()).sum::<usize>(),
        "typed_candidate_arguments":len("type_constraints"),
        "candidate_return_values":summaries.iter().filter(|s|!s.candidate_return.alternatives.is_empty()).count(),
        "typed_candidate_returns":x["return_candidates"].as_array().into_iter().flatten().filter(|s|s["candidate_types"].as_array().into_iter().flatten().any(|t|t!="Unknown")).count(),
        "value_provenance_nodes":r.nodes.len(),
        "value_provenance_edges":r.nodes.iter().map(|n|n.inputs.len()).sum::<usize>(),
        "global_storage_candidates":len("global_storage"),
        "mutable_storage_candidates":x["global_storage"].as_array().into_iter().flatten().filter(|s|s["mutable"]==true).count(),
        "reaching_definition_reads":summaries.iter().flat_map(|s|&s.accesses).filter(|a|!a.write).count(),
        "definitely_one_reads":summaries.iter().flat_map(|s|&s.accesses).filter(|a|!a.write && a.reaching_status=="DEFINITELY_ONE").count(),
        "finite_set_reads":summaries.iter().flat_map(|s|&s.accesses).filter(|a|!a.write && a.reaching_status=="FINITE_SET").count(),
        "function_pointer_candidates":len("function_pointers"),
        "callback_candidates":len("callback_candidates"),
        "table_candidates":x["tables"].as_array().into_iter().flatten().filter(|t|t["entries"].as_array().is_some_and(|e|e.len()>=2)).count(),
        "storage_runs":len("tables"),
        "object_candidates":len("objects"),"aggregate_candidates":len("aggregates"),
        "field_candidates":x["aggregates"].as_array().into_iter().flatten().map(|a|a["fields"].as_array().map_or(0,Vec::len)).sum::<usize>(),
        "vtable_candidates":len("vtables"),"class_candidates":len("classes"),"virtual_call_candidates":len("virtual_calls"),
        "machine_proven_indirect_candidates":summaries.iter().flat_map(|s|&s.indirects).filter(|s|s.classification=="MACHINE_PROVEN_SINGLE_TARGET").count(),
        "semantic_operations":summaries.iter().map(|s|s.accesses.len()+s.calls.len()).sum::<usize>()+len("virtual_calls")+len("callback_candidates"),
        "typed_ir_operations_examined":r.nodes.iter().filter_map(|n|n.operation.map(|op|(&n.function,n.block,op))).collect::<std::collections::BTreeSet<_>>().len(),
        "incomplete_functions":summaries.iter().filter(|s|!s.incomplete.is_empty()).count(),
        "constructor_destructor_candidates":"NOT_IMPLEMENTED"
    })
}
