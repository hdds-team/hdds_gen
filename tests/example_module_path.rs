// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! A generated `--example` project must name the module the Rust backend emits.
//!
//! The backend emits `pub mod <IdlName>` with the IDL spelling, under
//! `allow(non_snake_case)`. An example that imports a snake-cased path does not
//! compile as generated, and no fixture caught it because every example IDL in
//! the suite already had a `snake_case` module name.
//!
//! Both sides are pinned here: what the backend emits, and what the example
//! imports. A test that only checked the example would keep passing if the
//! backend changed its module naming.

#![allow(clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use hddsgen::codegen::examples_project::{generate_micro_project, generate_rust_project};
use hddsgen::{Backend, Parser};

/// A module name that is not `snake_case`, which is the DDS norm.
const IDL: &str = "module DroneFleet { struct Position { long drone_id; double latitude; }; };\n";

fn ast() -> hddsgen::ast::IdlFile {
    let mut p = Parser::try_new(IDL).expect("lexer");
    p.parse().expect("parse")
}

fn types_code() -> String {
    Backend::Rust
        .generator()
        .generate(&ast())
        .expect("rust codegen")
}

fn file<'a>(files: &'a HashMap<String, String>, name: &str) -> &'a str {
    files
        .get(name)
        .unwrap_or_else(|| panic!("missing {name}; have {:?}", files.keys()))
}

#[test]
fn the_backend_emits_the_idl_spelling_of_a_module() {
    let types = types_code();
    assert!(
        types.contains("pub mod DroneFleet"),
        "the backend no longer emits the IDL spelling; the examples below follow it"
    );
    assert!(
        !types.contains("pub mod drone_fleet"),
        "the backend snake-cases modules now; update the example import with it"
    );
}

#[test]
fn a_rust_example_imports_the_module_the_backend_emits() {
    let p = generate_rust_project(&ast(), &types_code(), "fleet.idl", None);
    for name in ["src/bin/publisher.rs", "src/bin/subscriber.rs"] {
        let src = file(&p.files, name);
        assert!(
            src.contains("use fleet::DroneFleet::Position;"),
            "{name} imports a path the library does not define: {src}"
        );
    }
}

#[test]
fn a_micro_example_imports_the_module_the_backend_emits() {
    let p = generate_micro_project(&ast(), &types_code(), "fleet.idl", None);
    for name in ["src/bin/sender.rs", "src/bin/receiver.rs"] {
        let src = file(&p.files, name);
        assert!(
            src.contains("use fleet::DroneFleet::Position;"),
            "{name} imports a path the library does not define: {src}"
        );
    }
}

#[test]
fn an_idl_file_named_after_a_rust_keyword_still_yields_a_usable_crate() {
    // `mod.idl` produced `name = "mod"`, which cannot be named in a `use`.
    let p = generate_rust_project(&ast(), &types_code(), "mod.idl", None);
    let manifest = file(&p.files, "Cargo.toml");
    assert!(
        manifest.contains("name = \"mod_types\""),
        "crate name is a Rust keyword: {manifest}"
    );
    let src = file(&p.files, "src/bin/publisher.rs");
    assert!(
        src.contains("use mod_types::DroneFleet::Position;"),
        "the import does not match the crate name: {src}"
    );
}
