//! SLEIGH compiler passthrough, plus usage and verification guidance.
//!
//! `--diff` describes the independent compiler and decompiler gates without
//! running either. Compilation arguments otherwise pass through to `slacomp`.

use std::io::Read;
use std::process::Stdio;

use crate::paths;

const DIFF_NOTE: &str = "\
kuna specs --diff is informational; it does not run a comparison.

From the repository root, check compiled .sla contents (after decompression)
against pinned Ghidra outputs:
  cargo test --manifest-path decompiler/Cargo.toml --release -p kuna-slacomp --test compiler_parity

The decompiler's behavioral parity gate is separate; it does not establish
compiler output equality:
  make test

The historical live C++ differential is not bundled. See docs/history.md and
decompiler/crates/kuna-slacomp/tests/golden/README.md for provenance.";

pub fn run(args: &[String]) -> i32 {
    // Ahead of the passthrough: slacomp owns no help flag and answers `-h` with
    // `Unknown option` and exit 1, so the alias has to describe itself.
    if args.iter().any(|a| a == "-h" || a == "--help") {
        usage();
        return 0;
    }
    if args.iter().any(|a| a == "--list-targets") {
        return list_targets(args);
    }
    if args.iter().any(|a| a == "--diff") {
        return crate::output::emit_with_status(&format!("{DIFF_NOTE}\n"), 0);
    }
    let compiler = match paths::slacomp() {
        Ok(program) => program,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    // Pass the remaining args straight through to slacomp (it owns `-a <dir>` and
    // the bare `<slaspec>...` forms).
    //
    // slacomp's progress is on stdout and its diagnostics on stderr, and for the
    // unlocated warnings ("1 NOP constructors found") the `Compiling <spec>:` line
    // above them is the only thing naming the spec they came from.  Capturing both
    // streams would print all 1004 warnings of a 148-spec run ahead of all 149
    // progress lines, after 17s of silence.  So stderr stays inherited and stdout
    // is copied through as it arrives: the two interleave as they did when this
    // was a bare `status()` passthrough, and the write still goes through the
    // fallible boundary.
    let mut child = match compiler.command().args(args).stdout(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to run slacomp: {e}");
            return 2;
        }
    };
    let mut pipe = child.stdout.take().expect("stdout was piped above");
    let mut buf = [0u8; 8192];
    let mut write_err = None;
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) if write_err.is_none() => {
                if let Err(e) = crate::output::emit_bytes(&buf[..n]) {
                    write_err = Some(e);
                }
            }
            // Past a write failure the reads continue to EOF rather than stopping:
            // slacomp is writing .sla files, so it is left to finish instead of
            // being killed by the EPIPE that closing this end early would hand it.
            Ok(_) => {}
            Err(e) => {
                write_err = Some(e);
                break;
            }
        }
    }
    // Closing before the wait matters only on the read-error break above, where
    // slacomp may still be writing: without it the child could block on a full
    // pipe that nothing is draining.
    drop(pipe);
    let status = match child.wait() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("error: failed to run slacomp: {e}");
            return 2;
        }
    };
    match write_err {
        Some(err) => crate::output::status_after(err, status),
        None => status,
    }
}

/// Every SLEIGH language id the spec roots define, one per line, from their
/// `.ldefs` files; the optional word after `--list-targets` keeps the ids that
/// contain it (case-insensitively).
fn list_targets(args: &[String]) -> i32 {
    let mut filter: Option<String> = None;
    let mut sleighpath: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--list-targets" => {}
            "--sleighpath" => {
                i += 1;
                sleighpath = args.get(i).cloned();
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown option {other} with --list-targets");
                return 2;
            }
            word => filter = Some(word.to_ascii_lowercase()),
        }
        i += 1;
    }
    let mut ids: Vec<String> = Vec::new();
    for root in crate::decompile_all::spec_roots(sleighpath.as_deref()) {
        collect_ldefs(std::path::Path::new(&root), &mut ids);
    }
    ids.sort();
    ids.dedup();
    if let Some(word) = &filter {
        ids.retain(|id| id.to_ascii_lowercase().contains(word.as_str()));
    }
    if ids.is_empty() {
        eprintln!("error: no SLEIGH language ids found (is the spec directory set? try --sleighpath)");
        return 1;
    }
    let mut text = ids.join("\n");
    text.push('\n');
    crate::output::emit_with_status(&text, 0)
}

fn collect_ldefs(dir: &std::path::Path, ids: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ldefs(&path, ids);
        } else if path.extension().is_some_and(|e| e == "ldefs") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                ids.extend(language_ids(&text));
            }
        }
    }
}

/// The `id="..."` of each `<language ...>` element in an `.ldefs` document.
fn language_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("<language ") {
        rest = &rest[at + "<language ".len()..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        if let Some(start) = tag.find(" id=\"").map(|p| p + 5).or_else(|| tag.strip_prefix("id=\"").map(|_| 4)) {
            if let Some(len) = tag[start..].find('"') {
                ids.push(tag[start..start + len].to_string());
            }
        }
        rest = &rest[end..];
    }
    ids
}

fn usage() {
    eprintln!(
        "usage: kuna specs [-a <dir>] [<file.slaspec>..] [--diff] [--list-targets [WORD] [--sleighpath D]]\n\
         \n\
         Compile SLEIGH processor specifications with the Rust SLEIGH compiler\n\
         (`slacomp`), which this command is a thin alias for -- every other flag is\n\
         passed straight through, and it takes upstream `sleigh_opt`'s CLI.\n\
         \n\
         -a <dir> compiles every .slaspec under <dir> recursively (= make specs over\n\
         specs/); a bare list of files compiles just those.  A .sla is a build\n\
         artifact and is gitignored; the engine finds them under KUNA_SPECS /\n\
         SLEIGHHOME.  A release kuna carries its compiled specs inside itself.\n\
         --list-targets prints every SLEIGH language id the spec directory defines (the\n\
         values --target takes), keeping those that contain WORD, e.g. `arm:le:32`.\n\
         --diff prints guidance for independent compiler and decompiler checks; it runs neither."
    );
}

#[cfg(test)]
mod tests {
    use super::language_ids;

    #[test]
    fn language_ids_reads_each_language_element() {
        let text = "<language_definitions>\n  <language processor=\"ARM\"\n            id=\"ARM:LE:32:v6\">\n  </language>\n  <language id=\"x86:LE:64:default\" processor=\"x86\"/>\n</language_definitions>";
        assert_eq!(language_ids(text), vec!["ARM:LE:32:v6", "x86:LE:64:default"]);
    }
}
