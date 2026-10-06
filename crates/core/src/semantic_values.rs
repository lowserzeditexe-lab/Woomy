//! Semantic v3 abstract values and point-specific storage. No host pointers.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::runtime::{GuestMemory, READ, WRITE};
use woomy_ppc::Register;

pub const VALUE_PROVENANCE_VERSION: u32 = 2;
pub const MAX_ALTERNATIVES: usize = 8;
pub const MAX_ORIGINS: usize = 128;
pub const MAX_STORAGE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Atom {
    Bits {
        bits: u32,
        address_source: Option<u32>,
    },
    Incoming {
        function: String,
        register: Register,
        offset: i32,
    },
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fact {
    pub alternatives: BTreeSet<Atom>,
    pub unknown: BTreeSet<String>,
    pub definitions: BTreeSet<String>,
}
impl Fact {
    pub fn unknown(reason: &str) -> Self {
        Self {
            unknown: BTreeSet::from([reason.into()]),
            ..Self::default()
        }
    }
    pub fn atom(atom: Atom) -> Self {
        Self {
            alternatives: BTreeSet::from([atom]),
            ..Self::default()
        }
    }
    pub fn bits(bits: u32) -> Self {
        Self::atom(Atom::Bits {
            bits,
            address_source: None,
        })
    }
    pub fn truncate(&self, mask: u32) -> Self {
        let mut out = Self {
            unknown: self.unknown.clone(),
            definitions: self.definitions.clone(),
            ..Self::default()
        };
        for atom in &self.alternatives {
            match atom {
                Atom::Bits { bits, .. } => out.merge(&Self::bits(bits & mask)),
                _ => {
                    out.unknown.insert("TRUNCATED_SYMBOLIC_VALUE".into());
                }
            }
        }
        out
    }
    pub fn merge(&mut self, other: &Self) {
        if self.unknown.contains("VALUE_BUDGET_EXHAUSTED")
            || other.unknown.contains("VALUE_BUDGET_EXHAUSTED")
        {
            *self = Self::unknown("VALUE_BUDGET_EXHAUSTED");
            return;
        }
        self.alternatives.extend(other.alternatives.iter().cloned());
        self.unknown.extend(other.unknown.iter().cloned());
        self.definitions.extend(other.definitions.iter().cloned());
        if self.alternatives.len() > MAX_ALTERNATIVES || self.definitions.len() > MAX_ORIGINS {
            *self = Self::unknown("VALUE_BUDGET_EXHAUSTED");
        }
    }
    pub fn at(mut self, node: &str) -> Self {
        // The graph retains predecessors; a fact contains immediate definitions.
        self.definitions = BTreeSet::from([node.into()]);
        self
    }
    pub fn status(&self) -> &'static str {
        if self.unknown.iter().any(|r| r.contains("CALL")) {
            "CLOBBERED"
        } else if !self.unknown.is_empty() || self.alternatives.is_empty() {
            "UNKNOWN"
        } else if self.alternatives.len() == 1 {
            "DEFINITELY_ONE"
        } else {
            "FINITE_SET"
        }
    }
    pub fn address(&self) -> Option<Location> {
        if !self.unknown.is_empty() || self.alternatives.len() != 1 {
            return None;
        }
        match self.alternatives.first()? {
            Atom::Bits { bits, .. } => Some(Location::Global(*bits)),
            Atom::Incoming {
                function,
                register,
                offset,
            } => Some(Location::Symbolic {
                function: function.clone(),
                register: *register,
                offset: *offset,
            }),
        }
    }
    pub fn add(&self, other: &Self) -> Self {
        if self.unknown.contains("VALUE_BUDGET_EXHAUSTED")
            || other.unknown.contains("VALUE_BUDGET_EXHAUSTED")
        {
            return Self::unknown("VALUE_BUDGET_EXHAUSTED");
        }
        let mut result = Self::default();
        result.unknown.extend(self.unknown.iter().cloned());
        result.unknown.extend(other.unknown.iter().cloned());
        for a in &self.alternatives {
            for b in &other.alternatives {
                let atom = match (a, b) {
                    (
                        Atom::Bits {
                            bits: a,
                            address_source: x,
                        },
                        Atom::Bits {
                            bits: b,
                            address_source: y,
                        },
                    ) => Atom::Bits {
                        bits: a.wrapping_add(*b),
                        address_source: match (x, y) {
                            (Some(x), Some(y)) if x == y => Some(*x),
                            (Some(x), None) | (None, Some(x)) => Some(*x),
                            _ => None,
                        },
                    },
                    (
                        Atom::Incoming {
                            function,
                            register,
                            offset,
                        },
                        Atom::Bits {
                            bits,
                            address_source: None,
                        },
                    )
                    | (
                        Atom::Bits {
                            bits,
                            address_source: None,
                        },
                        Atom::Incoming {
                            function,
                            register,
                            offset,
                        },
                    ) => Atom::Incoming {
                        function: function.clone(),
                        register: *register,
                        offset: offset.wrapping_add(*bits as i32),
                    },
                    _ => {
                        result
                            .unknown
                            .insert("UNSUPPORTED_ADDRESS_ARITHMETIC".into());
                        continue;
                    }
                };
                result.merge(&Self::atom(atom));
            }
        }
        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Location {
    Global(u32),
    Symbolic {
        function: String,
        register: Register,
        offset: i32,
    },
}
impl Location {
    pub(crate) fn overlaps(&self, width: u8, other: &Self, other_width: u8) -> bool {
        let pair = match (self, other) {
            (Self::Global(a), Self::Global(b)) => Some((*a as i64, *b as i64)),
            (
                Self::Symbolic {
                    function: a,
                    register: ar,
                    offset: ao,
                },
                Self::Symbolic {
                    function: b,
                    register: br,
                    offset: bo,
                },
            ) if a == b && ar == br => {
                return (bo.wrapping_sub(*ao) as u32) < width as u32
                    || (ao.wrapping_sub(*bo) as u32) < other_width as u32
            }
            // Distinct symbolic inputs can alias each other and globals.
            _ => None,
        };
        pair.is_none_or(|(a, b)| a < b + other_width as i64 && b < a + width as i64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialWord {
    pub address: u32,
    pub target: u32,
    pub evidence: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Storage {
    pub words: BTreeMap<(Location, u8), Fact>,
    pub initial_kills: BTreeSet<(Location, u8)>,
    pub invalidations: BTreeSet<String>,
    pub causes: BTreeSet<String>,
}
impl Storage {
    pub fn invalidate_at(&mut self, reason: &str, node: &str) {
        self.invalidate(reason);
        self.causes.insert(node.into());
        if self.causes.len() > MAX_ORIGINS {
            self.causes.clear();
            self.invalidations
                .insert("STORAGE_CAUSE_BUDGET_EXHAUSTED".into());
        }
    }
    pub fn invalidate(&mut self, reason: &str) {
        self.words.clear();
        self.initial_kills.clear();
        self.invalidations.insert(reason.into());
    }
    pub fn store(
        &mut self,
        location: Option<Location>,
        width: u8,
        value: Fact,
        memory: &GuestMemory,
    ) {
        let Some(location) = location else {
            self.invalidate("UNKNOWN_ALIAS");
            return;
        };
        if let Location::Global(a) = location {
            if memory.locate(a, width as usize, WRITE).is_none() {
                self.invalidate("UNMAPPED_OR_NONWRITABLE_STORE");
                return;
            }
        }
        self.words
            .retain(|(other, w), _| !location.overlaps(width, other, *w));
        // Retain writes even when their values are replaced, to prevent fallback
        // to stale loader bytes. Read-only image words cannot be overwritten by
        // a successful permission-checked store.
        self.initial_kills.insert((location.clone(), width));
        let value = match width {
            1 => value.truncate(0xff),
            2 => value.truncate(0xffff),
            4 => value,
            _ => Fact::unknown("UNSUPPORTED_WIDE_STORAGE_VALUE"),
        };
        self.words.insert((location, width), value);
        if self.words.len() > MAX_STORAGE || self.initial_kills.len() > MAX_STORAGE {
            self.invalidate("STORAGE_BUDGET_EXHAUSTED");
        }
    }
    pub fn load(
        &self,
        location: Option<Location>,
        width: u8,
        memory: &GuestMemory,
        initial: &BTreeMap<u32, InitialWord>,
        loader_entry: bool,
    ) -> Fact {
        let Some(location) = location else {
            return Fact::unknown("UNKNOWN_LOAD_ADDRESS");
        };
        if let Location::Global(a) = location {
            if memory.locate(a, width as usize, READ).is_none() {
                return Fact::unknown("UNMAPPED_OR_UNREADABLE_LOAD");
            }
        }
        if let Some(fact) = self.words.get(&(location.clone(), width)) {
            return fact.clone();
        }
        if !self.invalidations.is_empty() {
            return Fact {
                unknown: self.invalidations.clone(),
                definitions: self.causes.clone(),
                ..Fact::default()
            };
        }
        let Location::Global(a) = location else {
            return Fact::unknown("UNKNOWN_INCOMING_STORAGE");
        };
        let Some(word) = initial.get(&a).filter(|_| width == 4) else {
            return Fact::unknown("NO_RELOCATION_INITIALIZATION");
        };
        let Some((region, offset)) = memory.locate(a, 4, READ) else {
            return Fact::unknown("UNREADABLE_INITIALIZATION");
        };
        if memory.regions[region].permissions & WRITE != 0
            && self
                .initial_kills
                .iter()
                .any(|(l, w)| l.overlaps(*w, &Location::Global(a), width))
        {
            return Fact::unknown("UNKNOWN_WRITER_OVERLAP");
        }
        if memory.regions[region].permissions & WRITE != 0 && !loader_entry {
            return Fact::unknown("MUTABLE_STORAGE_AT_UNKNOWN_ENTRY");
        }
        let observed = u32::from_be_bytes(
            memory.regions[region].bytes[offset..offset + 4]
                .try_into()
                .expect("located word"),
        );
        if observed != word.target {
            return Fact::unknown("CONFLICTING_INITIALIZATION");
        }
        Fact::atom(Atom::Bits {
            bits: word.target,
            address_source: Some(word.target),
        })
        .at(&word.evidence)
    }
    pub fn load_value(
        &self,
        address: &Fact,
        width: u8,
        memory: &GuestMemory,
        initial: &BTreeMap<u32, InitialWord>,
        loader_entry: bool,
    ) -> Fact {
        let mut result = Fact {
            unknown: address.unknown.clone(),
            ..Fact::default()
        };
        for atom in &address.alternatives {
            result.merge(&self.load(
                Fact::atom(atom.clone()).address(),
                width,
                memory,
                initial,
                loader_entry,
            ));
        }
        if address.alternatives.is_empty() {
            result.unknown.insert("UNKNOWN_LOAD_ADDRESS".into());
        }
        result
    }
    pub fn join(
        states: &[&Self],
        memory: &GuestMemory,
        initial: &BTreeMap<u32, InitialWord>,
        loader_entry: bool,
    ) -> Self {
        let mut result = Self::default();
        let keys: BTreeSet<_> = states
            .iter()
            .flat_map(|s| s.words.keys().cloned())
            .collect();
        if keys.len() > MAX_STORAGE {
            result.invalidate("STORAGE_JOIN_BUDGET_EXHAUSTED");
            return result;
        }
        for state in states {
            result.causes.extend(state.causes.iter().cloned());
            result
                .invalidations
                .extend(state.invalidations.iter().cloned());
            result
                .initial_kills
                .extend(state.initial_kills.iter().cloned());
        }
        if result.initial_kills.len() > MAX_STORAGE || result.causes.len() > MAX_ORIGINS {
            result.invalidate("STORAGE_JOIN_BUDGET_EXHAUSTED");
            return result;
        }
        for (location, width) in keys {
            let mut fact = Fact::default();
            for state in states {
                fact.merge(&state.load(
                    Some(location.clone()),
                    width,
                    memory,
                    initial,
                    loader_entry,
                ));
            }
            result.words.insert((location, width), fact);
        }
        result
    }
}
