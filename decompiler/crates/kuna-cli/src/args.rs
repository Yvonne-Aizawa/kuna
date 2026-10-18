//! Shared argument consumption for the CLI's command-specific parsers.

pub(crate) fn take_value(argv: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let value = argv
        .get(*i + 1)
        .ok_or_else(|| format!("{flag} requires a value"))?;
    *i += 1;
    Ok(value.clone())
}

pub(crate) fn report(result: Result<i32, String>) -> i32 {
    match result {
        Ok(status) => status,
        Err(error) => {
            eprintln!("error: {error}");
            2
        }
    }
}

pub(crate) fn take_option(argv: &[String], i: &mut usize) -> Result<(String, String), String> {
    if *i + 2 >= argv.len() {
        return Err("--option requires NAME VALUE".into());
    }
    let name = &argv[*i + 1];
    crate::optname::check(name)?;
    let value = &argv[*i + 2];
    *i += 2;
    Ok((name.clone(), value.clone()))
}

/// `--symbols FILE`: check the file and hand its path to the loaders through
/// [`kuna_analysis::symbolnames::SYMBOLS_ENV`].
pub(crate) fn use_symbols(path: &str) -> Result<(), String> {
    let absolute = std::fs::canonicalize(path).map_err(|e| format!("--symbols {path}: {e}"))?;
    let names = kuna_analysis::symbolnames::read(&absolute).map_err(|e| format!("--symbols: {e}"))?;
    let mut seen: std::collections::BTreeMap<u64, &str> = std::collections::BTreeMap::new();
    for (addr, name) in &names {
        if let Some(first) = seen.insert(*addr, name.as_str()).filter(|first| first != name) {
            eprintln!("[kuna symbols] {path}: {addr:#x} is named both {first} and {name}; {name} wins");
        }
    }
    std::env::set_var(kuna_analysis::symbolnames::SYMBOLS_ENV, absolute);
    Ok(())
}
