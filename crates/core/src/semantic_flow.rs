//! Bounded forward CFG analysis. No memory values are cached: stores/aliasing
//! cannot justify reuse of a load. Calls and opaque effects kill all facts.
use crate::{semantic::Expr, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{Binary, Destination, Function, Operation, Terminator, ValueId};
use woomy_ppc::Register;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Anchor {
    pub block: u32,
    pub pc: u32,
    pub operation: ValueId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowFact {
    pub block: u32,
    pub register: Register,
    pub value: Expr,
    pub predecessors: Vec<u32>,
    pub consumer: Anchor,
    pub definitions: BTreeSet<Anchor>,
    pub rule: String,
}
#[derive(Clone, PartialEq, Eq)]
struct Fact {
    value: Expr,
    definitions: BTreeSet<Anchor>,
}
type State = BTreeMap<Register, Fact>;
pub struct Flow {
    pub incoming: BTreeMap<u32, BTreeMap<Register, Expr>>,
    pub facts: Vec<FlowFact>,
    pub iterations: usize,
}
fn unknown() -> Fact {
    Fact {
        value: Expr::Unknown,
        definitions: BTreeSet::new(),
    }
}
fn join(states: &[&State], registers: &BTreeSet<Register>) -> State {
    registers
        .iter()
        .map(|reg| {
            let values: Vec<_> = states
                .iter()
                .map(|s| s.get(reg).cloned().unwrap_or_else(unknown))
                .collect();
            let mut result = values.first().cloned().unwrap_or_else(unknown);
            if values.iter().any(|f| f.value == Expr::Unknown) {
                result = unknown();
            } else {
                let mut alternatives = BTreeSet::new();
                for fact in &values {
                    match &fact.value {
                        Expr::Conflict(v) => alternatives.extend(v.clone()),
                        v => {
                            alternatives.insert(v.clone());
                        }
                    }
                    result.definitions.extend(fact.definitions.clone());
                }
                if alternatives.len() > 4 || result.definitions.len() > 256 {
                    result = unknown();
                } else if alternatives.len() > 1 {
                    result.value = Expr::Conflict(alternatives.into_iter().collect());
                }
            }
            (*reg, result)
        })
        .collect()
}
fn transfer(f: &Function, block: u32, mut state: State) -> State {
    let mut values = BTreeMap::<ValueId, Fact>::new();
    let b = f
        .blocks
        .iter()
        .find(|b| b.id == block)
        .expect("known block");
    for op in &b.instructions {
        let get = |id: &ValueId| values.get(id).cloned().unwrap_or_else(unknown);
        let mut fact = match &op.operation {
            Operation::Constant { bits } => Fact {
                value: Expr::Constant(*bits as u32),
                definitions: BTreeSet::new(),
            },
            Operation::ReadRegister { register } => {
                state.get(register).cloned().unwrap_or_else(unknown)
            }
            Operation::WriteRegister { value, .. } | Operation::Address32 { value } => get(value),
            Operation::Binary {
                operator: Binary::AddWrapping,
                lhs,
                rhs,
            } => {
                let a = get(lhs);
                let b = get(rhs);
                let value = match (&a.value, &b.value) {
                    (Expr::Constant(a), Expr::Constant(b)) => Expr::Constant(a.wrapping_add(*b)),
                    (Expr::Base(base, offset), Expr::Constant(c))
                    | (Expr::Constant(c), Expr::Base(base, offset)) => offset
                        .checked_add(*c as i32 as i64)
                        .filter(|v| i32::try_from(*v).is_ok())
                        .map_or(Expr::Unknown, |v| Expr::Base(base.clone(), v)),
                    _ => Expr::Unknown,
                };
                Fact {
                    value,
                    definitions: a.definitions.union(&b.definitions).cloned().collect(),
                }
            }
            Operation::BranchCondition {
                decrement_ctr: true,
                ..
            } => {
                state.insert(Register::Ctr, unknown());
                unknown()
            }
            Operation::Call { .. }
            | Operation::UnknownInstruction { .. }
            | Operation::UnknownSideEffect { .. }
            | Operation::OpaqueMemoryEffect { .. } => {
                state.clear();
                values.clear();
                unknown()
            }
            _ => unknown(),
        };
        if fact.value != Expr::Unknown {
            fact.definitions.insert(Anchor {
                block,
                pc: op.source.address,
                operation: op.id,
            });
            if fact.definitions.len() > 256 {
                fact = unknown();
            }
        }
        if let Operation::WriteRegister { register, .. } = op.operation {
            state.insert(register, fact.clone());
        }
        if let Some(result) = &op.result {
            values.insert(result.id, fact);
        }
    }
    state
}
pub fn analyze(f: &Function) -> Result<Flow> {
    let registers: BTreeSet<_> =
        f.blocks
            .iter()
            .flat_map(|b| &b.instructions)
            .filter_map(|i| match i.operation {
                Operation::ReadRegister { register }
                | Operation::WriteRegister { register, .. } => Some(register),
                _ => None,
            })
            .collect();
    let entry: State = registers
        .iter()
        .map(|r| {
            (
                *r,
                Fact {
                    value: Expr::Base(format!("incoming.{r:?}"), 0),
                    definitions: BTreeSet::new(),
                },
            )
        })
        .collect();
    let mut predecessors = BTreeMap::<u32, BTreeSet<u32>>::new();
    for b in &f.blocks {
        let destinations = match b.terminator {
            Terminator::ConditionalLrBranch {
                not_taken: Destination::Block(t),
                ..
            } => vec![t],
            Terminator::Jump {
                target: Destination::Block(t),
            } => vec![t],
            Terminator::Branch {
                taken: Destination::Block(a),
                not_taken: Destination::Block(b),
                ..
            } => vec![a, b],
            _ => vec![],
        };
        for target in destinations {
            predecessors.entry(target).or_default().insert(b.id);
        }
    }
    let mut ins = BTreeMap::<u32, State>::new();
    let mut outs = BTreeMap::<u32, State>::new();
    let mut iterations = 0;
    loop {
        iterations += 1;
        if iterations > 256 {
            return Err(Error::Analysis("SEMANTIC_CFG_FIXED_POINT_BUDGET".into()));
        }
        let mut changed = false;
        for b in &f.blocks {
            let mut states: Vec<_> = predecessors
                .get(&b.id)
                .into_iter()
                .flatten()
                .filter_map(|p| outs.get(p))
                .collect();
            if b.id == f.entry {
                states.push(&entry);
            }
            if states.is_empty() {
                continue;
            }
            let input = join(&states, &registers);
            let output = transfer(f, b.id, input.clone());
            changed |= outs.get(&b.id) != Some(&output);
            ins.insert(b.id, input);
            outs.insert(b.id, output);
        }
        if !changed {
            break;
        }
    }
    let mut facts = vec![];
    for (block, state) in &ins {
        if *block == f.entry {
            continue;
        }
        for (register, fact) in state {
            if fact.value == Expr::Unknown {
                continue;
            }
            // Report only facts actually consumed before a local definition.
            let used = f
                .blocks
                .iter()
                .find(|b| b.id == *block)
                .unwrap()
                .instructions
                .iter()
                .find_map(|i| match i.operation {
                    Operation::ReadRegister { register: r } if r == *register => Some(true),
                    Operation::WriteRegister { register: r, .. } if r == *register => Some(false),
                    Operation::Call { .. }
                    | Operation::UnknownInstruction { .. }
                    | Operation::UnknownSideEffect { .. }
                    | Operation::OpaqueMemoryEffect { .. } => Some(false),
                    _ => None,
                })
                .unwrap_or(false);
            if used {
                facts.push(FlowFact {consumer: { let op=f.blocks.iter().find(|b|b.id==*block).unwrap().instructions.iter().find(|i|matches!(i.operation,Operation::ReadRegister {register:r} if r==*register)).unwrap(); Anchor {block:*block,pc:op.source.address,operation:op.id} },block:*block,register:*register,value:fact.value.clone(),predecessors:predecessors.get(block).into_iter().flatten().filter(|p|outs.contains_key(p)).copied().collect(),definitions:fact.definitions.clone(),rule:"ALL_REACHABLE_PREDECESSORS_AGREE_OR_EXPLICIT_CONFLICT; UNKNOWN_DOMINATES; NO_MEMORY_VALUE_REUSE".into()});
            }
        }
    }
    Ok(Flow {
        incoming: ins
            .into_iter()
            .map(|(b, s)| (b, s.into_iter().map(|(r, f)| (r, f.value)).collect()))
            .collect(),
        facts,
        iterations,
    })
}
