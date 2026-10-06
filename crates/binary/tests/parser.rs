#[path = "../../../tests/support/mod.rs"]
mod support;
use std::io::Write;
use support::*;
use woomy_binary::*;
#[test]
fn real_layout_sections_symbols_imports_exports_relocations() {
    let data = cafe_fixture();
    let m = parse(&data).unwrap();
    assert_eq!(m.entry_point, 0x1000);
    assert_eq!(m.sections.len(), 9);
    assert_eq!(m.sections[1].name, ".text");
    assert_eq!(m.symbols[1].name, "helper");
    assert_eq!(m.imports[0].library, "coreinit");
    assert_eq!(m.exports[0].address, 0x1000);
    assert_eq!(m.exports[0].name, "main");
    assert_eq!(m.relocations[0].symbol, 1);
    assert_eq!(m.relocations[0].offset, 0x2000);
    assert_eq!(m.relocations[0].addend, Some(0));
    let mut homebrew = data;
    put16(&mut homebrew, 16, 0xff01);
    assert!(parse(&homebrew).is_ok());
}
#[test]
fn every_truncation_rejected_without_panic() {
    let data = cafe_fixture();
    for n in 0..data.len() {
        assert!(parse(&data[..n]).is_err(), "length {n}");
    }
}
#[test]
fn malformed_offsets_indices_and_export_counts() {
    for (at, value) in [
        (32, u32::MAX),
        (64 + 40 + 16, u32::MAX),
        (64 + 4 * 40 + 24, 999),
        (64 + 7 * 40 + 28, 999),
        (64 + 4 * 40 + 36, 0),
    ] {
        let mut data = cafe_fixture();
        put32(&mut data, at, value);
        assert!(parse(&data).is_err());
    }
    let mut data = cafe_fixture();
    let m = parse(&data).unwrap();
    let at = m.sections[6].offset as usize;
    put32(&mut data, at, u32::MAX);
    assert!(parse(&data).is_err());
}
#[test]
fn compressed_section_and_bomb_limit() {
    let mut data = cafe_fixture();
    let m = parse(&data).unwrap();
    let src = &m.sections[1].data;
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
    z.write_all(src).unwrap();
    let compressed = z.finish().unwrap();
    let at = data.len();
    data.extend_from_slice(&(src.len() as u32).to_be_bytes());
    data.extend_from_slice(&compressed);
    put32(&mut data, 64 + 40 + 8, 0x08000006);
    put32(&mut data, 64 + 40 + 16, at as u32);
    put32(&mut data, 64 + 40 + 20, compressed.len() as u32 + 4);
    assert_eq!(parse(&data).unwrap().sections[1].data, *src);
    put32(&mut data, at, u32::MAX);
    assert!(matches!(parse(&data), Err(Error::Limit(_))));
}
#[test]
fn compressed_size_mismatch_and_truncated_stream() {
    let mut data = cafe_fixture();
    let mut z = flate2::write::ZlibEncoder::new(vec![], flate2::Compression::default());
    z.write_all(&[1, 2, 3, 4]).unwrap();
    let compressed = z.finish().unwrap();
    let at = data.len();
    data.extend_from_slice(&5u32.to_be_bytes());
    data.extend_from_slice(&compressed);
    put32(&mut data, 104 + 8, 0x08000006);
    put32(&mut data, 104 + 16, at as u32);
    put32(&mut data, 104 + 20, compressed.len() as u32 + 4);
    assert!(parse(&data).is_err());
}
