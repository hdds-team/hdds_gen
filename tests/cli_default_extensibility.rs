// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! `hddsgen gen` without `--default-extensibility` generates an
//! un-annotated type as APPENDABLE (DDS-XTypes 1.3 7.3.1.2.1.8) and says so
//! on stderr; `--default-extensibility final` gives the FINAL layout of
//! hddsgen 1.x, silently.

#![allow(clippy::expect_used)]

use std::process::Command;

fn gen(extra: &[&str]) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("hddsgen-cli-ext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let idl = dir.join("p.idl");
    std::fs::write(&idl, "struct P { long x; };\n").expect("idl");
    let out = Command::new(env!("CARGO_BIN_EXE_hddsgen"))
        .arg("gen")
        .args(extra)
        .arg("rust")
        .arg(&idl)
        .output()
        .expect("run hddsgen");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).expect("utf-8 code"),
        String::from_utf8(out.stderr).expect("utf-8 stderr"),
    )
}

#[test]
fn an_unannotated_type_is_appendable_and_the_old_default_is_one_option_away() {
    let (code, info) = gen(&[]);
    assert!(
        code.contains("begin_len()?"),
        "APPENDABLE carries a DHEADER"
    );
    assert!(info.contains("generated as APPENDABLE"), "{info}");
    assert!(info.contains("--default-extensibility final"), "{info}");

    let (code, info) = gen(&["--default-extensibility", "final"]);
    assert!(!code.contains("begin_len()?"), "FINAL carries no DHEADER");
    assert!(info.is_empty(), "{info}");
}

/// An un-annotated enumeration or bitmask is APPENDABLE too: its
/// `TypeObject`'s `enum_flags` / `bitmask_flags` is `IS_APPENDABLE` (0x0002),
/// and `@final` gives `IS_FINAL` (0x0001) (DDS-XTypes 1.3 7.2.3 Table 12,
/// 7.3.1.2.1.8).
#[test]
fn an_unannotated_enum_or_bitmask_announces_appendable() {
    use hddsgen::{Backend, Parser};
    let gen = |idl: &str| {
        let ast = Parser::try_new(idl).expect("lexer").parse().expect("parse");
        Backend::Rust.generator().generate(&ast).expect("generate")
    };
    let code = gen("enum E { A, B }; bitmask M { X, Y };");
    assert!(
        code.contains("enum_flags: ::hdds::xtypes::StructTypeFlag(0x0002)"),
        "{code}"
    );
    assert!(
        code.contains("bitmask_flags: ::hdds::xtypes::StructTypeFlag(0x0002)"),
        "{code}"
    );
    let code = gen("@final enum E { A, B }; @final bitmask M { X, Y };");
    assert!(
        code.contains("enum_flags: ::hdds::xtypes::StructTypeFlag(0x0001)"),
        "{code}"
    );
    assert!(
        code.contains("bitmask_flags: ::hdds::xtypes::StructTypeFlag(0x0001)"),
        "{code}"
    );
}
