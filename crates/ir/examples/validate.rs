//! Reproducible synthetic differential report. No game inputs are read/executed.
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use woomy_ir::execution::{compare, Machine};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: validate <report.json>")?;
    let words = [
        0x3863ffffu32,
        0x7c632214,
        0x7c632050,
        0x5465103a,
        0x90a60000,
        0x80e60000,
        0x4e800020,
    ];
    let bytes: Vec<_> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let hash = format!("{:x}", Sha256::digest(&bytes));
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
    let ir = woomy_ir::lift(
        "synthetic.scalar.00001000",
        "synthetic.scalar",
        &hash,
        0x1000,
        &cfg,
        &code,
    );
    let mut seed = 0x76543210u32;
    let mut cases = Vec::new();
    for index in 0..512 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let mut state = Machine::new(0x2000, vec![0xa5; 64]);
        for (i, r) in state.gpr.iter_mut().enumerate() {
            *r = seed.rotate_left(i as u32);
        }
        state.gpr[6] = 0x2000;
        state.lr = 0x80000003;
        state.ctr = !seed;
        state.xer = seed;
        state.fpscr = seed.rotate_right(3);
        state.cr = [15; 8];
        state.fpr = [0x7ff8000000004321; 32];
        let report = compare(&ir, &raw, state, 128);
        cases.push(serde_json::json!({"case":index,"seed":seed,"status":report.status,"state_equal":report.state_equal,"trace_equal":report.trace_equal,"exit_equal":report.exit_equal}));
    }
    let matches = cases.iter().filter(|c| c["status"] == "MATCH").count();
    let report = serde_json::json!({"scope":"Synthetic scalar fixture only; not validation of any game function","ir_version":woomy_ir::IR_VERSION,"fixture_sha256":hash,"cases":cases,"matched":matches,"total":512,"structural_verification":woomy_ir::verify(&ir)});
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "Synthetic PPC/IR differential report: {matches}/512 MATCH; {}",
        output.display()
    );
    if matches != 512 {
        return Err("differential validation failed".into());
    }
    Ok(())
}
