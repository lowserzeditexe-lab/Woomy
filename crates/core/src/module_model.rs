//! Logical Cafe addresses. Original bytes are never relocated or patched.
//! Only explicit-addend ADDR32/LO/HI/HA and REL24 are modeled (PPC ELF ABI 4-8).
use std::collections::BTreeMap;
use woomy_binary::{Module, Relocation, Symbol};
use woomy_ppc::dataflow::{add, Atom, State, Target, Value, ValueModel};
use woomy_ppc::{decode, DecodedInstruction, Flow, Operand, Register, Semantics};

pub struct ModuleModel<'a> {
    pub id: &'a str,
    pub module: &'a Module,
    relocations: BTreeMap<u32, Vec<&'a Relocation>>,
    symbols: BTreeMap<(u32, u32), &'a Symbol>,
    image: Option<&'a crate::cafe::CafeGuestImage>,
}
impl<'a> ModuleModel<'a> {
    pub fn new(id: &'a str, module: &'a Module) -> Self {
        let mut relocations: BTreeMap<u32, Vec<&Relocation>> = BTreeMap::new();
        for r in &module.relocations {
            if r.kind != 0 {
                relocations.entry(r.offset).or_default().push(r);
            }
        }
        Self {
            id,
            module,
            image: None,
            relocations,
            symbols: module
                .symbols
                .iter()
                .map(|s| ((s.table, s.index), s))
                .collect(),
        }
    }
    /// Audit-only opt-in: proven immutable image bytes supplement existing facts.
    /// Mutable initial bytes are never constants; load() still honors alias taint.
    pub fn with_image(
        id: &'a str,
        module: &'a Module,
        image: &'a crate::cafe::CafeGuestImage,
    ) -> crate::Result<Self> {
        image.verify()?;
        if image.module() != id
            || image.regions().iter().any(|r| {
                module
                    .sections
                    .iter()
                    .find(|s| s.index == r.section)
                    .is_none_or(|s| {
                        s.address != r.start
                            || s.size as u64 != r.end - r.start as u64
                            || (s.kind != 8 && crate::hash_bytes(&s.data) != r.initial_hash)
                    })
            })
        {
            return Err(crate::Error::Analysis(
                "LOADER_VALUE_MODEL_IDENTITY_MISMATCH".into(),
            ));
        }
        let mut model = Self::new(id, module);
        model.image = Some(image);
        Ok(model)
    }
    fn affected(&self, pc: u32) -> Vec<&Relocation> {
        self.relocations
            .range(pc.saturating_sub(3)..=pc.saturating_add(3))
            .flat_map(|(_, rs)| rs.iter().copied())
            .filter(|r| {
                let width = if [4, 5, 6].contains(&r.kind) { 2u64 } else { 4 };
                r.offset as u64 + width > pc as u64
            })
            .take(2) // Two overlapping records already make the field ambiguous.
            .collect()
    }
    fn valid_place(&self, r: &Relocation, width: u32) -> bool {
        let Some(s) = self.module.sections.get(r.target_section as usize) else {
            return false;
        };
        s.contains(r.offset) && r.offset as u64 + width as u64 <= s.address as u64 + s.size as u64
    }
    pub fn relocated_target(&self, r: &Relocation) -> Option<Target> {
        let addend = r.addend?;
        let s = self.symbols.get(&(r.symbol_table, r.symbol))?;
        if let Some(import) = self
            .module
            .imports
            .iter()
            .find(|i| i.section == s.section as u32)
        {
            if s.name.is_empty() {
                return None;
            }
            return Some(Target::ImportedSymbol {
                library: import.library.clone(),
                symbol: s.name.clone(),
                addend,
                is_code: import.code,
            });
        }
        if s.section == 0 || s.section >= 0xff00 {
            return None;
        }
        let section = self.module.sections.get(s.section as usize)?;
        let address = s.value.wrapping_add(addend as u32);
        if section.flags & 2 == 0 {
            return None;
        }
        // S + A can designate a different allocated section (Cafe's $TEXT
        // anchor, for example). Validate the resulting address, not its addend
        // against the symbol's section size.
        self.logical_address(address)
    }
    pub fn address(&self, target: &Target) -> Option<u32> {
        let Target::Address {
            module,
            section,
            offset,
        } = target
        else {
            return None;
        };
        if module != self.id {
            return None;
        }
        let s = self.module.sections.get(*section as usize)?;
        if *offset >= s.size {
            return None;
        }
        s.address.checked_add(*offset)
    }
    pub fn executable_target(&self, atom: &Atom) -> Option<u32> {
        let address = match atom {
            Atom::Constant(n) => *n & !3,
            Atom::Target(t) => self.address(t)? & !3,
            _ => return None,
        };
        self.module.word(address).map(|_| address)
    }
    pub fn loader_identity(&self) -> Option<&str> {
        self.image.map(|image| image.hash())
    }
    pub fn logical_address(&self, address: u32) -> Option<Target> {
        let mut matches = self
            .module
            .sections
            .iter()
            .filter(|s| s.flags & 2 != 0 && [1, 8].contains(&s.kind) && s.contains(address));
        let s = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(Target::Address {
            module: self.id.into(),
            section: s.index,
            offset: address - s.address,
        })
    }
    pub fn instruction(&self, pc: u32) -> Option<DecodedInstruction> {
        let raw = self.module.word(pc)?;
        let mut d = decode(pc, raw);
        let rs = self.affected(pc);
        if rs.is_empty() {
            return Some(d);
        }
        let original = d.clone();
        if d.status == "KNOWN" {
            d.status = "RELOCATION_REQUIRED".into();
        }
        d.semantics = Semantics::Unknown;
        if rs.len() != 1
            || !matches!(raw >> 26,7|8|10..=15|24..=29|32..=55)
            || !rs
                .iter()
                .all(|r| [4, 5, 6].contains(&r.kind) && r.offset == pc.saturating_add(2))
        {
            d.control_flow = Flow::Unknown;
        }
        if rs.len() != 1 || original.status != "KNOWN" {
            return Some(d);
        }
        let r = rs[0];
        if [4, 5, 6].contains(&r.kind)
            && r.offset == pc.checked_add(2)?
            && self.valid_place(r, 2)
            && self.relocated_target(r).is_some()
            && matches!(raw>>26,7|8|10..=15|24..=29|32..=55)
        {
            d.semantics = original.semantics.clone();
            d.control_flow = original.control_flow;
            d.status = "RELOCATION_MODELED".into();
        }
        if r.kind == 10
            && r.offset == pc
            && self.valid_place(r, 4)
            && raw >> 26 == 18
            && raw & 2 == 0
        {
            if let Some(target) = self.relocated_target(r) {
                match &target {
                    Target::ImportedSymbol {
                        library,
                        symbol,
                        addend: 0,
                        ..
                    } if raw & 1 != 0 => {
                        let code_import = self
                            .symbols
                            .get(&(r.symbol_table, r.symbol))
                            .is_some_and(|s| {
                                self.module
                                    .imports
                                    .iter()
                                    .any(|i| i.section == s.section as u32 && i.code)
                            });
                        if code_import {
                            d.control_flow = Flow::ExternalCall;
                            d.operands = vec![Operand::ImportSymbol {
                                library: library.clone(),
                                name: symbol.clone(),
                            }];
                            d.status = "IMPORT_REFERENCE".into();
                        }
                    }
                    Target::Address { .. } => {
                        if let Some(t) = self
                            .address(&target)
                            .filter(|t| self.module.executable_address(*t))
                        {
                            let delta = t.wrapping_sub(pc) as i32;
                            if (-0x0200_0000..0x0200_0000).contains(&delta) {
                                d.control_flow = if raw & 1 != 0 {
                                    Flow::Call {
                                        target: t,
                                        conditional: false,
                                    }
                                } else {
                                    Flow::Branch {
                                        target: t,
                                        conditional: false,
                                    }
                                };
                                d.operands = vec![Operand::BranchTarget(t)];
                                d.semantics = original.semantics;
                                d.status = "RELOCATION_MODELED".into();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(d)
    }
    /// Supplies identity and a proven local guest address, never the encoded
    /// immediate. Imported identities deliberately have no numeric address.
    pub fn ir_relocation(&self, pc: u32) -> Option<woomy_ir::Relocation> {
        let records = self.affected(pc);
        if records.len() != 1 {
            return None;
        }
        let r = records[0];
        if ![4, 5, 6].contains(&r.kind) || r.offset != pc.checked_add(2)? || !self.valid_place(r, 2)
        {
            return None;
        }
        let d = self.instruction(pc)?;
        if d.status != "RELOCATION_MODELED" {
            return None;
        }
        let target = self.relocated_target(r)?;
        let symbol = self.symbols.get(&(r.symbol_table, r.symbol))?;
        Some(woomy_ir::Relocation {
            module: self.id.into(),
            symbol: symbol.name.clone(),
            symbol_table: r.symbol_table,
            symbol_index: r.symbol,
            kind: r.kind,
            addend: r.addend?,
            place: r.offset,
            local_address: self.address(&target),
            target,
        })
    }
    /// Reads immutable file-backed words only. Mutable/BSS/import cells are runtime values.
    pub fn read_word(&self, address: u32) -> Value {
        if self
            .image
            .is_some_and(|image| !image.range_resolved(address, 4))
        {
            return Value::Unknown;
        }
        let old = self.original_read_word(address);
        if old != Value::Unknown {
            return old;
        }
        let Some(image) = self.image else {
            return old;
        };
        if address & 3 != 0 || !image.range_resolved(address, 4) {
            return Value::Unknown;
        }
        let Some((i, n)) = image.memory().locate(address, 4, woomy_ir::runtime::READ) else {
            return Value::Unknown;
        };
        let region = &image.memory().regions[i];
        if region.permissions & woomy_ir::runtime::WRITE != 0 {
            return Value::Unknown;
        }
        Value::constant(u32::from_be_bytes(
            region.bytes[n..n + 4].try_into().expect("checked word"),
        ))
    }
    fn original_read_word(&self, address: u32) -> Value {
        if address & 3 != 0 {
            return Value::Unknown;
        }
        let Some(Target::Address {
            section, offset, ..
        }) = self.logical_address(address)
        else {
            return Value::Unknown;
        };
        let s = &self.module.sections[section as usize];
        if s.kind != 1 || s.flags & 1 != 0 {
            return Value::Unknown;
        }
        let Some(bytes) = s.data.get(offset as usize..offset as usize + 4) else {
            return Value::Unknown;
        };
        let rs = self.affected(address);
        if !rs.is_empty() {
            if rs.len() == 1
                && rs[0].offset == address
                && rs[0].kind == 1
                && self.valid_place(rs[0], 4)
            {
                return self
                    .relocated_target(rs[0])
                    .map(|t| Value::Known(Atom::Target(t)))
                    .unwrap_or_default();
            }
            return Value::Unknown;
        }
        let Ok(bytes) = bytes.try_into() else {
            return Value::Unknown;
        };
        Value::constant(u32::from_be_bytes(bytes))
    }
    fn load(&self, address: &Value, input: &State) -> Value {
        if input.memory_tainted {
            return Value::Unknown;
        }
        let Some(atoms) = address.atoms() else {
            return Value::Unknown;
        };
        let mut result = None;
        for atom in atoms {
            let address = match atom {
                Atom::Constant(n) => Some(n),
                Atom::Target(t) => self.address(&t),
                _ => None,
            };
            let v = address.map(|a| self.read_word(a)).unwrap_or_default();
            result = Some(result.map_or_else(|| v.clone(), |old: Value| old.join(&v)));
        }
        result.unwrap_or_default()
    }
    fn low_pair(&self, base: &Value, target: &Target, adjusted: bool) -> Value {
        base.map(|a| match a {
            Atom::Upper {
                target: t,
                adjusted: ha,
            } if t == target && *ha == adjusted => Some(Atom::Target(target.clone())),
            _ => None,
        })
    }
}
impl ValueModel for ModuleModel<'_> {
    fn evaluate(&self, d: &DecodedInstruction, input: &State) -> Option<BTreeMap<Register, Value>> {
        let rt = Register::Gpr(((d.raw >> 21) & 31) as u8);
        let ra = Register::Gpr(((d.raw >> 16) & 31) as u8);
        let rb = Register::Gpr(((d.raw >> 11) & 31) as u8);
        let base = if (d.raw >> 16) & 31 == 0 {
            Value::constant(0)
        } else {
            input.value(ra)
        };
        let rs = self.affected(d.address);
        let mut out = BTreeMap::new();
        if d.status == "RELOCATION_MODELED" {
            let Some(r) = rs.first() else {
                return Some(out);
            };
            let Some(target) = self.relocated_target(r) else {
                return Some(out);
            };
            match (d.opcode.as_str(), r.kind) {
                ("addis", 5 | 6) if (d.raw >> 16) & 31 == 0 => {
                    out.insert(
                        rt,
                        Value::Known(Atom::Upper {
                            target,
                            adjusted: r.kind == 6,
                        }),
                    );
                }
                ("addi", 4) => {
                    out.insert(rt, self.low_pair(&base, &target, true));
                }
                ("ori", 4) => {
                    out.insert(ra, self.low_pair(&input.value(rt), &target, false));
                }
                ("lwz" | "lwzu", 4) => {
                    let address = self.low_pair(&base, &target, true);
                    out.insert(rt, self.load(&address, input));
                    if d.opcode == "lwzu" {
                        out.insert(ra, address);
                    }
                }
                _ => {}
            }
            return Some(out);
        }
        if d.status != "KNOWN" {
            return None;
        }
        if ["lwz", "lwzu", "lwzx", "lwzux"].contains(&d.opcode.as_str()) {
            let offset = if d.opcode.ends_with('x') {
                input.value(rb)
            } else {
                Value::constant(d.raw as u16 as i16 as i32 as u32)
            };
            let address = add(&base, &offset);
            out.insert(rt, self.load(&address, input));
            if d.opcode.contains('u') {
                out.insert(ra, address);
            }
            return Some(out);
        }
        None
    }
}
