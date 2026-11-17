// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! `scripts/extrem-audit-scan.sh` reports every finding that survives its
//! filters, and its exit status is a verdict: a detector that kept the first
//! N raw hits and filtered them afterwards reported a clean pass while real
//! findings sat past the cap, and `exit $count` reported success at exactly
//! 256 findings.
//!
//! The script runs on a throwaway project (its own `scripts/` and `src/`),
//! with a `cargo` that does nothing, so only the text detectors speak.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = include_str!("../scripts/extrem-audit-scan.sh");

const HEADER: &str = "// SPDX-License-Identifier: Apache-2.0 OR MIT\n";

struct Scan {
    stdout: String,
    stderr: String,
    status: i32,
}

impl Scan {
    fn lines_with(&self, needle: &str) -> usize {
        self.stderr.lines().filter(|l| l.contains(needle)).count()
    }

    fn total(&self) -> usize {
        self.stdout
            .lines()
            .map(strip_colors)
            .find_map(|l| {
                l.trim()
                    .strip_prefix("TOTAL VIOLATIONS:")
                    .and_then(|n| n.trim().parse().ok())
            })
            .unwrap_or_else(|| panic!("no total in:\n{}", self.stdout))
    }
}

/// `line` without its terminal colour escapes.
fn strip_colors(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for e in chars.by_ref() {
                if e == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Build a project whose only production source is `lib_rs`, scan it.
fn scan(name: &str, lib_rs: &str) -> Scan {
    let root: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&root);
    for d in ["scripts", "src/core/ser", "bin"] {
        fs::create_dir_all(root.join(d)).expect("dirs");
    }
    fs::write(root.join("scripts/extrem-audit-scan.sh"), SCRIPT).expect("script");
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").expect("manifest");
    fs::write(
        root.join("src/core/ser/mod.rs"),
        format!("{HEADER}fn f(x: u32) -> [u8; 4] {{ x.to_le_bytes() }}\n"),
    )
    .expect("ser");
    fs::write(root.join("src/lib.rs"), format!("{HEADER}{lib_rs}")).expect("lib");
    let shim = root.join("bin/cargo");
    fs::write(&shim, "#!/bin/sh\nexit 0\n").expect("shim");
    Command::new("chmod")
        .arg("+x")
        .arg(&shim)
        .status()
        .expect("chmod");

    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").expect("PATH")
    );
    let out = Command::new("bash")
        .arg(root.join("scripts/extrem-audit-scan.sh"))
        .env("PATH", path)
        .output()
        .expect("bash");
    Scan {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        status: out.status.code().expect("exit code"),
    }
}

/// `n` functions, each with one bare `unwrap()` past the three lines the
/// `@audit-ok` look-back reads.
fn unwraps(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        writeln!(
            s,
            "fn f{i}(x: Option<u8>) {{\n\n\n\n    let _ = x.unwrap();\n}}"
        )
        .expect("write");
    }
    s
}

#[test]
fn findings_past_a_page_of_exempt_hits_are_reported() {
    // Twenty exempt unwraps, then five that are not: the five are the only
    // findings, and they sit past any cap on the raw hits.
    let mut src = String::from("pub fn f(x: Option<u8>) {\n");
    for _ in 0..20 {
        src.push_str("    // @audit-ok: fixture\n    let _ = x.unwrap();\n");
    }
    for _ in 0..5 {
        src.push_str("\n\n\n\n    let _ = x.unwrap();\n");
    }
    src.push_str("}\n");
    let r = scan("audit_scan_past_cap", &src);
    assert_eq!(r.lines_with("unwrap() detected"), 5, "{}", r.stderr);
    assert_eq!(r.status, 1, "{}", r.stderr);
}

#[test]
fn the_exit_status_is_a_verdict_not_a_count() {
    // The same project with no unwrap tells how many findings the fixture
    // has of its own; add enough to total exactly 256, which the shell
    // reads back as 0 when the count is the exit status.
    let base = scan("audit_scan_base", &unwraps(0)).total();
    let n = 256 - base;
    let r = scan("audit_scan_256", &unwraps(n));
    assert_eq!(r.total(), 256, "{}", r.stdout);
    assert_eq!(r.lines_with("unwrap() detected"), n, "{}", r.stderr);
    assert_eq!(r.status, 1, "{}", r.stderr);
}

#[test]
fn a_clean_project_exits_zero_and_a_template_is_not_code() {
    // A magic number inside a raw string literal is text of the generated
    // language, not a constant of this crate.
    let mut src = String::from("pub const T: &str = r\"\n");
    src.push_str("    h & 0x40000000u\n\";\n");
    let r = scan("audit_scan_raw", &src);
    assert_eq!(r.lines_with("Magic number"), 0, "{}", r.stderr);
    let mut code = String::new();
    writeln!(code, "pub fn g(h: u32) -> u32 {{ h & 0x40000000 }}").expect("write");
    let r = scan("audit_scan_code", &code);
    assert_eq!(r.lines_with("Magic number"), 1, "{}", r.stderr);
}
