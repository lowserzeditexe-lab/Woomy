//! Bounded, independent Cafe ELF parser. No loader or relocation application.
use flate2::read::ZlibDecoder;
use serde::Serialize;
use std::io::Read;
use thiserror::Error;

pub const RPX_PARSER_VERSION: u32 = 1;
pub const MAX_MODULE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_SECTION_BYTES: usize = 128 * 1024 * 1024;
const MAX_RECORDS: usize = 2_000_000;
pub const SHT_IMPORTS: u32 = 0x80000002;
pub const SHT_EXPORTS: u32 = 0x80000001;

#[derive(Debug, Error)]
pub enum Error {
    #[error("binary bounds violation at {offset}, length {length}")]
    Bounds { offset: usize, length: usize },
    #[error("invalid Cafe ELF: {0}")]
    Invalid(&'static str),
    #[error("resource limit: {0}")]
    Limit(&'static str),
    #[error("invalid zlib section")]
    Compression,
}
pub type Result<T> = std::result::Result<T, Error>;

pub struct BinaryReader<'a> {
    data: &'a [u8],
    position: usize,
}
impl<'a> BinaryReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    pub fn seek_checked(&mut self, offset: usize) -> Result<()> {
        if offset > self.data.len() {
            return Err(Error::Bounds { offset, length: 0 });
        }
        self.position = offset;
        Ok(())
    }
    pub fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.position.checked_add(length).ok_or(Error::Bounds {
            offset: self.position,
            length,
        })?;
        let data = self.data.get(self.position..end).ok_or(Error::Bounds {
            offset: self.position,
            length,
        })?;
        self.position = end;
        Ok(data)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.read_bytes(N)?
            .try_into()
            .map_err(|_| Error::Invalid("integer width"))
    }
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    pub fn read_be_u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.array()?))
    }
    pub fn read_be_u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    pub fn read_be_u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.array()?))
    }
}
fn string_at(data: &[u8], offset: u32) -> Result<String> {
    let tail = data
        .get(offset as usize..)
        .ok_or(Error::Invalid("string offset"))?;
    let end = tail
        .iter()
        .take(4097)
        .position(|b| *b == 0)
        .ok_or(Error::Invalid("unterminated/oversized string"))?;
    let s = std::str::from_utf8(&tail[..end]).map_err(|_| Error::Invalid("non UTF-8 string"))?;
    Ok(s.to_owned())
}
fn budgeted_string(data: &[u8], offset: u32, total: &mut usize) -> Result<String> {
    let value = string_at(data, offset)?;
    *total = total
        .checked_add(value.len())
        .ok_or(Error::Limit("metadata strings"))?;
    if *total > 32 * 1024 * 1024 {
        return Err(Error::Limit("metadata strings"));
    }
    Ok(value)
}

#[derive(Debug, Serialize)]
pub struct Section {
    pub index: u32,
    pub name: String,
    pub kind: u32,
    pub flags: u32,
    pub address: u32,
    pub offset: u32,
    pub file_size: u32,
    pub size: u32,
    pub link: u32,
    pub info: u32,
    pub alignment: u32,
    pub entry_size: u32,
    #[serde(skip)]
    pub data: Vec<u8>,
}
impl Section {
    pub fn executable(&self) -> bool {
        self.kind == 1 && self.flags & 4 != 0
    }
    pub fn contains(&self, address: u32) -> bool {
        address >= self.address && (address as u64) < self.address as u64 + self.size as u64
    }
}
#[derive(Debug, Serialize)]
pub struct Symbol {
    pub table: u32,
    pub index: u32,
    pub name: String,
    pub value: u32,
    pub size: u32,
    pub info: u8,
    pub section: u16,
}
#[derive(Debug, Serialize)]
pub struct Import {
    pub section: u32,
    pub library: String,
    pub count: u32,
    pub signature: u32,
    pub code: bool,
}
#[derive(Debug, Serialize)]
pub struct Export {
    pub section: u32,
    pub name: String,
    pub address: u32,
    pub code: bool,
}
#[derive(Debug, Serialize)]
pub struct Relocation {
    pub section: u32,
    pub target_section: u32,
    pub symbol_table: u32,
    pub offset: u32,
    pub kind: u8,
    pub symbol: u32,
    pub addend: Option<i32>,
}
#[derive(Debug, Serialize)]
pub struct Module {
    pub entry_point: u32,
    pub sections: Vec<Section>,
    pub imports: Vec<Import>,
    pub exports: Vec<Export>,
    pub symbols: Vec<Symbol>,
    pub relocations: Vec<Relocation>,
    pub provenance: &'static str,
}
impl Module {
    pub fn executable_address(&self, address: u32) -> bool {
        address & 3 == 0
            && self
                .sections
                .iter()
                .any(|s| s.executable() && s.contains(address))
    }
    pub fn word(&self, address: u32) -> Option<u32> {
        let s = self
            .sections
            .iter()
            .find(|s| s.executable() && s.contains(address))?;
        let i = (address - s.address) as usize;
        let data = s.data.get(i..i.checked_add(4)?)?;
        Some(u32::from_be_bytes(data.try_into().ok()?))
    }
}

pub fn parse(data: &[u8]) -> Result<Module> {
    let mut string_bytes = 0usize;
    if data.len() > MAX_MODULE_BYTES {
        return Err(Error::Limit("module bytes"));
    }
    let mut r = BinaryReader::new(data);
    if r.read_bytes(4)? != b"\x7fELF" {
        return Err(Error::Invalid("magic"));
    }
    if r.read_u8()? != 1 || r.read_u8()? != 2 || r.read_u8()? != 1 {
        return Err(Error::Invalid("expected ELF32 big endian version 1"));
    }
    if r.read_be_u16()? != 0xcafe {
        return Err(Error::Invalid("Cafe ABI"));
    }
    r.seek_checked(16)?;
    // 0xff01 is used by devkitPro; 0xfe01 is also observed in retail Cafe RPX.
    // Both retain the same validated Cafe ABI, ELF32 and PowerPC layout.
    if ![0xfe01, 0xff01].contains(&r.read_be_u16()?)
        || r.read_be_u16()? != 20
        || r.read_be_u32()? != 1
    {
        return Err(Error::Invalid("Cafe PowerPC header"));
    }
    let entry_point = r.read_be_u32()?;
    let phoff = r.read_be_u32()?;
    let shoff = r.read_be_u32()? as usize;
    let _flags = r.read_be_u32()?;
    if r.read_be_u16()? != 52 {
        return Err(Error::Invalid("ELF header size"));
    }
    let _phentsize = r.read_be_u16()?;
    if r.read_be_u16()? != 0 || phoff != 0 {
        return Err(Error::Invalid("program headers in Cafe ELF"));
    }
    let stride = r.read_be_u16()? as usize;
    let count = r.read_be_u16()? as usize;
    let names = r.read_be_u16()? as usize;
    if stride != 40 || count == 0 || count > 4096 || names >= count {
        return Err(Error::Invalid("section table dimensions"));
    }
    r.seek_checked(shoff)?;
    r.read_bytes(count * stride)?;
    let mut sections = Vec::with_capacity(count);
    let mut name_offsets = Vec::with_capacity(count);
    let mut total = 0usize;
    for index in 0..count {
        r.seek_checked(shoff + index * stride)?;
        name_offsets.push(r.read_be_u32()?);
        let kind = r.read_be_u32()?;
        let flags = r.read_be_u32()?;
        let address = r.read_be_u32()?;
        let offset = r.read_be_u32()?;
        let file_size = r.read_be_u32()?;
        let link = r.read_be_u32()?;
        let info = r.read_be_u32()?;
        let alignment = r.read_be_u32()?;
        let entry_size = r.read_be_u32()?;
        if alignment != 0 && !alignment.is_power_of_two() {
            return Err(Error::Invalid("section alignment"));
        }
        let mut size = file_size;
        let bytes = if kind == 8 || kind == 0 || file_size == 0 {
            Vec::new()
        } else {
            r.seek_checked(offset as usize)?;
            let src = r.read_bytes(file_size as usize)?;
            if flags & 0x08000000 != 0 {
                let mut sr = BinaryReader::new(src);
                size = sr.read_be_u32()?;
                if size as usize > MAX_SECTION_BYTES {
                    return Err(Error::Limit("inflated section"));
                }
                total = total
                    .checked_add(size as usize)
                    .ok_or(Error::Limit("total sections"))?;
                if total > MAX_MODULE_BYTES {
                    return Err(Error::Limit("total sections"));
                }
                let mut z = ZlibDecoder::new(&src[4..]);
                let mut out = Vec::new();
                (&mut z)
                    .take(size as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|_| Error::Compression)?;
                if out.len() != size as usize || z.total_in() as usize != src.len() - 4 {
                    return Err(Error::Compression);
                }
                out
            } else {
                if size as usize > MAX_SECTION_BYTES {
                    return Err(Error::Limit("section bytes"));
                }
                total = total
                    .checked_add(size as usize)
                    .ok_or(Error::Limit("total sections"))?;
                if total > MAX_MODULE_BYTES {
                    return Err(Error::Limit("total sections"));
                }
                src.to_vec()
            }
        };
        if address as u64 + size as u64 > 0x1_0000_0000 {
            return Err(Error::Invalid("section virtual range"));
        }
        sections.push(Section {
            index: index as u32,
            name: String::new(),
            kind,
            flags,
            address,
            offset,
            file_size,
            size,
            link,
            info,
            alignment,
            entry_size,
            data: bytes,
        });
    }
    if names != 0 {
        if sections[names].kind != 3 {
            return Err(Error::Invalid("section name table type"));
        }
        let resolved: Vec<String> = name_offsets
            .iter()
            .map(|n| budgeted_string(&sections[names].data, *n, &mut string_bytes))
            .collect::<Result<_>>()?;
        for (s, name) in sections.iter_mut().zip(resolved) {
            s.name = name;
        }
    }
    // Overlapping allocated code is ambiguous; reject instead of guessing.
    let mut ranges: Vec<_> = sections
        .iter()
        .filter(|s| s.executable() && s.size > 0)
        .map(|s| (s.address as u64, s.address as u64 + s.size as u64))
        .collect();
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[0].1 > w[1].0) {
        return Err(Error::Invalid("overlapping executable sections"));
    }
    let mut module = Module {
        entry_point,
        sections,
        imports: vec![],
        exports: vec![],
        symbols: vec![],
        relocations: vec![],
        provenance: "REAL",
    };
    let mut records = 0usize;
    for s in &module.sections {
        let mut sr = BinaryReader::new(&s.data);
        match s.kind {
            2 | 11 => {
                let strings = module
                    .sections
                    .get(s.link as usize)
                    .filter(|t| t.kind == 3)
                    .ok_or(Error::Invalid("symbol string table"))?;
                check_entries(s, 16, &mut records)?;
                for index in 0..s.data.len() / 16 {
                    let name =
                        budgeted_string(&strings.data, sr.read_be_u32()?, &mut string_bytes)?;
                    let value = sr.read_be_u32()?;
                    let size = sr.read_be_u32()?;
                    let info = sr.read_u8()?;
                    let _other = sr.read_u8()?;
                    let section = sr.read_be_u16()?;
                    if section != 0 && section < 0xff00 && section as usize >= module.sections.len()
                    {
                        return Err(Error::Invalid("symbol section index"));
                    }
                    module.symbols.push(Symbol {
                        table: s.index,
                        index: index as u32,
                        name,
                        value,
                        size,
                        info,
                        section,
                    });
                }
            }
            4 | 9 => {
                let width = if s.kind == 4 { 12 } else { 8 };
                check_entries(s, width, &mut records)?;
                if s.info as usize >= module.sections.len() {
                    return Err(Error::Invalid("relocation target section"));
                }
                let symbols = module
                    .sections
                    .get(s.link as usize)
                    .filter(|t| t.kind == 2 || t.kind == 11)
                    .ok_or(Error::Invalid("relocation symbol table"))?;
                for _ in 0..s.data.len() / width {
                    let offset = sr.read_be_u32()?;
                    let info = sr.read_be_u32()?;
                    if (info >> 8) as usize >= symbols.data.len() / 16 {
                        return Err(Error::Invalid("relocation symbol index"));
                    }
                    let addend = if width == 12 {
                        Some(sr.read_be_u32()? as i32)
                    } else {
                        None
                    };
                    module.relocations.push(Relocation {
                        section: s.index,
                        target_section: s.info,
                        symbol_table: s.link,
                        offset,
                        kind: info as u8,
                        symbol: info >> 8,
                        addend,
                    });
                }
            }
            SHT_IMPORTS => {
                let count = sr.read_be_u32()?;
                let signature = sr.read_be_u32()?;
                module.imports.push(Import {
                    section: s.index,
                    library: budgeted_string(&s.data, 8, &mut string_bytes)?,
                    count,
                    signature,
                    code: s.flags & 4 != 0,
                });
            }
            SHT_EXPORTS => {
                let count = sr.read_be_u32()? as usize;
                let _signature = sr.read_be_u32()?;
                if count > s.data.len().saturating_sub(8) / 8
                    || count > MAX_RECORDS.saturating_sub(records)
                {
                    return Err(Error::Invalid("export count"));
                }
                records += count;
                for _ in 0..count {
                    let address = sr.read_be_u32()?;
                    let offset = sr.read_be_u32()?;
                    module.exports.push(Export {
                        section: s.index,
                        name: budgeted_string(&s.data, offset, &mut string_bytes)?,
                        address,
                        code: s.flags & 4 != 0,
                    });
                }
            }
            _ => {}
        }
    }
    Ok(module)
}
fn check_entries(s: &Section, width: usize, records: &mut usize) -> Result<()> {
    if s.entry_size as usize != width || !s.data.len().is_multiple_of(width) {
        return Err(Error::Invalid("table entry size"));
    }
    *records = records
        .checked_add(s.data.len() / width)
        .ok_or(Error::Limit("records"))?;
    if *records > MAX_RECORDS {
        return Err(Error::Limit("records"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn endian_and_bounds() {
        let mut r = BinaryReader::new(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(r.read_be_u16().ok(), Some(0x0102));
        assert_eq!(r.read_be_u32().ok(), Some(0x03040506));
        assert!(r.read_be_u32().is_err());
        assert!(r.seek_checked(9).is_err());
        assert!(r.read_bytes(usize::MAX).is_err());
        assert!(r.seek_checked(0).is_ok());
        assert_eq!(r.read_be_u64().ok(), Some(0x0102030405060708));
    }
    #[test]
    fn rejects_header() {
        assert!(parse(b"not an ELF").is_err());
    }
    proptest! { #[test] fn arbitrary_files_never_panic(data in proptest::collection::vec(any::<u8>(), 0..4096)) { let _ = parse(&data); } }
}
