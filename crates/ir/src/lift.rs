use super::*;
use woomy_ppc::Operand;

struct Builder<'a> {
    module: &'a str,
    source: &'a DecodedInstruction,
    relocation: Option<&'a Relocation>,
    slot: u16,
    ops: Vec<Instruction>,
}
impl Builder<'_> {
    fn emit(&mut self, operation: Operation, ty: Option<Type>) -> ValueId {
        let id = ValueId {
            address: self.source.address,
            slot: self.slot,
        };
        self.slot += 1;
        self.ops.push(Instruction {
            id,
            result: ty.map(|ty| Value { id, ty }),
            operation,
            source: provenance(self.module, self.source),
        });
        id
    }
    fn constant(&mut self, bits: u32) -> ValueId {
        self.emit(Operation::Constant { bits: bits as u64 }, Some(Type::I32))
    }
    fn immediate(&mut self, signed: bool, shift: u8) -> ValueId {
        if let Some(relocation) = self.relocation {
            self.emit(
                Operation::RelocationImmediate {
                    relocation: relocation.clone(),
                    signed,
                    shift,
                },
                Some(Type::I32),
            )
        } else {
            let field = self.source.raw as u16;
            self.constant(
                (if signed {
                    field as i16 as i32 as u32
                } else {
                    field as u32
                }) << shift,
            )
        }
    }
    fn read(&mut self, register: Register) -> ValueId {
        self.emit(
            Operation::ReadRegister { register },
            Some(register_type(register)),
        )
    }
    fn write(&mut self, register: Register, value: ValueId) {
        self.emit(Operation::WriteRegister { register, value }, None);
    }
    fn binary(&mut self, operator: Binary, lhs: ValueId, rhs: ValueId) -> ValueId {
        self.emit(Operation::Binary { operator, lhs, rhs }, Some(Type::I32))
    }
    fn unknown(&mut self, reason: &str) {
        self.emit(
            Operation::UnknownInstruction {
                opcode: self.source.opcode.clone(),
                reason: reason.into(),
                effects: effects(self.source),
            },
            None,
        );
    }
    fn record(&mut self, value: ValueId) {
        let zero = self.constant(0);
        let xer = self.read(Register::Xer);
        let field = self.emit(
            Operation::Compare32 {
                lhs: value,
                rhs: zero,
                xer,
                signed: true,
            },
            Some(Type::I32),
        );
        self.write(Register::Cr(0), field);
    }
    fn flags(
        &mut self,
        lhs: ValueId,
        rhs: ValueId,
        result: ValueId,
        subtract: bool,
        carry: bool,
        overflow: bool,
    ) {
        if carry || overflow {
            let xer = self.read(Register::Xer);
            let flags = self.emit(
                Operation::IntegerFlags32 {
                    lhs,
                    rhs,
                    result,
                    xer,
                    subtract,
                    carry,
                    overflow,
                },
                Some(Type::I32),
            );
            self.write(Register::Xer, flags);
        }
    }
    fn call(&mut self, target: CallTarget) {
        self.emit(
            Operation::Call {
                target,
                link_value: self.source.address.wrapping_add(4),
                effects: unknown_effects(),
                signature: Type::Unknown,
            },
            None,
        );
    }
    fn instruction(&mut self) {
        let d = self.source;
        match d.control_flow {
            Flow::Branch {
                conditional: true, ..
            }
            | Flow::Indirect {
                conditional: true,
                link: false,
            } if d.status == "KNOWN"
                && d.raw & 1 == 0
                && (d.raw >> 26 == 16 || (d.raw >> 26 == 19 && (d.raw >> 1) & 1023 == 16)) =>
            {
                let bo = (d.raw >> 21) & 31;
                let bi = ((d.raw >> 16) & 31) as u8;
                self.emit(
                    Operation::BranchCondition {
                        cr_bit: if bo & 16 == 0 { Some(bi) } else { None },
                        cr_expected: bo & 8 != 0,
                        decrement_ctr: bo & 4 == 0,
                        ctr_expected_zero: bo & 2 != 0,
                    },
                    Some(Type::Bool),
                );
                return;
            }
            Flow::ExternalCall => {
                let target = d
                    .operands
                    .iter()
                    .find_map(|o| {
                        if let Operand::ImportSymbol { library, name } = o {
                            Some(CallTarget::Imported {
                                library: library.clone(),
                                symbol: name.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .unwrap_or(CallTarget::Unknown);
                self.call(target);
                return;
            }
            Flow::Call {
                target,
                conditional: false,
            } => {
                if target == d.address.wrapping_add(4) {
                    let pc = self.constant(target);
                    self.write(Register::Lr, pc);
                } else {
                    self.call(CallTarget::Static {
                        module: self.module.into(),
                        address: target,
                    });
                }
                return;
            }
            Flow::Indirect {
                link: true,
                conditional: false,
            } => {
                self.call(CallTarget::Indirect {
                    source: if (d.raw >> 1) & 1023 == 528 {
                        Register::Ctr
                    } else {
                        Register::Lr
                    },
                });
                return;
            }
            Flow::Branch {
                conditional: false, ..
            }
            | Flow::Return { conditional: false }
            | Flow::Trap
            | Flow::Indirect {
                link: false,
                conditional: false,
            } if d.status == "KNOWN" || d.status == "RELOCATION_MODELED" => return,
            Flow::Next => {}
            _ => {
                self.unknown("Control effects not lowered; see opaque terminator");
                return;
            }
        }
        if d.status != "KNOWN" && self.relocation.is_none() {
            self.unknown(
                "Relocation-dependent or unknown instruction; raw immediate is not a linked value",
            );
            return;
        }
        let rt = Register::Gpr(((d.raw >> 21) & 31) as u8);
        let ra = Register::Gpr(((d.raw >> 16) & 31) as u8);
        let rb = Register::Gpr(((d.raw >> 11) & 31) as u8);
        match d.opcode.as_str() {
            "cmpi" | "cmpli" | "cmp" | "cmpl" if d.raw & 0x0020_0000 == 0 => {
                let lhs = self.read(ra);
                let rhs = if d.opcode.ends_with('i') {
                    self.immediate(d.opcode == "cmpi", 0)
                } else {
                    self.read(rb)
                };
                let xer = self.read(Register::Xer);
                let field = self.emit(
                    Operation::Compare32 {
                        lhs,
                        rhs,
                        xer,
                        signed: !d.opcode.starts_with("cmpl"),
                    },
                    Some(Type::I32),
                );
                self.write(Register::Cr(((d.raw >> 23) & 7) as u8), field);
            }
            "addic" | "addic." | "subfic" => {
                let a = self.read(ra);
                let n = self.immediate(true, 0);
                let sub = d.opcode == "subfic";
                let (lhs, rhs) = if sub { (n, a) } else { (a, n) };
                let v = self.binary(
                    if sub {
                        Binary::SubWrapping
                    } else {
                        Binary::AddWrapping
                    },
                    lhs,
                    rhs,
                );
                self.write(rt, v);
                self.flags(lhs, rhs, v, sub, true, false);
                if d.opcode == "addic." {
                    self.record(v);
                }
            }
            "addi" | "addis" => {
                let base = if ra == Register::Gpr(0) {
                    self.constant(0)
                } else {
                    self.read(ra)
                };
                let n = self.immediate(true, if d.opcode == "addis" { 16 } else { 0 });
                let v = self.binary(Binary::AddWrapping, base, n);
                self.write(rt, v);
            }
            "ori" | "oris" | "xori" | "xoris" => {
                let src = self.read(rt);
                let n = self.immediate(false, if d.opcode.ends_with('s') { 16 } else { 0 });
                let v = self.binary(
                    if d.opcode.starts_with('o') {
                        Binary::Or
                    } else {
                        Binary::Xor
                    },
                    src,
                    n,
                );
                self.write(ra, v);
            }
            "add" | "subf" | "addc" | "subfc" => {
                let a = self.read(ra);
                let b = self.read(rb);
                let sub = d.opcode.starts_with("sub");
                let (lhs, rhs) = if sub { (b, a) } else { (a, b) };
                let v = self.binary(
                    if sub {
                        Binary::SubWrapping
                    } else {
                        Binary::AddWrapping
                    },
                    lhs,
                    rhs,
                );
                self.write(rt, v);
                self.flags(
                    lhs,
                    rhs,
                    v,
                    sub,
                    d.opcode.ends_with('c'),
                    d.raw & 0x400 != 0,
                );
                if d.raw & 1 != 0 {
                    self.record(v);
                }
            }
            "or" | "and" | "xor" => {
                let a = self.read(rt);
                let b = self.read(rb);
                let v = self.binary(
                    match d.opcode.as_str() {
                        "or" => Binary::Or,
                        "and" => Binary::And,
                        _ => Binary::Xor,
                    },
                    a,
                    b,
                );
                self.write(ra, v);
                if d.raw & 1 != 0 {
                    self.record(v);
                }
            }
            "rlwinm" => {
                let value = self.read(rt);
                let v = self.emit(
                    Operation::RotateMask32 {
                        value,
                        shift: ((d.raw >> 11) & 31) as u8,
                        begin: ((d.raw >> 6) & 31) as u8,
                        end: ((d.raw >> 1) & 31) as u8,
                    },
                    Some(Type::I32),
                );
                self.write(ra, v);
                if d.raw & 1 != 0 {
                    self.record(v);
                }
            }
            "mfspr" | "mtspr" => {
                let spr = ((d.raw >> 16) & 31) | ((d.raw >> 6) & 0x3e0);
                let reg = match spr {
                    8 => Register::Lr,
                    9 => Register::Ctr,
                    _ => {
                        self.unknown("SPR semantics not lowered");
                        return;
                    }
                };
                if d.opcode == "mfspr" {
                    let v = self.read(reg);
                    self.write(rt, v);
                } else {
                    let v = self.read(rt);
                    self.write(reg, v);
                }
            }
            "mulli" | "mullw" | "mulhw" | "mulhwu" | "slw" | "srw" | "neg"
                if d.opcode == "mulli"
                    || d.raw & 0x400 == 0
                    || ["mulhw", "mulhwu", "slw", "srw"].contains(&d.opcode.as_str()) =>
            {
                let shift = matches!(d.opcode.as_str(), "slw" | "srw");
                let a = if d.opcode == "neg" {
                    self.constant(0)
                } else {
                    self.read(if shift { rt } else { ra })
                };
                let b = if d.opcode == "mulli" {
                    self.immediate(true, 0)
                } else {
                    self.read(if d.opcode == "neg" { ra } else { rb })
                };
                let operator = match d.opcode.as_str() {
                    "mulhw" => Binary::MultiplyHighSigned,
                    "mulhwu" => Binary::MultiplyHighUnsigned,
                    "slw" => Binary::ShiftLeft,
                    "srw" => Binary::ShiftRight,
                    "neg" => Binary::SubWrapping,
                    _ => Binary::MultiplyLow,
                };
                let value = self.binary(operator, a, b);
                self.write(if shift { ra } else { rt }, value);
                if d.opcode != "mulli" && d.raw & 1 != 0 {
                    self.record(value);
                }
            }
            name if [
                "lwz", "lwzu", "lwzx", "lwzux", "lbz", "lbzu", "lbzx", "lbzux", "lhz", "lhzu",
                "lhzx", "lhzux", "stw", "stwu", "stwx", "stwux", "stb", "stbu", "stbx", "stbux",
                "sth", "sthu", "sthx", "sthux", "lmw", "stmw", "lfd", "lfdu", "stfd", "stfdu",
            ]
            .contains(&name) =>
            {
                let base = if ra == Register::Gpr(0) {
                    self.constant(0)
                } else {
                    self.read(ra)
                };
                let n = if name.ends_with('x') {
                    self.read(rb)
                } else {
                    self.immediate(true, 0)
                };
                let bits = self.binary(Binary::AddWrapping, base, n);
                let width = if name.contains('b') {
                    1
                } else if name.contains('h') {
                    2
                } else if name.contains('f') {
                    8
                } else {
                    4
                };
                let multiple = name == "lmw" || name == "stmw";
                let first = ((d.raw >> 21) & 31) as u8;
                for register in first..(if multiple { 32 } else { first + 1 }) {
                    let offset = self.constant((register - first) as u32 * 4);
                    let at = self.binary(Binary::AddWrapping, bits, offset);
                    let address = self.emit(Operation::Address32 { value: at }, Some(Type::Ptr));
                    let target = if width == 8 {
                        Register::Fpr(register)
                    } else {
                        Register::Gpr(register)
                    };
                    if name.starts_with('l') {
                        let value = self.emit(
                            Operation::Load {
                                address,
                                width,
                                endian: Endian::Big,
                                may_trap: true,
                            },
                            Some(register_type(target)),
                        );
                        self.write(target, value);
                    } else {
                        let value = self.read(target);
                        self.emit(
                            Operation::Store {
                                address,
                                value,
                                width,
                                endian: Endian::Big,
                                may_trap: true,
                            },
                            None,
                        );
                    }
                }
                if name.ends_with('u') || name.ends_with("ux") {
                    self.write(ra, bits);
                }
            }
            _ => self.unknown("Decoder effects known but executable IR semantics not implemented"),
        }
    }
}

/// Lower the already bounded CFG. No relocation value, calling convention or
/// unsupported machine effect is inferred from disassembly text.
pub fn lift(
    id: &str,
    module: &str,
    module_hash: &str,
    entry: u32,
    cfg: &Cfg,
    code: &BTreeMap<u32, DecodedInstruction>,
) -> Function {
    lift_with_relocations(id, module, module_hash, entry, cfg, code, &BTreeMap::new())
}
pub fn lift_with_relocations(
    id: &str,
    module: &str,
    module_hash: &str,
    entry: u32,
    cfg: &Cfg,
    code: &BTreeMap<u32, DecodedInstruction>,
    relocations: &BTreeMap<u32, Relocation>,
) -> Function {
    let mut function = Function {
        id: id.into(),
        module: module.into(),
        module_hash: module_hash.into(),
        entry,
        ir_version: IR_VERSION,
        decoder_version: woomy_ppc::PPC_DECODER_VERSION,
        cfg_version: woomy_ppc::CFG_ANALYZER_VERSION,
        analysis_version: woomy_ppc::DATAFLOW_VERSION,
        status: "LIFTED".into(),
        blocks: vec![],
        limitations: vec![
            "Structural validation does not establish execution equivalence".into(),
            "Function boundaries and CFG coverage are inferred; external effects remain explicit"
                .into(),
        ],
    };
    if cfg.truncated {
        function.limitations.push("Source CFG truncated".into());
        function.status = "PARTIALLY_LIFTED".into();
    }
    let block_ids: BTreeSet<_> = cfg.blocks.iter().map(|b| b.start).collect();
    let destination = |address: Option<u32>| match address {
        Some(a) if block_ids.contains(&a) => Destination::Block(a),
        Some(a) => Destination::External(a),
        None => Destination::Unknown,
    };
    let mut count = 0usize;
    for b in &cfg.blocks {
        let Some(last) = code.get(&b.end) else {
            function.status = "INVALID_SOURCE".into();
            continue;
        };
        let mut block = Block {
            id: b.start,
            sources: vec![],
            instructions: vec![],
            terminator: Terminator::OpaqueTerminator {
                reason: "Missing control semantics".into(),
            },
            terminator_source: provenance(module, last),
            successors: cfg
                .edges
                .iter()
                .filter(|e| {
                    e.from == b.start
                        && !(e.kind == "Indirect"
                            && matches!(last.control_flow, Flow::Indirect { link: true, .. }))
                })
                .map(|e| Successor {
                    destination: destination(e.to),
                    kind: e.kind.clone(),
                })
                .collect(),
        };
        for pc in &b.instructions {
            let Some(d) = code.get(pc) else {
                function.status = "INVALID_SOURCE".into();
                continue;
            };
            block.sources.push(Source {
                address: *pc,
                raw: d.raw,
                status: d.status.clone(),
            });
            let mut builder = Builder {
                module,
                source: d,
                relocation: relocations.get(pc),
                slot: 0,
                ops: vec![],
            };
            builder.instruction();
            count += builder.ops.len();
            if count > MAX_OPERATIONS {
                function.status = "LIFT_LIMIT".into();
                function
                    .limitations
                    .push("IR operation budget exceeded".into());
                function.blocks.clear();
                return function;
            }
            if builder.ops.iter().any(|i| {
                matches!(
                    i.operation,
                    Operation::UnknownInstruction { .. }
                        | Operation::Call { .. }
                        | Operation::RelocationImmediate {
                            relocation: Relocation {
                                local_address: None,
                                ..
                            },
                            ..
                        }
                )
            }) {
                function.status = "PARTIALLY_LIFTED".into();
            }
            block.instructions.extend(builder.ops);
        }
        block.terminator=match last.control_flow {
            Flow::Indirect{link:false,conditional:true} if last.status=="KNOWN" && last.raw>>26==19 && (last.raw>>1)&1023==16 && last.raw&1==0 => Terminator::ConditionalLrBranch {condition:block.instructions.last().expect("bclr condition").id,not_taken:destination(Some(last.address.wrapping_add(4)))},
            Flow::Branch{target,conditional:true} if last.status=="KNOWN" && last.raw>>26==16 && last.raw&1==0=>Terminator::Branch{condition:block.instructions.last().expect("branch condition").id,taken:destination(Some(target)),not_taken:destination(Some(last.address.wrapping_add(4)))},
            Flow::Branch{target,conditional:false}=>Terminator::Jump{target:destination(Some(target))},
            Flow::Return{conditional:false} if block.successors.is_empty()=>Terminator::Return{source:Register::Lr,clear_low_bits:2},
            Flow::Trap=>Terminator::Trap,
            Flow::Indirect{link:false,conditional:false}=>Terminator::UnresolvedIndirectBranch{source:if (last.raw>>1)&1023==528{Register::Ctr}else{Register::Lr},clear_low_bits:2},
            Flow::Next|Flow::ExternalCall|Flow::Call{conditional:false,..}|Flow::Indirect{link:true,conditional:false}=> {
                // The indirect call edge describes its callee, not the next block.
                let edges:Vec<_>=block.successors.iter().filter(|s|s.kind=="Fallthrough" || (s.kind=="Unknown" && s.destination==destination(last.address.checked_add(4)))).collect();
                if edges.len()==1 {Terminator::Jump{target:edges[0].destination.clone()}}else{Terminator::OpaqueTerminator{reason:"Missing or ambiguous continuation".into()}}
            }
            _=>Terminator::OpaqueTerminator{reason:"Conditional/unknown control effects not lowered; successors are analysis evidence only".into()},
        };
        if matches!(
            block.terminator,
            Terminator::OpaqueTerminator { .. } | Terminator::UnresolvedIndirectBranch { .. }
        ) || block.successors.iter().any(|e| {
            matches!(
                e.destination,
                Destination::Unknown | Destination::External(_)
            )
        }) {
            function.status = "PARTIALLY_LIFTED".into();
        }
        function.blocks.push(block);
    }
    function
}
