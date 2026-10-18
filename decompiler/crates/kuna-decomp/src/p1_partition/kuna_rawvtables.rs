//! (kuna `rawvtables`) Function pointers in the data of a headerless ARM32 image.
//!
//! A raw image has no symbols, and the direct-call sweep of `rawdiscover` cannot
//! see a method reached only through a vtable or a callback table: nothing in the
//! code names its address. The pointers are in the data, as 32-bit words pointing
//! into the code. `--option rawvtables CODE:SCAN[,SCAN]...` names the code range
//! the pointers must land in and the data ranges to read, each as `start+size` in
//! hex, and every run of [`MIN_RUN`] or more consecutive pointer words there seeds
//! its targets as function entries. One word on its own proves little (a constant
//! that happens to look like an address); a run is what a table is.
//!
//! A word is a pointer when its target, with the Thumb bit cleared, lies in the
//! code range. An even, word-aligned target is an A32 entry; an odd one is a Thumb
//! entry (the interworking convention), left to `rawthumb` to decode in its mode;
//! an even target that is only halfword-aligned is neither and ends the run.

use kuna_base::error::{KunaError, KunaResult};

/// The shortest run of consecutive pointer words taken for a table.
pub const MIN_RUN: usize = 3;

/// The ranges `rawvtables` reads: where pointers must land, and where to look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawVtableSpec {
    /// `(start, size)` of the code the pointers must target.
    pub code: (u64, u64),
    /// `(start, size)` of each data range to read.
    pub scans: Vec<(u64, u64)>,
}

/// The function entries a scan found, as even byte addresses.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TableRoots {
    /// Targets with the Thumb bit clear and word alignment.
    pub arm: Vec<u64>,
    /// Targets with the Thumb bit set, bit cleared here.
    pub thumb: Vec<u64>,
}

fn parse_range(text: &str) -> KunaResult<(u64, u64)> {
    let bad = || KunaError::parse(format!("rawvtables range {text:?} is not <start>+<size> in hex"));
    let (start, size) = text.trim().split_once('+').ok_or_else(bad)?;
    let hex = |v: &str| {
        let v = v.trim();
        u64::from_str_radix(v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")).unwrap_or(v), 16)
            .map_err(|_| bad())
    };
    let (start, size) = (hex(start)?, hex(size)?);
    if size == 0 || start.checked_add(size).is_none() {
        return Err(bad());
    }
    Ok((start, size))
}

/// Parse `off` (no spec) or `CODE:SCAN[,SCAN]...`.
pub fn parse(value: &str) -> KunaResult<Option<RawVtableSpec>> {
    let value = value.trim();
    if value == "off" {
        return Ok(None);
    }
    let (code, scans) = value.split_once(':').ok_or_else(|| {
        KunaError::parse("rawvtables expects off or <code>:<scan>[,<scan>...] with <start>+<size> ranges")
    })?;
    let scans = scans.split(',').map(parse_range).collect::<KunaResult<Vec<_>>>()?;
    Ok(Some(RawVtableSpec { code: parse_range(code)?, scans }))
}

/// Read `image` (mapped at `base`) over the spec's scan ranges and collect the
/// targets of every run of pointer words.
pub fn scan(spec: &RawVtableSpec, image: &[u8], base: u64) -> TableRoots {
    let (code_lo, code_len) = spec.code;
    let code_hi = code_lo.saturating_add(code_len);
    let mut roots = TableRoots::default();
    for &(start, size) in &spec.scans {
        let lo = start.max(base).next_multiple_of(4);
        let hi = start.saturating_add(size).min(base + image.len() as u64);
        let mut run: Vec<u32> = Vec::new();
        let mut at = lo;
        while at + 4 <= hi {
            let off = (at - base) as usize;
            let word = u32::from_le_bytes(image[off..off + 4].try_into().unwrap());
            let target = u64::from(word & !1);
            let pointer = target >= code_lo && target < code_hi && (word & 1 == 1 || word % 4 == 0);
            if pointer {
                run.push(word);
            } else {
                flush(&mut run, &mut roots);
            }
            at += 4;
        }
        flush(&mut run, &mut roots);
    }
    roots.arm.sort_unstable();
    roots.arm.dedup();
    roots.thumb.sort_unstable();
    roots.thumb.dedup();
    roots
}

fn flush(run: &mut Vec<u32>, roots: &mut TableRoots) {
    if run.len() >= MIN_RUN {
        for &word in run.iter() {
            let target = u64::from(word & !1);
            if word & 1 == 1 {
                roots.thumb.push(target);
            } else {
                roots.arm.push(target);
            }
        }
    }
    run.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    #[test]
    fn parses_off_and_ranges_and_rejects_the_rest() {
        assert_eq!(parse("off").unwrap(), None);
        let spec = parse("0x1000+0x100:0x2000+0x40,0x3000+0x10").unwrap().unwrap();
        assert_eq!(spec.code, (0x1000, 0x100));
        assert_eq!(spec.scans, vec![(0x2000, 0x40), (0x3000, 0x10)]);
        for bad in ["on", "0x1000+0x100", "0x1000+0:0x2000+4", "x+1:2+3", "0x1000+0x100:"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_run_of_three_pointers_seeds_arm_and_thumb_targets() {
        let words = [0x1000, 0x1011, 0x1020, 0, 0x1000, 0x1010, 0, 0x2000, 0x1002, 0x1030, 0x1040];
        let spec = parse("0x1000+0x100:0x2000+0x2c").unwrap().unwrap();
        let roots = scan(&spec, &image(&words), 0x2000);
        assert_eq!(roots.arm, vec![0x1000, 0x1020]);
        assert_eq!(roots.thumb, vec![0x1010]);
    }

    #[test]
    fn a_short_run_a_halfword_target_and_an_outside_target_seed_nothing() {
        let words = [0x1000, 0x1020, 0, 0x1002, 0x1020, 0x1030, 0x9000, 0x1000, 0x1010];
        let spec = parse("0x1000+0x100:0x2000+0x24").unwrap().unwrap();
        assert_eq!(scan(&spec, &image(&words), 0x2000), TableRoots::default());
    }
}
