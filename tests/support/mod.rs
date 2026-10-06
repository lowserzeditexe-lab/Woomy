//! All bytes here are generated fixtures, never proprietary binaries.
pub fn put16(b: &mut [u8], at: usize, n: u16) {
    b[at..at + 2].copy_from_slice(&n.to_be_bytes());
}
pub fn put32(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_be_bytes());
}
pub fn cafe_fixture() -> Vec<u8> {
    let instructions = [
        0x38600001u32,
        0x2c030000,
        0x4182000c,
        0x48000015,
        0x4bfffff8,
        0x4e800020,
        0,
        0x60000000,
        0x38630001,
        0x4e800020,
    ];
    let text: Vec<u8> = instructions.iter().flat_map(|w| w.to_be_bytes()).collect();
    let strings = b"\0helper\0OSReport\0".to_vec();
    let mut symbols = vec![0u8; 48];
    put32(&mut symbols, 16, 1);
    put32(&mut symbols, 20, 0x1020);
    put32(&mut symbols, 24, 8);
    symbols[28] = 0x12;
    put16(&mut symbols, 30, 1);
    put32(&mut symbols, 32, 8);
    put32(&mut symbols, 36, 0xc0000010);
    symbols[44] = 0x12;
    put16(&mut symbols, 46, 5);
    let mut imports = vec![0u8; 24];
    put32(&mut imports, 0, 1);
    imports[8..17].copy_from_slice(b"coreinit\0");
    let mut exports = vec![0u8; 29];
    put32(&mut exports, 0, 1);
    put32(&mut exports, 8, 0x1000);
    put32(&mut exports, 12, 24);
    exports[24..].copy_from_slice(b"main\0");
    let mut reloc = vec![0u8; 12];
    put32(&mut reloc, 0, 0x2000);
    put32(&mut reloc, 4, 0x101);
    let names = [
        "",
        ".text",
        ".data",
        ".strtab",
        ".symtab",
        ".fimport_coreinit",
        ".fexports",
        ".rela.data",
        ".shstrtab",
    ];
    let mut shstr = vec![];
    let mut offsets = vec![];
    for name in names {
        offsets.push(shstr.len() as u32);
        shstr.extend_from_slice(name.as_bytes());
        shstr.push(0);
    }
    let sections = [
        vec![],
        text,
        vec![0; 4],
        strings,
        symbols,
        imports,
        exports,
        reloc,
        shstr,
    ];
    let mut out = vec![0u8; 64 + sections.len() * 40];
    out[0..9].copy_from_slice(b"\x7fELF\x01\x02\x01\xca\xfe");
    put16(&mut out, 16, 0xfe01);
    put16(&mut out, 18, 20);
    put32(&mut out, 20, 1);
    put32(&mut out, 24, 0x1000);
    put32(&mut out, 32, 64);
    put16(&mut out, 40, 52);
    put16(&mut out, 46, 40);
    put16(&mut out, 48, 9);
    put16(&mut out, 50, 8);
    for (i, bytes) in sections.iter().enumerate() {
        let at = 64 + i * 40;
        let offset = out.len() as u32;
        put32(&mut out, at, offsets[i]);
        put32(
            &mut out,
            at + 4,
            [0, 1, 1, 3, 2, 0x80000002, 0x80000001, 4, 3][i],
        );
        put32(&mut out, at + 8, [0, 6, 3, 0, 0, 6, 6, 0, 0][i]);
        put32(
            &mut out,
            at + 12,
            [0, 0x1000, 0x2000, 0, 0, 0xc0000000, 0, 0, 0][i],
        );
        put32(&mut out, at + 16, offset);
        put32(&mut out, at + 20, bytes.len() as u32);
        put32(
            &mut out,
            at + 24,
            if i == 4 {
                3
            } else if i == 7 {
                4
            } else {
                0
            },
        );
        put32(&mut out, at + 28, if i == 7 { 2 } else { 0 });
        put32(&mut out, at + 32, 4);
        put32(
            &mut out,
            at + 36,
            if i == 4 {
                16
            } else if i == 7 {
                12
            } else {
                0
            },
        );
        out.extend_from_slice(bytes);
    }
    out
}
