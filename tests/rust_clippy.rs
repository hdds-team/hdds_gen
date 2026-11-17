// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The generated Rust code is clean under `cargo clippy -- -D warnings`.
//!
//! Every IDL of the test corpus (and `fixtures/clippy_surface.idl`, which
//! covers the shapes a user crate meets: optionals in each extensibility,
//! strings behind references, enums of each width, unions, bitsets,
//! bitmasks, keyed structs, externals) is compiled by hddsgen, each into
//! its own module of one crate, and the crate is linted. The only
//! allowance is `dead_code`, the crate has no caller for the types.
//!
//! The generated code depends on the `hdds` crate of the HDDS repository:
//! run with `HDDSGEN_HDDS_DIR=<hdds checkout> cargo test --test rust_clippy
//! -- --ignored`. Ignored by default, failing when requested without the
//! checkout. Cargo runs offline: the HDDS dependencies must already be in
//! the local registry cache.

#![allow(clippy::expect_used)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, Parser};

const IDLS: &[(&str, &str)] = &[
    ("surface", include_str!("fixtures/clippy_surface.idl")),
    (
        "corpus",
        include_str!("fixtures/xtypes_datapath/corpus.idl"),
    ),
    ("golden", include_str!("fixtures/key_hash_golden.idl")),
    ("ext", include_str!("fixtures/key_hash_ext.idl")),
    (
        "typeobject_extra",
        include_str!("fixtures/xtypes_datapath/typeobject_extra.idl"),
    ),
    ("interop", include_str!("fixtures/interop_test.idl")),
];

fn hdds_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        dir.join("crates/hdds/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={:?} must name an HDDS checkout (crates/hdds): \
         the clippy test did NOT run",
        dir.display().to_string()
    );
    dir
}

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::Rust.generator().generate(&ast).expect("codegen")
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test rust_clippy -- --ignored"]
fn generated_rust_is_clippy_clean() {
    let hdds = hdds_dir();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("rust_clippy");
    fs::create_dir_all(root.join("src")).expect("crate dir");
    let manifest = format!(
        "[package]\nname = \"rust_clippy\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [dependencies]\nhdds = {{ path = \"{}\" }}\n\n[workspace]\n",
        hdds.join("crates/hdds").display()
    );
    fs::write(root.join("Cargo.toml"), manifest).expect("manifest");

    let mut main = String::new();
    for (name, idl) in IDLS {
        let file = format!("{name}.rs");
        fs::write(root.join("src").join(&file), generate(idl)).expect("source");
        let _ = writeln!(
            main,
            "#[allow(dead_code)]\nmod {name} {{ include!(\"{file}\"); }}"
        );
    }
    main.push_str("fn main() {}\n");
    fs::write(root.join("src/main.rs"), main).expect("main");

    let output = Command::new(env!("CARGO"))
        .args(["clippy", "--offline", "--quiet", "--", "-D", "warnings"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .expect("cargo clippy");
    assert!(
        output.status.success(),
        "generated Rust is not clippy clean\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
