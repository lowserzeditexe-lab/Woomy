use std::collections::{BTreeMap, BTreeSet};
use woomy_ir::{execution::Machine, Function};
pub fn fixture(words: &[u32]) -> (Function, BTreeMap<u32, u32>) {
    let raw: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(i, w)| (0x1000 + i as u32 * 4, *w))
        .collect();
    let code = raw
        .iter()
        .map(|(pc, w)| (*pc, woomy_ppc::decode(*pc, *w)))
        .collect();
    let cfg = woomy_ppc::build_cfg(0x1000, &code, &BTreeSet::from([0x1000]), 100);
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    (
        woomy_ir::lift(
            "synthetic.native.00001000",
            "synthetic.native",
            &woomy_codegen::hash(&bytes),
            0x1000,
            &cfg,
            &code,
        ),
        raw,
    )
}
pub fn programs() -> Vec<(&'static str, Vec<u32>)> {
    let mut programs = vec![
        (
            "arithmetic",
            vec![0x3863ffff, 0x7c632214, 0x7c632050, 0x5465103a, 0x4e800020],
        ),
        (
            "memory",
            vec![
                0x3863ffff, 0x7c632214, 0x7c632050, 0x5465103a, 0x90a60000, 0x80e60000, 0x90660004,
                0x81060004, 0x4e800020,
            ],
        ),
        (
            "blocks",
            vec![
                0x7d0802a6, 0x48000005, 0x7c6802a6, 0x48000008, 0x386000ff, 0x7d0803a6, 0x7c6903a6,
                0x7c8902a6, 0x4e800020,
            ],
        ),
        ("trap", vec![0x3863ffff, 0x0fe00000]),
        (
            "compare_branch",
            vec![
                0x2c030000, 0x4182000c, 0x39200001, 0x48000008, 0x39200002, 0x2883ffff, 0x7c832000,
                0x7d032040, 0x4e800020,
            ],
        ),
        (
            "ctr_loop",
            vec![0x38a00003, 0x7ca903a6, 0x38630001, 0x4200fffc, 0x4e800020],
        ),
        (
            "xer_record",
            vec![
                0x3463ffff, 0x7c632615, 0x7ca32015, 0x7cc32011, 0x7ce32451, 0x5465103b, 0x7c632039,
                0x4e800020,
            ],
        ),
    ];
    programs.push(("relocation_ha", vec![0x3c600000, 0x38630000, 0x4e800020]));
    programs.push(("relocation_hi", vec![0x3c600000, 0x60630000, 0x4e800020]));
    let mut bo_cases = Vec::new();
    for bo in 0..32u32 {
        bo_cases.push((16 << 26) | (bo << 21) | (bo << 16) | 8);
        bo_cases.push(0x396b0001);
    }
    bo_cases.push(0x4e800020);
    programs.push(("branch_options", bo_cases));
    programs
}
pub fn fixture_for(name: &str, words: &[u32]) -> (Function, BTreeMap<u32, u32>) {
    if name != "relocation_ha" && name != "relocation_hi" {
        return fixture(words);
    }
    let (base, mut raw) = fixture(words);
    let mut decoded: BTreeMap<_, _> = raw
        .iter()
        .map(|(pc, w)| (*pc, woomy_ppc::decode(*pc, *w)))
        .collect();
    for pc in [0x1000, 0x1004] {
        decoded.get_mut(&pc).unwrap().status = "RELOCATION_MODELED".into();
    }
    let cfg = woomy_ppc::build_cfg(0x1000, &decoded, &BTreeSet::from([0x1000]), 100);
    let relocations = [
        (0x1000, if name == "relocation_ha" { 6 } else { 5 }),
        (0x1004, 4),
    ]
    .into_iter()
    .map(|(pc, kind)| {
        (
            pc,
            woomy_ir::Relocation {
                module: base.module.clone(),
                symbol: "synthetic_data".into(),
                symbol_table: 4,
                symbol_index: 1,
                kind,
                addend: 0,
                place: pc + 2,
                target: woomy_ppc::dataflow::Target::Address {
                    module: base.module.clone(),
                    section: 2,
                    offset: 0,
                },
                local_address: Some(0x1234abcd),
            },
        )
    })
    .collect();
    let ir = woomy_ir::lift_with_relocations(
        &base.id,
        &base.module,
        &base.module_hash,
        base.entry,
        &cfg,
        &decoded,
        &relocations,
    );
    // Independently specified linked reference copy. Original source words in
    // IR retain zero placeholders; no original file or IR provenance is patched.
    raw.insert(
        0x1000,
        if name == "relocation_ha" {
            0x3c601235
        } else {
            0x3c601234
        },
    );
    raw.insert(
        0x1004,
        if name == "relocation_ha" {
            0x3863abcd
        } else {
            0x6063abcd
        },
    );
    (ir, raw)
}
pub fn states(count: usize) -> Vec<(Machine, usize)> {
    let mut seed = 0x76543210u32;
    (0..count)
        .map(|case| {
            let mut state = Machine::new(0x2000, vec![0xa5; 64]);
            for r in &mut state.gpr {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                *r = seed;
            }
            state.gpr[3] = match case {
                0 => 0,
                1 => 1,
                2 => 0x7fffffff,
                3 => 0x80000000,
                4 => u32::MAX,
                _ => seed,
            };
            state.gpr[6] = 0x2000;
            state.lr = 0x80000003;
            state.ctr = match case {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => !seed,
            };
            state.xer = seed;
            state.fpscr = seed.rotate_left(3);
            for (i, r) in state.cr.iter_mut().enumerate() {
                *r = ((seed >> (i * 4)) & 15) as u8;
            }
            for (i, r) in state.fpr.iter_mut().enumerate() {
                *r = 0x7ff8000000004321u64.rotate_left(i as u32);
            }
            (state, 128)
        })
        .collect()
}
