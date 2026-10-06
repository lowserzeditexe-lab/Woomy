//! Machine-derived may-effects. Different incoming registers never prove disjointness.
use crate::semantic_interprocedural::{substitute, Call, Node, Summary};
use crate::semantic_values::{Fact, Location, Storage, MAX_ORIGINS};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{CallTarget, ValueId};

pub const MEMORY_EFFECT_VERSION: u32 = 1;
pub const ALIAS_VERSION: u32 = 1;
pub const EFFECT_POLICY_VERSION: u32 = 1;
const MAX_EFFECTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Alias {
    MustAlias,
    MayAlias,
    ProvenDisjoint,
    Unknown,
}
pub fn alias(a: Option<&Location>, aw: u8, b: Option<&Location>, bw: u8) -> Alias {
    let (Some(a), Some(b)) = (a, b) else {
        return Alias::Unknown;
    };
    if aw == 0 || bw == 0 {
        return Alias::Unknown;
    }
    if matches!(a,Location::Global(x) if *x as u64+aw as u64 > 1u64<<32)
        || matches!(b,Location::Global(x) if *x as u64+bw as u64 > 1u64<<32)
    {
        return Alias::Unknown;
    }
    if a == b && aw == bw {
        return Alias::MustAlias;
    }
    if !a.overlaps(aw, b, bw) {
        Alias::ProvenDisjoint
    } else {
        Alias::MayAlias
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Effect {
    pub function: String,
    pub block: u32,
    pub pc: u32,
    pub operation: Option<ValueId>,
    pub source: String,
    pub location: Option<Location>,
    pub width: u8,
    pub write: bool,
    /// Conditional execution and faults before this operation are not excluded.
    pub certainty: String,
    pub provenance: BTreeSet<String>,
    /// Set of composition witnesses, not an invented ordered dynamic call stack.
    pub via_calls: BTreeSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemorySummary {
    pub version: u32,
    /// All effects accounted for, not all addresses precise. An unknown read
    /// cannot imply a write; unknown_writes still forbids preservation.
    pub complete: bool,
    pub effects: Vec<Effect>,
    pub unknown_reads: bool,
    pub unknown_writes: bool,
    pub reasons: BTreeMap<String, String>,
    pub escaping_values: BTreeMap<String, Fact>,
    pub summary_sources: BTreeMap<String, String>,
    pub exhausted: bool,
}
impl Default for MemorySummary {
    fn default() -> Self {
        Self {
            version: MEMORY_EFFECT_VERSION,
            complete: true,
            effects: vec![],
            unknown_reads: false,
            unknown_writes: false,
            reasons: BTreeMap::new(),
            escaping_values: BTreeMap::new(),
            summary_sources: BTreeMap::new(),
            exhausted: false,
        }
    }
}
impl MemorySummary {
    fn unknown(&mut self, site: &str, reason: &str) {
        self.complete = false;
        self.unknown_reads = true;
        self.unknown_writes = true;
        self.reasons.insert(site.into(), reason.into());
    }
    fn widen(&mut self) {
        self.effects.clear();
        self.escaping_values.clear();
        self.reasons.clear();
        self.unknown("summary", "MEMORY_EFFECT_BUDGET_EXHAUSTED");
        self.exhausted = true;
    }
    fn insert(&mut self, e: Effect) {
        if self.exhausted {
            return;
        }
        if e.provenance.len() > MAX_ORIGINS || e.via_calls.len() > MAX_ORIGINS {
            self.widen();
            return;
        }
        if let Some(old) = self.effects.iter_mut().find(|old| {
            old.source == e.source
                && old.location == e.location
                && old.width == e.width
                && old.write == e.write
        }) {
            old.provenance.extend(e.provenance);
            old.via_calls.extend(e.via_calls);
            if old.provenance.len() > MAX_ORIGINS || old.via_calls.len() > MAX_ORIGINS {
                self.widen();
            }
        } else {
            self.effects.push(e);
        }
        if self.effects.len() > MAX_EFFECTS {
            self.widen();
        }
    }
}
pub fn instantiate(e: &Effect, callee: &str, call: &Call) -> Effect {
    let mut e = e.clone();
    if let Some(Location::Symbolic {
        function,
        register,
        offset,
    }) = &e.location
    {
        if function == callee {
            let regs = call.inputs.iter().cloned().collect();
            let value = substitute(
                &Fact::atom(crate::semantic_values::Atom::Incoming {
                    function: function.clone(),
                    register: *register,
                    offset: *offset,
                }),
                callee,
                &regs,
            );
            e.location = value.address();
            e.provenance.extend(value.definitions);
        }
    }
    e.certainty = "MAY_EXECUTE; COMPOSED_CALL_PATH".into();
    e.via_calls.insert(call.node.clone());
    e
}

/// Least may-effect closure over known direct calls. Recursive expansion is
/// union-only, with an absorbing unknown on budget exhaustion.
pub fn summarize(
    summaries: &BTreeMap<String, Summary>,
    nodes: &BTreeMap<String, Node>,
    functions: &[&woomy_ir::Function],
) -> BTreeMap<String, MemorySummary> {
    let mut local = BTreeMap::new();
    for (fid, s) in summaries {
        let mut m = MemorySummary::default();
        for reason in &s.incomplete {
            m.unknown(fid, reason);
        }
        for access in &s.accesses {
            let n = &nodes[&access.node];
            if access.location.is_none() {
                if access.write {
                    m.unknown_writes = true;
                } else {
                    m.unknown_reads = true;
                }
                m.reasons.insert(
                    access.node.clone(),
                    if access.write {
                        "UNKNOWN_WRITE_ADDRESS"
                    } else {
                        "UNKNOWN_READ_ADDRESS"
                    }
                    .into(),
                );
            }
            let mut provenance = access.address.definitions.clone();
            provenance.insert(access.node.clone());
            m.insert(Effect {
                function: fid.clone(),
                block: n.block,
                pc: n.pc,
                operation: n.operation,
                source: access.node.clone(),
                location: access.location.clone(),
                width: access.width,
                write: access.write,
                certainty: if functions.iter().any(|f| f.id == *fid && f.entry == n.block) {
                    "MUST_ON_NORMAL_RETURN; FAULTS_EXCLUDED"
                } else {
                    "MAY_EXECUTE; CONDITIONAL_PATH"
                }
                .into(),
                provenance,
                via_calls: BTreeSet::new(),
            });
            if access.write
                && access.value.alternatives.iter().any(|a| {
                    matches!(
                        a,
                        crate::semantic_values::Atom::Incoming { .. }
                            | crate::semantic_values::Atom::Bits {
                                address_source: Some(_),
                                ..
                            }
                    )
                })
            {
                m.escaping_values
                    .insert(access.node.clone(), access.value.clone());
            }
        }
        for call in &s.calls {
            if call
                .callee
                .as_ref()
                .is_none_or(|c| !summaries.contains_key(c))
            {
                m.unknown(
                    &call.node,
                    match call.target {
                        CallTarget::Imported { .. } => "EFFECT_UNKNOWN_IMPORT",
                        CallTarget::Indirect { .. } => "EFFECT_UNKNOWN_INDIRECT",
                        _ => "EFFECT_UNKNOWN_CALLEE",
                    },
                );
            }
        }
        local.insert(fid.clone(), m);
    }
    let mut result = local.clone();
    for _ in 0..128 {
        let previous = result.clone();
        for (fid, s) in summaries {
            let m = result.get_mut(fid).expect("seeded");
            if m.exhausted {
                continue;
            }
            for call in &s.calls {
                let Some((callee, cm)) = call
                    .callee
                    .as_ref()
                    .and_then(|c| previous.get(c).map(|m| (c, m)))
                else {
                    continue;
                };
                m.summary_sources.insert(call.node.clone(), callee.clone());
                if !cm.complete {
                    m.unknown(&call.node, "EFFECT_INCOMPLETE_CALLEE");
                    // An arbitrary may-write subsumes individual writes. Keep
                    // the callee reference instead of repeatedly expanding its
                    // incomplete effects through every caller and recursive path.
                    continue;
                }
                let registers = call.inputs.iter().cloned().collect();
                for (source, value) in &cm.escaping_values {
                    let mut value = substitute(value, callee, &registers);
                    value.definitions.insert(call.node.clone());
                    m.escaping_values
                        .entry(source.clone())
                        .or_default()
                        .merge(&value);
                }
                if m.escaping_values.len() > MAX_EFFECTS {
                    m.widen();
                    break;
                }
                for effect in &cm.effects {
                    let e = instantiate(effect, callee, call);
                    if e.location.is_none() {
                        if e.write {
                            m.unknown_writes = true;
                        } else {
                            m.unknown_reads = true;
                        }
                        m.reasons
                            .insert(call.node.clone(), "EFFECT_SUBSTITUTION_UNKNOWN".into());
                    }
                    m.insert(e);
                }
            }
            m.effects.sort_by_cached_key(|e| {
                format!("{}:{:?}:{}:{}", e.source, e.location, e.width, e.write)
            });
        }
        if previous == result {
            return result;
        }
    }
    for m in result.values_mut() {
        m.widen();
    }
    result
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AliasQuery {
    pub storage: Location,
    pub storage_width: u8,
    pub effect: Effect,
    pub relationship: Alias,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallEffect {
    pub node: String,
    pub callee: Option<String>,
    pub classification: String,
    pub complete: bool,
    pub effects: Vec<Effect>,
    pub aliases: Vec<AliasQuery>,
    pub preserved: usize,
    pub invalidated: usize,
}
pub fn apply(call: &Call, summary: Option<&MemorySummary>, storage: &mut Storage) -> CallEffect {
    let mut result = CallEffect {
        node: call.node.clone(),
        callee: call.callee.clone(),
        classification: String::new(),
        complete: false,
        effects: vec![],
        aliases: vec![],
        preserved: 0,
        invalidated: 0,
    };
    let valid = summary.filter(|s| {
        s.complete && !s.unknown_writes && s.version == MEMORY_EFFECT_VERSION && !s.exhausted
    });
    let Some(s) = valid else {
        result.classification = if summary
            .is_some_and(|s| s.complete && s.unknown_writes && s.version == MEMORY_EFFECT_VERSION)
        {
            "EFFECT_MAY_ALIAS"
        } else {
            match call.target {
                CallTarget::Imported { .. } => "EFFECT_UNKNOWN_IMPORT",
                CallTarget::Indirect { .. } => "EFFECT_UNKNOWN_INDIRECT",
                _ => "EFFECT_INCOMPLETE_CALLEE",
            }
        }
        .into();
        result.invalidated = storage.words.len();
        storage.invalidate_at("UNKNOWN_CALL_EFFECT", &call.node);
        return result;
    };
    let callee = call.callee.as_deref().unwrap_or("");
    result.effects = s
        .effects
        .iter()
        .map(|e| instantiate(e, callee, call))
        .collect();
    result.complete = true;
    let writes: Vec<_> = result.effects.iter().filter(|e| e.write).collect();
    if writes.len().saturating_mul(storage.words.len()) > 4096 {
        result.classification = "EFFECT_ALIAS_BUDGET_EXHAUSTED".into();
        result.complete = false;
        result.invalidated = storage.words.len();
        storage.invalidate_at("UNKNOWN_CALL_EFFECT_ALIAS_BUDGET", &call.node);
        return result;
    }
    if writes.iter().any(|e| e.location.is_none()) {
        result.classification = "EFFECT_MAY_ALIAS".into();
        result.complete = false;
        result.invalidated = storage.words.len();
        storage.invalidate_at("UNKNOWN_CALL_EFFECT_ALIAS", &call.node);
        return result;
    }
    result.classification = if writes.is_empty() {
        "EFFECT_PROVEN_READ_ONLY"
    } else {
        "EFFECT_PRECISE_WRITE"
    }
    .into();
    for ((location, width), value) in &mut storage.words {
        let mut clobber = false;
        for effect in &writes {
            let relationship = alias(
                Some(location),
                *width,
                effect.location.as_ref(),
                effect.width,
            );
            clobber |= relationship != Alias::ProvenDisjoint;
            result.aliases.push(AliasQuery {
                storage: location.clone(),
                storage_width: *width,
                effect: (*effect).clone(),
                relationship,
            });
        }
        if clobber {
            *value = Fact::unknown("PRECISE_CALL_WRITE_MAY_ALIAS").at(&call.node);
            result.invalidated += 1;
        } else {
            value.definitions.insert(call.node.clone());
            if value.definitions.len() > MAX_ORIGINS {
                *value = Fact::unknown("VALUE_BUDGET_EXHAUSTED");
            }
            result.preserved += 1;
        }
    }
    for e in writes {
        storage
            .initial_kills
            .insert((e.location.clone().expect("checked"), e.width));
    }
    if result.invalidated == 0 && result.preserved > 0 && !s.effects.iter().all(|e| !e.write) {
        result.classification = "EFFECT_PROVEN_DISJOINT".into();
    }
    if storage.initial_kills.len() > crate::semantic_values::MAX_STORAGE {
        storage.invalidate_at("STORAGE_BUDGET_EXHAUSTED", &call.node);
        result.complete = false;
    }
    result
}
