//! Names for addresses, from a user-supplied file (`--symbols FILE`).
//!
//! A line holds an address and a name, in either order, separated by spaces,
//! commas, `=`, `:` or `|`: `0x12b284 gfl::Alloc`, `12b284,gfl::Alloc`,
//! `gfl::Alloc = 0x12b284`. Blank lines and lines that hold no address and name
//! are skipped. Loaders that can name an address read the file named by
//! [`SYMBOLS_ENV`]: a raw image names the function there, and a Nintendo 3DS CRO
//! module also names an import by its address in the static module.

use std::path::Path;

/// The variable the CLI sets to the `--symbols` path for the process it loads in.
pub const SYMBOLS_ENV: &str = "KUNA_SYMBOLS";

fn address(token: &str) -> Option<u64> {
    let (digits, prefixed) = match token.strip_prefix("0x").or_else(|| token.strip_prefix("0X")) {
        Some(rest) => (rest, true),
        None => (token, false),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (prefixed || digits.len() >= 3).then(|| u64::from_str_radix(digits, 16).ok()).flatten()
}

fn is_name(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '~' | '<' | '>' | '.' | '$' | '@' | '?'))
        && !token.starts_with(|c: char| c.is_ascii_digit())
}

/// Parse a symbol file: the `(address, name)` pairs and the number of lines that
/// held something else (blank lines and `#`/`//` comments do not count).
pub fn parse(text: &str) -> (Vec<(u64, String)>, usize) {
    let mut names = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let tokens: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | '=' | '|' | '`' | ';'))
            .filter(|t| !t.is_empty())
            .collect();
        let pair = match tokens.as_slice() {
            [a, b] => match (address(a), address(b)) {
                (Some(addr), _) if is_name(b) && !(address(b).is_some() && b.starts_with("0x")) => {
                    Some((addr, b.to_string()))
                }
                (_, Some(addr)) if is_name(a) => Some((addr, a.to_string())),
                _ => None,
            },
            _ => None,
        };
        match pair {
            Some(entry) => names.push(entry),
            None => skipped += 1,
        }
    }
    (names, skipped)
}

/// Read and parse a symbol file.
pub fn read(path: &Path) -> Result<Vec<(u64, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let (names, _) = parse(&text);
    if names.is_empty() {
        return Err(format!("{} holds no `<address> <name>` lines", path.display()));
    }
    Ok(names)
}

/// Say once, on stderr, how many supplied addresses named nothing, with the first few.
pub fn report_unmatched(image: &str, addresses: impl Iterator<Item = u64>, matched: impl Fn(u64) -> bool) {
    let mut all: Vec<u64> = addresses.collect();
    all.sort_unstable();
    let missed: Vec<u64> = all.iter().copied().filter(|a| !matched(*a)).collect();
    if missed.is_empty() {
        return;
    }
    let shown: Vec<String> = missed.iter().take(5).map(|a| format!("{a:#x}")).collect();
    let more = if missed.len() > shown.len() { ", ..." } else { "" };
    kuna_base::notes::say_once(
        image,
        &format!(
            "[kuna symbols] {} of {} symbol(s) named nothing in {image} ({}{more}); check the addresses",
            missed.len(),
            all.len(),
            shown.join(", ")
        ),
    );
}

/// The names in the file [`SYMBOLS_ENV`] points to; empty when it is unset or
/// unreadable (the CLI has already rejected an unreadable file).
pub fn from_env() -> Vec<(u64, String)> {
    std::env::var_os(SYMBOLS_ENV)
        .and_then(|path| read(Path::new(&path)).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_either_order_and_the_usual_separators() {
        let (names, skipped) = parse(
            "# comment\n0x12b284 gfl::Alloc\n12b290,gfl_Free\nmemcpy_ = 0x100200\n| `Init` | 0x100300 |\n\nnonsense line here\n0x10 7\n",
        );
        assert_eq!(
            names,
            vec![
                (0x12b284, "gfl::Alloc".to_string()),
                (0x12b290, "gfl_Free".to_string()),
                (0x100200, "memcpy_".to_string()),
                (0x100300, "Init".to_string()),
            ]
        );
        assert_eq!(skipped, 2);
    }

    #[test]
    fn a_short_bare_number_is_not_an_address() {
        assert_eq!(parse("12 name\n").0, vec![]);
        assert_eq!(parse("0x12 name\n").0, vec![(0x12, "name".to_string())]);
    }
}
