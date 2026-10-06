use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{execution::Machine, runtime::*, *};
fn branch(pc: u32, target: u32) -> u32 {
    0x48000001 | (target.wrapping_sub(pc) & 0x03fffffc)
}
fn caller(entry: u32, targets: &[u32]) -> Vec<u32> {
    let mut words = vec![0x9421ffe0, 0x7c0802a6, 0x90010024];
    for target in targets {
        words.push(branch(entry + words.len() as u32 * 4, *target));
    }
    words.extend([0x80010024, 0x7c0803a6, 0x38210020, 0x4e800020]);
    words
}
pub fn names() -> Vec<&'static str> {
    vec![
        "direct",
        "nested",
        "multiple",
        "recursive",
        "ctr_call",
        "lr_call",
        "memory_widths",
        "memory_updates_indexed",
        "integer",
        "test_add",
        "test_memory_copy",
        "test_return_constant",
    ]
}
pub fn fixture(name: &str) -> (Program, BTreeMap<u32, u32>) {
    let leaf = vec![0x7c632214, 0x90660000, 0x7c6903a6, 0x34e3ffff, 0x4e800020];
    let mut functions = vec![(0x1000, caller(0x1000, &[0x1100])), (0x1100, leaf.clone())];
    match name {
        "nested" => {
            functions[1].1 = caller(0x1100, &[0x1200]);
            functions.push((0x1200, leaf));
        }
        "multiple" => {
            functions[0].1 = caller(0x1000, &[0x1100, 0x1200, 0x1100]);
            functions.push((0x1200, vec![0x38630003, 0x4e800020]));
        }
        "recursive" => {
            functions = vec![(
                0x1000,
                vec![
                    0x9421ffe0,
                    0x7c0802a6,
                    0x90010024,
                    0x2c030000,
                    0x4182000c,
                    0x3863ffff,
                    branch(0x1018, 0x1000),
                    0x80010024,
                    0x7c0803a6,
                    0x38210020,
                    0x4e800020,
                ],
            )];
        }
        "ctr_call" | "lr_call" => {
            let spr = if name == "ctr_call" {
                0x7d8903a6
            } else {
                0x7d8803a6
            };
            let call = if name == "ctr_call" {
                0x4e800421
            } else {
                0x4e800021
            };
            functions[0].1 = vec![
                0x9421ffe0, 0x7c0802a6, 0x90010024, 0x39801100, spr, call, 0x80010024, 0x7c0803a6,
                0x38210020, 0x4e800020,
            ];
        }
        "memory_widths" => {
            functions[1].1 = vec![
                0x98660001, 0x88e60001, 0xb0860002, 0xa1060002, 0x90660004, 0x81260004, 0xd8060008,
                0xc8260008, 0xbf860020, 0xbb860020, 0x4e800020,
            ]
        }
        "memory_updates_indexed" => {
            let d = |op: u32, t: u32, a: u32, imm: u32| (op << 26) | (t << 21) | (a << 16) | imm;
            let x = |xo: u32, t: u32, a: u32, b: u32| {
                (31 << 26) | (t << 21) | (a << 16) | (b << 11) | (xo << 1)
            };
            let mut words = vec![x(444, 6, 14, 6), d(14, 15, 0, 4)];
            for (store, load) in [(37, 33), (39, 35), (45, 41), (55, 51)] {
                words.push(d(store, 3, 14, 8));
                words.push(d(load, 4, 14, 8));
            }
            for (store, load) in [
                (151, 23),
                (183, 55),
                (215, 87),
                (247, 119),
                (407, 279),
                (439, 311),
            ] {
                words.push(x(store, 3, 14, 15));
                words.push(x(load, 4, 14, 15));
            }
            words.push(0x4e800020);
            functions[1].1 = words;
        }
        "integer" => {
            functions[1].1 = vec![
                0x1ce3ffff, 0x7d0321d6, 0x7d232096, 0x7d432016, 0x7c6b2030, 0x7c6c2430, 0x7da300d0,
                0x4e800020,
            ]
        }
        n if n.starts_with("test_") => {
            functions = vec![(0x1000, caller(0x1000, &[0x7000]))];
        }
        _ => {}
    }
    let raw: BTreeMap<_, _> = functions
        .iter()
        .flat_map(|(entry, words)| {
            words
                .iter()
                .enumerate()
                .map(move |(n, w)| (*entry + n as u32 * 4, *w))
        })
        .collect();
    let mut code: BTreeMap<_, _> = raw
        .iter()
        .map(|(pc, w)| (*pc, woomy_ppc::decode(*pc, *w)))
        .collect();
    let mut imports = BTreeMap::new();
    if name.starts_with("test_") {
        let import = Import {
            library: "woomy_test".into(),
            symbol: name.into(),
        };
        let d = code.get_mut(&0x100c).unwrap();
        d.control_flow = woomy_ppc::Flow::ExternalCall;
        d.operands = vec![woomy_ppc::Operand::ImportSymbol {
            library: import.library.clone(),
            name: import.symbol.clone(),
        }];
        imports.insert(0x100c, import);
    }
    let entries: BTreeSet<_> = functions.iter().map(|f| f.0).collect();
    let functions = functions
        .iter()
        .map(|(entry, _)| {
            let cfg = woomy_ppc::build_cfg(*entry, &code, &entries, 1000);
            lift(
                &format!("synthetic.guest.{entry:08X}"),
                "synthetic.guest",
                &"3".repeat(64),
                *entry,
                &cfg,
                &code,
            )
        })
        .collect();
    (
        Program {
            runtime_context: None,
            source_barriers: BTreeMap::new(),
            image: None,
            version: RUNTIME_ABI_VERSION,
            entry: 0x1000,
            functions,
            imports,
            provider: ProviderKind::TestRuntime,
            limits: Limits::default(),
        },
        raw,
    )
}
pub fn states(
    program: &Program,
    raw: &BTreeMap<u32, u32>,
    name: &str,
    count: usize,
) -> Vec<(Machine, usize)> {
    let mut seed = 0x13579bdfu32;
    (0..count)
        .map(|index| {
            let mut s = Machine::new(0, vec![]);
            for v in &mut s.gpr {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *v = seed;
            }
            for v in &mut s.fpr {
                *v = ((seed as u64) << 32) | (!seed as u64);
                seed = seed.wrapping_add(0x87654321);
            }
            s.xer = seed;
            s.fpscr = !seed;
            s.cr = std::array::from_fn(|n| ((seed >> (n * 4)) & 15) as u8);
            s.ctr = !seed;
            if index < 5 {
                s.gpr[3] = [0, 1, 0xffffffff, 0x80000000, 0x7fffffff][index];
                s.gpr[4] = [0, 31, 32, 63, 64][index];
            }
            s.lr = 0xffff0003;
            s.gpr[1] = 0x31c0;
            s.gpr[6] = 0x2000;
            if name == "recursive" {
                s.gpr[3] = index as u32 % 8;
            }
            if name == "test_memory_copy" {
                s.gpr[3] = 0x2040;
                s.gpr[4] = 0x2000;
                s.gpr[5] = 32;
            }
            for f in &program.functions {
                let first = f
                    .blocks
                    .iter()
                    .flat_map(|b| &b.sources)
                    .map(|s| s.address)
                    .min()
                    .unwrap();
                let last = f
                    .blocks
                    .iter()
                    .flat_map(|b| &b.sources)
                    .map(|s| s.address)
                    .max()
                    .unwrap();
                let bytes = (first..=last)
                    .step_by(4)
                    .flat_map(|pc| raw.get(&pc).copied().unwrap_or(0).to_be_bytes())
                    .collect();
                s.guest_memory.regions.push(Region {
                    base: first,
                    permissions: READ | EXECUTE,
                    kind: 0,
                    provenance: "SYNTHETIC_PPC".into(),
                    bytes,
                });
            }
            s.guest_memory.regions.push(Region {
                base: 0x2000,
                permissions: READ | WRITE,
                kind: 2,
                provenance: "SYNTHETIC_DATA".into(),
                bytes: (0..256).map(|n| (n ^ index) as u8).collect(),
            });
            s.guest_memory.regions.push(Region {
                base: 0x3000,
                permissions: READ | WRITE,
                kind: 3,
                provenance: "SYNTHETIC_STACK".into(),
                bytes: vec![0; 512],
            });
            (s, 512)
        })
        .collect()
}
