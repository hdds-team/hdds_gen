// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Every backend numbers the enumerators of one IDL the same way.
//!
//! An enumerator with no explicit value takes the previous enumerator's
//! value plus one (the first takes 0), as in C and C++ and as DDS-XTypes 1.3
//! (7.3.1.2.1.5) specifies for enumerators that carry a value next to ones
//! that do not. Two peers generated from the same IDL in different languages
//! must therefore agree on every enumerator, whatever the backend.
//!
//! The IDL mixes explicit and implicit enumerators, a negative explicit
//! value, and an explicit value that moves the counter backwards. The C and
//! C++ backends leave an implicit enumerator without a value for the
//! compiler to number; the test applies the same rule to what they emit, and
//! the enum body is compiled with `static_assert`s when a compiler exists.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IDL: &str = "\
enum Priority { LOW, NORMAL, HIGH = 10, CRITICAL };
enum Signed { NEG = -5, NEXT, ZERO, LAST };
enum Resumed { RA, RB = 5, RC, RD = 2, RE };
struct Msg { Priority p; Signed s; Resumed r; };
";

/// The value of every enumerator: previous plus one, 0 for the first.
const EXPECTED: [(&str, &[(&str, i64)]); 3] = [
    (
        "Priority",
        &[("LOW", 0), ("NORMAL", 1), ("HIGH", 10), ("CRITICAL", 11)],
    ),
    (
        "Signed",
        &[("NEG", -5), ("NEXT", -4), ("ZERO", -3), ("LAST", -2)],
    ),
    (
        "Resumed",
        &[("RA", 0), ("RB", 5), ("RC", 6), ("RD", 2), ("RE", 3)],
    ),
];

/// One directory per backend: the tests run in parallel, and a shared input
/// path would let one test rewrite the file another one is generating from.
fn dir(lang: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("enum_numbering")
        .join(lang);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn generate(lang: &str) -> String {
    let file = dir(lang).join("EnumNumbering.idl");
    fs::write(&file, IDL).expect("idl");
    let out = Command::new(env!("CARGO_BIN_EXE_hddsgen"))
        .args(["gen", lang])
        .arg(&file)
        .output()
        .expect("hddsgen");
    assert!(
        out.status.success(),
        "gen {lang}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

/// The lines of the body of `name`'s enum in the output of `lang`.
fn body<'a>(lang: &str, name: &str, text: &'a str) -> Vec<&'a str> {
    let lines: Vec<&str> = text.lines().collect();
    let start = match lang {
        "rust" | "micro" => format!("pub enum {name} {{"),
        "typescript" => format!("export enum {name} {{"),
        "python" => format!("class {name}(IntEnum):"),
        "cpp" => format!("enum class {name} {{"),
        "c-micro" => format!("enum {name} {{"),
        "c" => {
            let end = lines
                .iter()
                .position(|l| l.trim_start().starts_with(&format!("}} {name}")))
                .unwrap_or_else(|| panic!("c: no `}} {name}`"));
            let open = lines[..end]
                .iter()
                .rposition(|l| l.trim() == "typedef enum {")
                .expect("typedef enum {");
            return lines[open + 1..end].to_vec();
        }
        other => panic!("unknown backend {other}"),
    };
    let open = lines
        .iter()
        .position(|l| l.trim() == start)
        .unwrap_or_else(|| panic!("{lang}: no `{start}`"));
    lines[open + 1..]
        .iter()
        .take_while(|l| !l.trim().is_empty() && !l.trim_start().starts_with('}'))
        .copied()
        .collect()
}

/// `(name, value)` of every enumerator in `lines`; an enumerator written
/// without a value takes the previous one plus one, as a C compiler does.
fn enumerators(lang: &str, name: &str, lines: &[&str]) -> Vec<(String, i64)> {
    let prefix = format!("{}_", name.to_uppercase());
    let mut next: i64 = 0;
    let mut out = Vec::new();
    for line in lines {
        let l = line.trim().trim_end_matches(',');
        if l.is_empty() || l.starts_with("#[") || l.starts_with("//") || l.starts_with("///") {
            continue;
        }
        let (n, v) = match l.split_once(" = ") {
            Some((n, v)) => (n.trim(), Some(v.trim().parse::<i64>().expect("int"))),
            None => (l, None),
        };
        let n = if matches!(lang, "c" | "c-micro") {
            n.strip_prefix(&prefix).unwrap_or(n)
        } else {
            n
        };
        let value = v.unwrap_or(next);
        next = value + 1;
        out.push((n.to_string(), value));
    }
    out
}

fn expected(name: &str) -> Vec<(String, i64)> {
    EXPECTED
        .iter()
        .find(|(n, _)| *n == name)
        .expect("enum")
        .1
        .iter()
        .map(|(n, v)| ((*n).to_string(), *v))
        .collect()
}

fn check_backend(lang: &str) {
    let text = generate(lang);
    for (name, _) in EXPECTED {
        let got = enumerators(lang, name, &body(lang, name, &text));
        assert_eq!(got, expected(name), "{lang}: enum {name}");
    }
}

#[test]
fn rust_numbers_enumerators_in_sequence() {
    check_backend("rust");
    let text = generate("rust");
    // The decoder and the TypeObject carry the same numbers.
    for (name, values) in EXPECTED {
        for (lit, v) in values {
            let arm = format!("{v} => Ok(Self::{lit}),");
            assert!(text.contains(&arm), "rust: enum {name}: no `{arm}`");
            let at = text
                .find(&format!("CompleteMemberDetail::new(\"{lit}\")"))
                .unwrap_or_else(|| panic!("rust: no TypeObject literal {lit}"));
            let tail = &text[..at];
            let value = tail.rfind("value: ").expect("value:");
            let digits: String = tail[value + 7..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            assert_eq!(digits, v.to_string(), "rust: TypeObject literal {lit}");
        }
    }
}

#[test]
fn micro_numbers_enumerators_in_sequence() {
    check_backend("micro");
    let text = generate("micro");
    for (name, values) in EXPECTED {
        for (lit, v) in values {
            let arm = format!("{v} => Ok(Self::{lit}),");
            assert!(text.contains(&arm), "micro: enum {name}: no `{arm}`");
        }
    }
}

#[test]
fn typescript_numbers_enumerators_in_sequence() {
    check_backend("typescript");
}

#[test]
fn python_numbers_enumerators_in_sequence() {
    check_backend("python");
}

#[test]
fn c_micro_numbers_enumerators_in_sequence() {
    check_backend("c-micro");
}

#[test]
fn c_numbers_enumerators_in_sequence() {
    check_backend("c");
    compile_probe(
        "c",
        "gcc",
        &["-std=c11", "-fsyntax-only"],
        |name, lit, v| {
            format!(
                "_Static_assert({}_{lit} == {v}, \"{name}\");",
                name.to_uppercase()
            )
        },
    );
}

#[test]
fn cpp_numbers_enumerators_in_sequence() {
    check_backend("cpp");
    compile_probe(
        "cpp",
        "g++",
        &["-std=c++17", "-fsyntax-only"],
        |name, lit, v| {
            format!("static_assert(static_cast<int>({name}::{lit}) == {v}, \"{name}\");")
        },
    );
}

/// Compiles the enum bodies of `lang` alone, with one `static_assert` per
/// enumerator; skipped when `tool` is not installed.
fn compile_probe(
    lang: &str,
    tool: &str,
    flags: &[&str],
    assert: impl Fn(&str, &str, i64) -> String,
) {
    if Command::new(tool).arg("--version").output().is_err() {
        eprintln!("no {tool}: probe skipped");
        return;
    }
    let text = generate(lang);
    let mut src = String::new();
    for (name, values) in EXPECTED {
        let lines = body(lang, name, &text);
        if lang == "c" {
            src.push_str("typedef enum {\n");
            for l in &lines {
                src.push_str(l);
                src.push('\n');
            }
            let _ = writeln!(src, "}} {name};");
        } else {
            let _ = writeln!(src, "enum class {name} {{");
            for l in &lines {
                src.push_str(l);
                src.push('\n');
            }
            src.push_str("};\n");
        }
        for (lit, v) in values {
            src.push_str(&assert(name, lit, *v));
            src.push('\n');
        }
    }
    let file = dir(lang).join(format!(
        "probe_{lang}.{}",
        if lang == "c" { "c" } else { "cpp" }
    ));
    fs::write(&file, src).expect("probe");
    let out = Command::new(tool)
        .args(flags)
        .arg(&file)
        .output()
        .expect("compiler");
    assert!(
        out.status.success(),
        "{lang}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
