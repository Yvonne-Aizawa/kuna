//! Nintendo 3DS CRO/CRS dynamic-module loader.
//!
//! A module is loaded at address 0, so addresses are file offsets, with each
//! segment of its table mapped at its file offset under its type's permissions
//! (`.text` executable, `.rodata` read-only, `.data` writable) and `.bss` zero
//! filled after the file. The relocations RO applies when it loads the module are
//! applied to the mapped bytes: the internal ones (`R_ARM_ABS32` and friends)
//! point a patched word at another segment, and the import ones point it at a
//! four-byte `bx lr` stub in a block after `.bss`, one per import, named after the
//! import; a thunk of the form `ldr pc,[pc,#n]` through such a word takes the
//! import's name too, so the module's calls to it read as calls to the import. Without them a jump table is zeros and every call through an imported
//! pointer is anonymous. `option crorelocs off` skips both.
//!
//! The format is parsed by hand: the `object` crate does not know it. Layout is
//! 3dbrew's CRO0 page; the header starts after four SHA-256 hashes at 0x80.

use std::cell::Cell;
use std::rc::Rc;

use kuna_base::address::{Address, RangeList};
use kuna_base::error::{KunaError, KunaResult};
use kuna_base::space::AddrSpace;
use kuna_base::types::Wrap;
use kuna_sleigh::loadimage::{section_flags, LoadImage, LoadImageFunc, LoadImageSection};

use crate::loader::le::read_u32;

/// The SLEIGH language of a 3DS module (the ARM11).
pub const ARCH: &str = "ARM:LE:32:v6:default";

/// Where 3DS titles load their static module (`code.bin`), the base an anonymous
/// import's segment offset is read against for `--symbols`.
pub const STATIC_MODULE_BASE: u64 = 0x10_0000;

const HEADER: usize = 0x80;
const HEADER_END: usize = 0x138;
const ABSENT: u32 = 0xffff_ffff;
const BX_LR: u32 = 0xe12f_ff1e;

/// Does `bytes` start a CRO or CRS module?
pub fn is_cro(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER_END && matches!(&bytes[HEADER..HEADER + 4], b"CRO0" | b"FIXD" | b"CRS0")
}

#[derive(Debug, Clone)]
struct Section {
    vma: u64,
    size: u64,
    flags: u32,
    data: Vec<u8>,
}

/// A name the module defines or imports, with its address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CroSymbol {
    pub name: String,
    pub addr: u64,
    /// In an executable section.
    pub code: bool,
}

pub struct CroLoadImage {
    filename: String,
    sections: Vec<Section>,
    symbols: Vec<CroSymbol>,
    code_pointers: Vec<u64>,
    spaceid: Option<Rc<AddrSpace>>,
    cursection: Cell<usize>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    name: &'a str,
}

impl Reader<'_> {
    fn u32(&self, at: usize) -> KunaResult<u32> {
        read_u32(self.bytes, at).ok_or_else(|| self.invalid(&format!("truncated at {at:#x}")))
    }

    fn invalid(&self, why: &str) -> KunaError {
        KunaError::lowlevel(format!("{}: invalid CRO module: {why}", self.name))
    }

    fn string(&self, at: usize) -> KunaResult<String> {
        let tail = self.bytes.get(at..).ok_or_else(|| self.invalid("string offset out of range"))?;
        let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        Ok(String::from_utf8_lossy(&tail[..end]).into_owned())
    }
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    offset: u32,
    size: u32,
    kind: u32,
}

const TYPE_TEXT: u32 = 0;
const TYPE_RODATA: u32 = 1;
const TYPE_DATA: u32 = 2;

const H_SIZE: usize = 0x90;
const H_BSS: usize = 0x94;
const H_CONTROL: usize = 0xa0;
const H_ONLOAD: usize = 0xa4;
const H_ONEXIT: usize = 0xa8;
const H_ONUNRESOLVED: usize = 0xac;
const H_SEGMENTS: usize = 0xc8;
const H_NAMED_EXPORTS: usize = 0xd0;
const H_IMPORT_MODULES: usize = 0xf0;
const H_IMPORT_RELOCS: usize = 0xf8;
const H_NAMED_IMPORTS: usize = 0x100;
const H_INDEXED_IMPORTS: usize = 0x108;
const H_ANON_IMPORTS: usize = 0x110;
const H_INTERNAL_RELOCS: usize = 0x128;

const R_ABS32: u8 = 2;
const R_REL32: u8 = 3;
const R_THM_PC22: u8 = 10;
const R_CALL: u8 = 28;
const R_JUMP24: u8 = 29;
const R_TARGET1: u8 = 38;
const R_PREL31: u8 = 42;

struct Module<'a> {
    rd: &'a Reader<'a>,
    segments: Vec<Segment>,
    image: Vec<u8>,
}

impl Module<'_> {
    fn base_of(&self, index: usize) -> Option<u64> {
        self.segments.get(index).map(|s| u64::from(s.offset))
    }

    /// The file offset a segment-offset word (`index | offset << 4`) names.
    fn locate(&self, value: u32) -> Option<usize> {
        usize::try_from(self.base_of((value & 0xf) as usize)? + u64::from(value >> 4)).ok()
    }

    fn kind_at(&self, value: u32) -> Option<u32> {
        self.segments.get((value & 0xf) as usize).map(|s| s.kind)
    }

    /// Apply one relocation of `kind` at segment offset `place`, pointing at
    /// `target + addend`. A kind or place it cannot apply is skipped.
    fn apply(&mut self, place: u32, kind: u8, target: u64, addend: i32) {
        let Some(offset) = self.locate(place) else { return };
        let Some(word) = self.image.get_mut(offset..offset + 4) else { return };
        let value = target.wrapping_add(addend as i64 as u64) as u32;
        let pc = offset as u32;
        let old = u32::from_le_bytes(word.try_into().unwrap());
        let new = match kind {
            R_ABS32 | R_TARGET1 => value,
            R_REL32 => value.wrapping_sub(pc),
            R_PREL31 => (old & 0x8000_0000) | (value.wrapping_sub(pc) & 0x7fff_ffff),
            R_CALL | R_JUMP24 => (old & 0xff00_0000) | ((value.wrapping_sub(pc + 8) >> 2) & 0x00ff_ffff),
            R_THM_PC22 => {
                let delta = value.wrapping_sub(pc + 4);
                let hi = (old as u16 & 0xf800) | ((delta >> 12) & 0x7ff) as u16;
                let lo = ((old >> 16) as u16 & 0xf800) | ((delta >> 1) & 0x7ff) as u16;
                u32::from(hi) | (u32::from(lo) << 16)
            }
            _ => return,
        };
        word.copy_from_slice(&new.to_le_bytes());
    }
}

impl CroLoadImage {
    pub fn from_bytes(filename: &str, bytes: &[u8]) -> KunaResult<Self> {
        Self::build(filename, bytes, kuna_decomp::kuna_crorelocs::crorelocs_enabled())
    }

    pub fn build(filename: &str, bytes: &[u8], relocate: bool) -> KunaResult<Self> {
        let rd = Reader { bytes, name: filename };
        if !is_cro(bytes) {
            return Err(rd.invalid("missing CRO0/FIXD/CRS0 magic at 0x80"));
        }
        let file_size = rd.u32(H_SIZE)? as usize;
        let bss_size = u64::from(rd.u32(H_BSS)?);
        if file_size > bytes.len() || file_size < HEADER_END {
            return Err(rd.invalid("file size field disagrees with the file"));
        }

        let (seg_at, seg_n) = (rd.u32(H_SEGMENTS)? as usize, rd.u32(H_SEGMENTS + 4)? as usize);
        if seg_n > 16 {
            return Err(rd.invalid("more than 16 segments"));
        }
        let mut segments = Vec::new();
        for i in 0..seg_n {
            let at = seg_at + 12 * i;
            segments.push(Segment { offset: rd.u32(at)?, size: rd.u32(at + 4)?, kind: rd.u32(at + 8)? });
        }
        let mut module = Module { rd: &rd, segments, image: bytes[..file_size].to_vec() };

        let bss_vma = file_size as u64;
        let extern_vma = (bss_vma + bss_size).next_multiple_of(16);
        let mut symbols: Vec<CroSymbol> = Vec::new();
        let mut add_symbol = |module: &Module, name: String, value: u32| {
            if let Some(addr) = module.locate(value) {
                if !symbols.iter().any(|s| s.addr == addr as u64) {
                    let code = module.kind_at(value) == Some(TYPE_TEXT);
                    symbols.push(CroSymbol { name, addr: addr as u64, code });
                }
            }
        };
        let (exp_at, exp_n) = (rd.u32(H_NAMED_EXPORTS)? as usize, rd.u32(H_NAMED_EXPORTS + 4)? as usize);
        for i in 0..exp_n.min(0x10000) {
            let name = rd.string(rd.u32(exp_at + 8 * i)? as usize)?;
            add_symbol(&module, name, rd.u32(exp_at + 8 * i + 4)?);
        }
        for (label, at) in [
            ("nnroControlObject_", H_CONTROL),
            ("OnLoad", H_ONLOAD),
            ("OnExit", H_ONEXIT),
            ("OnUnresolved", H_ONUNRESOLVED),
        ] {
            let value = rd.u32(at)?;
            if value != ABSENT {
                add_symbol(&module, label.to_string(), value);
            }
        }

        let supplied: std::collections::HashMap<u64, String> =
            crate::symbolnames::from_env().into_iter().collect();
        let mut used: std::collections::HashSet<u64> = std::collections::HashSet::new();
        let mut stubs: Vec<(String, u64)> = Vec::new();
        let mut code_pointers: Vec<u64> = Vec::new();
        if relocate {
            let (rel_at, rel_n) = (rd.u32(H_INTERNAL_RELOCS)? as usize, rd.u32(H_INTERNAL_RELOCS + 4)? as usize);
            for i in 0..rel_n.min(0x100000) {
                let at = rel_at + 12 * i;
                let (place, kind, seg, addend) = (rd.u32(at)?, bytes[at + 4], bytes[at + 5], rd.u32(at + 8)? as i32);
                if let Some(target) = module.base_of(usize::from(seg)) {
                    module.apply(place, kind, target, addend);
                    let into_text = module.segments.get(usize::from(seg)).map(|s| s.kind) == Some(TYPE_TEXT);
                    let from_text = module.kind_at(place) == Some(TYPE_TEXT);
                    if matches!(kind, R_ABS32 | R_TARGET1) && into_text && !from_text {
                        code_pointers.push(target.wrapping_add(addend as i64 as u64));
                    }
                }
            }

            let import_relocs = rd.u32(H_IMPORT_RELOCS)? as usize;
            let mut patch_list = |module: &mut Module, name: String, head: u32| -> KunaResult<()> {
                let addr = match stubs.iter().find(|(n, _)| *n == name) {
                    Some((_, addr)) => *addr,
                    None => {
                        let addr = extern_vma + 4 * stubs.len() as u64;
                        stubs.push((name, addr));
                        addr
                    }
                };
                let mut at = head as usize;
                let _ = import_relocs;
                for _ in 0..0x10000 {
                    let (place, kind, last, addend) = (rd.u32(at)?, bytes[at + 4], bytes[at + 5], rd.u32(at + 8)? as i32);
                    module.apply(place, kind, addr, addend);
                    if last != 0 {
                        break;
                    }
                    at += 12;
                }
                Ok(())
            };
            let (named_at, named_n) = (rd.u32(H_NAMED_IMPORTS)? as usize, rd.u32(H_NAMED_IMPORTS + 4)? as usize);
            let import_strings = |at: u32| rd.string(at as usize);
            for i in 0..named_n.min(0x10000) {
                let name = import_strings(rd.u32(named_at + 8 * i)?)?;
                patch_list(&mut module, name, rd.u32(named_at + 8 * i + 4)?)?;
            }
            let (mod_at, mod_n) = (rd.u32(H_IMPORT_MODULES)? as usize, rd.u32(H_IMPORT_MODULES + 4)? as usize);
            let _ = (rd.u32(H_INDEXED_IMPORTS)?, rd.u32(H_ANON_IMPORTS)?);
            for m in 0..mod_n.min(0x1000) {
                let base = mod_at + 20 * m;
                let module_name = rd.string(rd.u32(base)? as usize)?.trim_matches('|').to_string();
                let (idx_at, idx_n) = (rd.u32(base + 4)? as usize, rd.u32(base + 8)? as usize);
                let (anon_at, anon_n) = (rd.u32(base + 12)? as usize, rd.u32(base + 16)? as usize);
                for i in 0..idx_n.min(0x10000) {
                    let index = rd.u32(idx_at + 8 * i)?;
                    patch_list(&mut module, format!("import_{module_name}_{index}"), rd.u32(idx_at + 8 * i + 4)?)?;
                }
                for i in 0..anon_n.min(0x10000) {
                    let target = rd.u32(anon_at + 8 * i)?;
                    let name = if target & 0xf == 0 {
                        let address = STATIC_MODULE_BASE + u64::from(target >> 4);
                        match supplied.get(&address) {
                            Some(name) => {
                                used.insert(address);
                                name.clone()
                            }
                            None => format!("import_{module_name}_{address:x}"),
                        }
                    } else {
                        format!("import_{module_name}_{}_{:x}", target & 0xf, target >> 4)
                    };
                    patch_list(&mut module, name, rd.u32(anon_at + 8 * i + 4)?)?;
                }
            }
        }

        for seg in module.segments.iter().filter(|s| s.kind == TYPE_TEXT) {
            let (start, end) = (seg.offset as usize, (seg.offset + seg.size) as usize);
            for at in (start..end.min(module.image.len())).step_by(4) {
                let Some(word) = module.image.get(at..at + 4) else { break };
                let word = u32::from_le_bytes(word.try_into().unwrap());
                if word & 0xff7f_f000 != 0xe51f_f000 {
                    continue;
                }
                let delta = (word & 0xfff) as usize;
                let pool = if word & 0x0080_0000 != 0 { at + 8 + delta } else { (at + 8).wrapping_sub(delta) };
                let Some(slot) = module.image.get(pool..pool + 4) else { continue };
                let target = u64::from(u32::from_le_bytes(slot.try_into().unwrap()));
                if let Some((name, _)) = stubs.iter().find(|(_, addr)| *addr == target) {
                    if !symbols.iter().any(|sym| sym.addr == at as u64) {
                        symbols.push(CroSymbol { name: name.clone(), addr: at as u64, code: true });
                    }
                }
            }
        }

        for (addr, name) in &supplied {
            if let Some(symbol) = symbols.iter_mut().find(|s| s.addr == *addr) {
                symbol.name = name.clone();
                used.insert(*addr);
            } else if module
                .segments
                .iter()
                .any(|s| s.kind == TYPE_TEXT && *addr >= u64::from(s.offset) && *addr < u64::from(s.offset + s.size))
            {
                symbols.push(CroSymbol { name: name.clone(), addr: *addr, code: true });
                used.insert(*addr);
            }
        }
        crate::symbolnames::report_unmatched(filename, supplied.keys().copied(), |a| used.contains(&a));

        let mut sections = Vec::new();
        for seg in &module.segments {
            let flags = match seg.kind {
                TYPE_TEXT => section_flags::CODE | section_flags::READONLY,
                TYPE_RODATA => section_flags::DATA | section_flags::READONLY,
                TYPE_DATA => section_flags::DATA,
                _ => continue,
            };
            let (start, end) = (seg.offset as usize, seg.offset as usize + seg.size as usize);
            if seg.size == 0 || end > module.image.len() {
                continue;
            }
            sections.push(Section {
                vma: u64::from(seg.offset),
                size: u64::from(seg.size),
                flags,
                data: module.image[start..end].to_vec(),
            });
        }
        if bss_size != 0 {
            sections.push(Section { vma: bss_vma, size: bss_size, flags: section_flags::DATA, data: Vec::new() });
        }
        if !stubs.is_empty() {
            let data: Vec<u8> = stubs.iter().flat_map(|_| BX_LR.to_le_bytes()).collect();
            sections.push(Section {
                vma: extern_vma,
                size: data.len() as u64,
                flags: section_flags::CODE | section_flags::READONLY,
                data,
            });
            for (name, addr) in &stubs {
                symbols.push(CroSymbol { name: name.clone(), addr: *addr, code: true });
            }
        }
        if sections.iter().all(|s| s.flags & section_flags::CODE == 0) {
            return Err(rd.invalid("no executable segment"));
        }
        Ok(CroLoadImage {
            filename: filename.to_string(),
            sections,
            symbols,
            code_pointers,
            spaceid: None,
            cursection: Cell::new(0),
        })
    }

    pub fn attach_to_space(&mut self, id: Rc<AddrSpace>) {
        self.spaceid = Some(id);
    }

    /// The exports and import stubs, with their addresses.
    pub fn symbols(&self) -> &[CroSymbol] {
        &self.symbols
    }

    /// Where a relocation outside the code points into it: a vtable or callback
    /// table word, so the start of a function. The Thumb bit, if set, is kept.
    pub fn code_pointers(&self) -> &[u64] {
        &self.code_pointers
    }

    /// `(start, size)` of each executable section.
    pub fn executable_ranges(&self) -> Vec<(u64, u64)> {
        self.sections
            .iter()
            .filter(|s| s.flags & section_flags::CODE != 0)
            .map(|s| (s.vma, s.size))
            .collect()
    }
}

impl LoadImage for CroLoadImage {
    fn get_file_name(&self) -> &str {
        &self.filename
    }

    fn load_fill(&mut self, ptr: &mut [u8], addr: &Address) -> KunaResult<()> {
        let mut cursor = addr.get_offset();
        let mut written = 0usize;
        while written < ptr.len() {
            if let Some(s) = self.sections.iter().find(|s| cursor >= s.vma && cursor - s.vma < s.size) {
                let at = (cursor - s.vma) as usize;
                let count = ((s.size - (cursor - s.vma)) as usize).min(ptr.len() - written);
                let backed = s.data.get(at..).unwrap_or(&[]);
                let n = backed.len().min(count);
                ptr[written..written + n].copy_from_slice(&backed[..n]);
                ptr[written + n..written + count].fill(0);
                cursor = cursor.wadd(count as u64);
                written += count;
                continue;
            }
            if written == 0 {
                let mut message = format!("Unable to load {} bytes at {}", ptr.len(), addr.get_shortcut());
                addr.print_raw(&mut message)?;
                return Err(KunaError::data_unavail(message));
            }
            let gap = self
                .sections
                .iter()
                .filter_map(|s| (s.vma > cursor).then(|| s.vma - cursor))
                .min()
                .unwrap_or((ptr.len() - written) as u64)
                .min((ptr.len() - written) as u64) as usize;
            ptr[written..written + gap].fill(0);
            cursor = cursor.wadd(gap as u64);
            written += gap;
        }
        Ok(())
    }

    fn open_symbols(&self) {}

    fn get_next_symbol(&self, _record: &mut LoadImageFunc) -> bool {
        false
    }

    fn open_section_info(&self) {
        self.cursection.set(0);
    }

    fn get_next_section(&self, record: &mut LoadImageSection) -> bool {
        let index = self.cursection.get();
        let Some(section) = self.sections.get(index) else {
            return false;
        };
        let space = self.spaceid.as_ref().expect("CroLoadImage::getNextSection before attachToSpace");
        record.address = Address::new(Rc::clone(space), section.vma);
        record.size = section.size;
        record.flags = section.flags;
        self.cursection.set(index + 1);
        index + 1 < self.sections.len()
    }

    fn get_segments(&self) -> Vec<(u64, u64, u32)> {
        self.sections.iter().map(|s| (s.vma, s.size, s.flags)).collect()
    }

    fn get_readonly(&self, list: &mut RangeList) {
        let Some(space) = self.spaceid.as_ref() else {
            return;
        };
        for s in &self.sections {
            if s.flags & section_flags::READONLY != 0 && s.size != 0 {
                list.insert_range(Rc::clone(space), s.vma, s.vma.wadd(s.size).wsub(1));
            }
        }
    }

    fn get_arch_type(&self) -> Vec<u8> {
        ARCH.as_bytes().to_vec()
    }

    fn adjust_vma(&mut self, adjust: i64) {
        let space = self.spaceid.as_ref().expect("CroLoadImage::adjustVma before attachToSpace");
        let byte_adjust = AddrSpace::address_to_byte(adjust as u64, space.get_word_size());
        for s in &mut self.sections {
            s.vma = s.vma.wadd(byte_adjust);
        }
        for sym in &mut self.symbols {
            sym.addr = sym.addr.wadd(byte_adjust);
        }
    }
}

/// A small synthetic CRO for tests: `.text` at file 0x200, `.rodata` at 0x300, `.data`
/// at 0x310; an export at TEXT+0x14; a thunk at TEXT that is `ldr pc,[pc,#8]` through the
/// patched word at TEXT+0x10 (one anonymous import); the export at TEXT+0x14 is `cmp r0,#3; ldrcc pc,[pc,r0,lsl #2]`
/// over a jump table at TEXT+0x20 of three zero words that internal relocations point at
/// TEXT+0x30, 0x34 and 0x38, each a branch to a `mov r0,#10/20/30; bx lr` body; and a word at the start of `.rodata`
/// relocated to TEXT+0x40, a code pointer from outside the code.
#[doc(hidden)]
pub mod synthetic {
    pub const TEXT: usize = 0x200;
    pub const FILE_SIZE: usize = 0x1000;
    pub const BSS: u32 = 0x20;
    pub const IMPORT_TARGET: u32 = 0x1230;

    fn put(image: &mut [u8], at: usize, value: u32) {
        image[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn reloc(image: &mut [u8], at: usize, place: u32, seg: u8, last: u8, addend: u32) {
        put(image, at, place);
        image[at + 4] = 2;
        image[at + 5] = seg;
        image[at + 6] = last;
        put(image, at + 8, addend);
    }

    pub fn module() -> Vec<u8> {
        let mut image = vec![0u8; FILE_SIZE];
        image[0x80..0x84].copy_from_slice(b"CRO0");
        put(&mut image, 0x90, FILE_SIZE as u32);
        put(&mut image, 0x94, BSS);
        for at in [0xa0, 0xa4, 0xa8, 0xac] {
            put(&mut image, at, 0xffff_ffff);
        }
        put(&mut image, 0xc8, 0x400);
        put(&mut image, 0xcc, 3);
        put(&mut image, 0xd0, 0x430);
        put(&mut image, 0xd4, 1);
        put(&mut image, 0xf0, 0x440);
        put(&mut image, 0xf4, 1);
        put(&mut image, 0xf8, 0x460);
        put(&mut image, 0xfc, 1);
        put(&mut image, 0x110, 0x480);
        put(&mut image, 0x114, 1);
        put(&mut image, 0x128, 0x4a0);
        put(&mut image, 0x12c, 4);
        for (i, (offset, size, kind)) in [(TEXT as u32, 0x60u32, 0u32), (0x300, 0x10, 1), (0x310, 0x10, 2)].iter().enumerate() {
            put(&mut image, 0x400 + 12 * i, *offset);
            put(&mut image, 0x400 + 12 * i + 4, *size);
            put(&mut image, 0x400 + 12 * i + 8, *kind);
        }
        image[0x500..0x507].copy_from_slice(b"Export\0");
        image[0x520..0x528].copy_from_slice(b"|static\0");
        put(&mut image, 0x430, 0x500);
        put(&mut image, 0x434, 0x14 << 4);
        put(&mut image, 0x440, 0x520);
        put(&mut image, 0x44c, 0x480);
        put(&mut image, 0x450, 1);
        reloc(&mut image, 0x460, 0x10 << 4, 0, 1, 0);
        put(&mut image, 0x480, IMPORT_TARGET);
        put(&mut image, 0x484, 0x460);
        put(&mut image, TEXT, 0xe59f_f008);
        let code: [(usize, u32); 15] = [
            (0x14, 0xe350_0003),
            (0x18, 0x379f_f100),
            (0x1c, 0xea00_000d),
            (0x30, 0xea00_0002),
            (0x34, 0xea00_0003),
            (0x38, 0xea00_0004),
            (0x40, 0xe3a0_000a),
            (0x44, 0xe12f_ff1e),
            (0x48, 0xe3a0_0014),
            (0x4c, 0xe12f_ff1e),
            (0x50, 0xe3a0_001e),
            (0x54, 0xe12f_ff1e),
            (0x58, 0xe3a0_0063),
            (0x5c, 0xe12f_ff1e),
            (0x2c, 0),
        ];
        for (at, word) in code {
            put(&mut image, TEXT + at, word);
        }
        for (i, addend) in [0x30u32, 0x34, 0x38].iter().enumerate() {
            reloc(&mut image, 0x4a0 + 12 * i, (0x20 + 4 * i as u32) << 4, 0, 0, *addend);
        }
        reloc(&mut image, 0x4a0 + 36, 1, 0, 0, 0x40);
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(image: &CroLoadImage, vma: u64) -> u32 {
        let section = image.sections.iter().find(|s| vma >= s.vma && vma < s.vma + s.size).unwrap();
        let at = (vma - section.vma) as usize;
        u32::from_le_bytes(section.data[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn relocations_fill_the_jump_table_and_point_the_import_word_at_a_stub() {
        let image = CroLoadImage::build("m.cro", &synthetic::module(), true).unwrap();
        let text = synthetic::TEXT as u64;
        assert_eq!(
            [word(&image, text + 0x20), word(&image, text + 0x24), word(&image, text + 0x28)],
            [0x230, 0x234, 0x238]
        );
        let stub = image.symbols().iter().find(|s| s.name == "import_static_100123" && s.addr >= 0x1020).unwrap();
        assert_eq!(u64::from(word(&image, text + 0x10)), stub.addr);
        assert!(image.symbols().iter().any(|s| s.name == "import_static_100123" && s.addr == text));
    }

    #[test]
    fn a_relocated_word_outside_the_code_is_a_code_pointer_and_a_jump_table_is_not() {
        let image = CroLoadImage::build("m.cro", &synthetic::module(), true).unwrap();
        assert_eq!(image.code_pointers(), &[0x240]);
        assert!(CroLoadImage::build("m.cro", &synthetic::module(), false).unwrap().code_pointers().is_empty());
    }

    #[test]
    fn without_relocations_the_table_stays_zero() {
        let image = CroLoadImage::build("m.cro", &synthetic::module(), false).unwrap();
        let text = synthetic::TEXT as u64;
        assert_eq!([word(&image, text + 0x20), word(&image, text + 0x24)], [0, 0]);
        assert!(image.symbols().iter().all(|s| !s.name.starts_with("import_")));
    }

    #[test]
    fn the_export_is_a_code_symbol_and_a_non_cro_is_refused() {
        let image = CroLoadImage::build("m.cro", &synthetic::module(), true).unwrap();
        assert!(image.symbols().iter().any(|s| s.name == "Export" && s.addr == 0x214 && s.code));
        assert!(!is_cro(&[0u8; 0x200]));
        assert!(CroLoadImage::build("x", &[0u8; 0x200], true).is_err());
    }
}
