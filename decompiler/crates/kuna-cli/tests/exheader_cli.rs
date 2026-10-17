//! `--exheader` loads a decrypted 3DS `.code` image with the layout its NCCH
//! extended header declares.
use crate::common;

const BASE: u32 = 0x100000;

fn code_image() -> Vec<u8> {
    let mut image = vec![0u8; 0x2020];
    let words: [u32; 4] = [0xe59f0004, 0xe5900000, 0xe12fff1e, BASE + 0x2010];
    let close_handle: [u32; 2] = [0xef000023, 0xe12fff1e];
    for (i, w) in words.iter().chain(&[0, 0, 0, 0]).chain(&close_handle).enumerate() {
        image[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    image[0x40..0x44].copy_from_slice(&[0x07, 0x20, 0x70, 0x47]);
    for (at, value) in [(0x50, 0xe3a00001u32), (0x54, 0xe12fff1e), (0x60, 0xe3a00002), (0x64, 0xe12fff1e)] {
        image[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (i, pointer) in [BASE + 0x50, BASE + 0x60, BASE + 0x41].iter().enumerate() {
        image[0x1000 + i * 4..0x1004 + i * 4].copy_from_slice(&pointer.to_le_bytes());
    }
    for (i, pointer) in [BASE + 0x4, BASE + 0x8, BASE + 0x9].iter().enumerate() {
        image[0x1010 + i * 4..0x1014 + i * 4].copy_from_slice(&pointer.to_le_bytes());
    }
    image
}

fn exheader(data_size: u32) -> Vec<u8> {
    let mut header = vec![0u8; 0x400];
    let segments = [(0x10, BASE, 1, 0x10), (0x20, BASE + 0x1000, 1, 0x10), (0x30, BASE + 0x2000, data_size.div_ceil(0x1000), data_size)];
    for (at, address, pages, size) in segments {
        for (i, w) in [address, pages, size].iter().enumerate() {
            header[at + i * 4..at + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
    header
}

fn write(stem: &str, bytes: &[u8]) -> String {
    let path = common::scratch_file(stem, "bin");
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn specs() -> String {
    common::repo_root().join("specs").to_string_lossy().into_owned()
}

#[test]
fn exheader_maps_text_and_rodata_read_only() {
    let code = write("3ds-code", &code_image());
    let header = write("3ds-exheader", &exheader(0x20));
    let sp = specs();
    let raw = ["--raw-image", "--target", "ARM:LE:32:v6:default", "--base", "0x100000", "--isa", "arm"];

    let mut plain = vec!["decompile", &code, "0x100000"];
    plain.extend(raw);
    plain.extend(["--sleighpath", &sp]);
    let (stdout, stderr, status) = common::run_kuna(&plain);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("*dat_10000c"), "raw image should leave the pool slot: {stdout}");

    let (stdout, stderr, status) =
        common::run_kuna(&["decompile", &code, "0x100000", "--exheader", &header, "--sleighpath", &sp]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("dat_102010") && !stdout.contains("*dat_10000c"), "{stdout}");

    let (stdout, stderr, status) =
        common::run_kuna(&["functions", &code, "--exheader", &header, "--json", "--sleighpath", &sp]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("\"address_hex\": \"0x100000\""), "{stdout}");
}

#[test]
fn exheader_refuses_mismatched_input() {
    let code = write("3ds-code-short", &code_image());
    let sp = specs();
    let big = write("3ds-exheader-big", &exheader(0x8000));
    let (_, stderr, status) =
        common::run_kuna(&["decompile", &code, "0x100000", "--exheader", &big, "--sleighpath", &sp]);
    assert_ne!(status, 0);
    assert!(stderr.contains("matching decrypted"), "{stderr}");

    let header = write("3ds-exheader-base", &exheader(0x20));
    let (_, stderr, status) = common::run_kuna(&[
        "decompile", &code, "0x100000", "--exheader", &header, "--base", "0x200000", "--sleighpath", &sp,
    ]);
    assert_ne!(status, 0);
    assert!(stderr.contains("contradicts the exheader"), "{stderr}");
}

#[test]
fn exheader_names_supervisor_calls() {
    let code = write("3ds-code-svc", &code_image());
    let header = write("3ds-exheader-svc", &exheader(0x20));
    let sp = specs();
    let (stdout, stderr, status) =
        common::run_kuna(&["decompile", &code, "0x100020", "--exheader", &header, "--sleighpath", &sp]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("svcCloseHandle(a0)"), "{stdout}");

    let (stdout, stderr, status) = common::run_kuna(&[
        "decompile", &code, "0x100020", "--exheader", &header, "--option", "ctrsvc", "off", "--sleighpath", &sp,
    ]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("software_interrupt(0x23)"), "{stdout}");
}

#[test]
fn an_odd_address_decodes_that_function_as_thumb() {
    let code = write("3ds-code-thumb", &code_image());
    let header = write("3ds-exheader-thumb", &exheader(0x20));
    let sp = specs();
    let (stdout, stderr, status) =
        common::run_kuna(&["decompile", &code, "0x100041", "--exheader", &header, "--sleighpath", &sp]);
    assert_eq!(status, 0, "{stderr}");
    assert!(stdout.contains("return 7;"), "{stdout}");

    let (stdout, _, _) = common::run_kuna(&[
        "decompile", &code, "0x100041", "--exheader", &header, "--option", "rawthumb", "off", "--sleighpath", &sp,
    ]);
    assert!(!stdout.contains("return 7;"), "{stdout}");
}

#[test]
fn a_pointer_table_in_rodata_seeds_the_functions_it_names() {
    let code = write("3ds-code-vtable", &code_image());
    let header = write("3ds-exheader-vtable", &exheader(0x20));
    let sp = specs();
    let list = |extra: &[&str]| {
        let mut args = vec!["functions", code.as_str(), "--exheader", header.as_str(), "--json", "--sleighpath", sp.as_str()];
        args.extend(extra);
        let (stdout, stderr, status) = common::run_kuna(&args);
        assert_eq!(status, 0, "{stderr}");
        stdout
    };
    let seeded = list(&[]);
    for address in ["0x100050", "0x100060", "0x100040"] {
        assert!(seeded.contains(&format!("\"address_hex\": \"{address}\"")), "{address}: {seeded}");
    }
    assert!(!seeded.contains("\"address_hex\": \"0x100004\""), "mid-function pointer seeded: {seeded}");
    assert!(!seeded.contains("\"address_hex\": \"0x100008\""), "mid-function pointer seeded: {seeded}");
    let plain = list(&["--option", "rawvtables", "off"]);
    assert!(!plain.contains("\"address_hex\": \"0x100060\""), "{plain}");
}
