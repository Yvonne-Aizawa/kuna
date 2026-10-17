//! Nintendo 3DS NCCH extended header, for loading a decrypted ExeFS `.code` image.
//!
//! `--exheader <path>` turns a flat `.code` file into a raw image whose layout comes
//! from the header's three code-segment records (address, page count, byte size):
//! the image is ARMv6K little-endian, based at the `.text` address, entered at its
//! start, with `.text` and `.rodata` declared read-only so literal-pool loads fold
//! to the globals they name. `.data` stays writable. Only a decrypted and
//! decompressed `.code` whose three segments are contiguous is accepted.

use kuna_console::assertions::Directive;
use kuna_console::engine::ArmIsa;

pub(crate) const TARGET: &str = "ARM:LE:32:v6:default";

const PAGE: u64 = 0x1000;
const HEADER_LEN: usize = 0x40;
const TEXT_AT: usize = 0x10;
const RODATA_AT: usize = 0x20;
const DATA_AT: usize = 0x30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Segment {
    address: u64,
    pages: u64,
    size: u64,
}

impl Segment {
    fn read(bytes: &[u8], at: usize, name: &str) -> Result<Self, String> {
        let word = |off: usize| u64::from(u32::from_le_bytes(bytes[at + off..at + off + 4].try_into().unwrap()));
        let seg = Segment { address: word(0), pages: word(4), size: word(8) };
        if seg.address % PAGE != 0 {
            return Err(format!("exheader {name} address {:#x} is not page aligned", seg.address));
        }
        if seg.size > seg.pages * PAGE {
            return Err(format!(
                "exheader {name} size {:#x} exceeds its {} pages",
                seg.size, seg.pages
            ));
        }
        Ok(seg)
    }

    fn end(&self) -> u64 {
        self.address + self.pages * PAGE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    text: Segment,
    rodata: Segment,
    data: Segment,
}

impl Layout {
    pub(crate) fn base(&self) -> u64 {
        self.text.address
    }

    fn vtable_spec(&self) -> String {
        let range = |seg: Segment| format!("{:#x}+{:#x}", seg.address, seg.pages * PAGE);
        format!("{}:{},{}", range(self.text), range(self.rodata), range(self.data))
    }

    fn readonly_specs(&self) -> [String; 2] {
        [self.text, self.rodata]
            .map(|seg| format!("readonly {:#x}+{:#x}", seg.address, seg.pages * PAGE))
    }
}

pub(crate) fn parse(bytes: &[u8], image_len: u64) -> Result<Layout, String> {
    if bytes.len() < HEADER_LEN {
        return Err(format!(
            "exheader is {} bytes, shorter than the {HEADER_LEN}-byte system control info",
            bytes.len()
        ));
    }
    let text = Segment::read(bytes, TEXT_AT, ".text")?;
    let rodata = Segment::read(bytes, RODATA_AT, ".rodata")?;
    let data = Segment::read(bytes, DATA_AT, ".data")?;
    if text.address == 0 || text.pages == 0 {
        return Err("exheader declares an empty .text segment".into());
    }
    if rodata.address != text.end() || data.address != rodata.end() {
        return Err(format!(
            "exheader segments are not contiguous (.text {:#x}-{:#x}, .rodata {:#x}, .data {:#x}); \
             only a single flat .code image is supported",
            text.address,
            text.end(),
            rodata.address,
            data.address
        ));
    }
    let needed = data.address - text.address + data.size;
    if image_len < needed {
        return Err(format!(
            "the code image is {image_len:#x} bytes but the exheader needs {needed:#x}; \
             it is not the matching decrypted and decompressed .code"
        ));
    }
    Ok(Layout { text, rodata, data })
}

/// The `rawvtables` value for this layout: pointers into `.text` read from
/// `.rodata` and `.data`.
pub(crate) fn rawvtables_value(layout: &Layout) -> String {
    layout.vtable_spec()
}

pub(crate) fn read(exheader: &str, binary: &str) -> Result<Layout, String> {
    let header = std::fs::read(exheader).map_err(|e| format!("cannot read {exheader}: {e}"))?;
    let image_len = std::fs::metadata(binary)
        .map_err(|e| format!("cannot read {binary}: {e}"))?
        .len();
    parse(&header, image_len)
}

/// Fill the raw-image fields the exheader determines. An explicit `--target` or
/// `--isa` wins; a `--base` that disagrees with the header is an error.
pub(crate) fn apply(
    layout: &Layout,
    target: &mut Option<String>,
    base: &mut Option<u64>,
    isa: &mut Option<ArmIsa>,
    assertions: &mut Vec<Directive>,
) -> Result<Vec<String>, String> {
    if target.as_deref().is_none_or(|t| t.trim().is_empty()) {
        *target = Some(TARGET.into());
    }
    match base {
        Some(b) if *b != layout.base() => {
            return Err(format!(
                "--base {b:#x} contradicts the exheader's .text address {:#x}",
                layout.base()
            ));
        }
        _ => *base = Some(layout.base()),
    }
    isa.get_or_insert(ArmIsa::Arm);
    let specs = layout.readonly_specs();
    for spec in &specs {
        assertions.extend(crate::assertdecl::parse_flag(spec)?);
    }
    Ok(specs.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(text: [u32; 3], ro: [u32; 3], data: [u32; 3]) -> Vec<u8> {
        let mut b = vec![0u8; 0x40];
        for (at, seg) in [(TEXT_AT, text), (RODATA_AT, ro), (DATA_AT, data)] {
            for (i, w) in seg.iter().enumerate() {
                b[at + i * 4..at + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn contiguous_layout_declares_text_and_rodata_readonly() {
        let h = header([0x100000, 0x43c, 0x43bedc], [0x53c000, 0x6a, 0x69a44], [0x5a6000, 0x45, 0x440c0]);
        let layout = parse(&h, 0x4eb000).unwrap();
        assert_eq!(layout.base(), 0x100000);
        assert_eq!(
            layout.readonly_specs(),
            ["readonly 0x100000+0x43c000", "readonly 0x53c000+0x6a000"]
        );
    }

    #[test]
    fn rejects_gaps_short_images_and_oversized_segments() {
        let gap = header([0x100000, 1, 0x10], [0x200000, 1, 0x10], [0x201000, 1, 0x10]);
        assert!(parse(&gap, 1 << 20).unwrap_err().contains("not contiguous"));
        let ok = header([0x100000, 1, 0x10], [0x101000, 1, 0x10], [0x102000, 1, 0x10]);
        assert!(parse(&ok, 0x1000).unwrap_err().contains("matching decrypted"));
        let big = header([0x100000, 1, 0x2000], [0x101000, 1, 0x10], [0x102000, 1, 0x10]);
        assert!(parse(&big, 1 << 20).unwrap_err().contains("exceeds"));
        assert!(parse(&[0; 8], 0).unwrap_err().contains("shorter"));
    }

    #[test]
    fn explicit_base_must_match_and_target_isa_default() {
        let h = header([0x100000, 1, 0x10], [0x101000, 1, 0x10], [0x102000, 1, 0x10]);
        let layout = parse(&h, 0x2010).unwrap();
        let (mut t, mut b, mut i, mut a) = (None, Some(0x4000), None, Vec::new());
        assert!(apply(&layout, &mut t, &mut b, &mut i, &mut a).is_err());
        let (mut t, mut b, mut i, mut a) = (None, None, None, Vec::new());
        apply(&layout, &mut t, &mut b, &mut i, &mut a).unwrap();
        assert_eq!((t.as_deref(), b, i), (Some(TARGET), Some(0x100000), Some(ArmIsa::Arm)));
        assert_eq!(a.len(), 2);
    }
}
