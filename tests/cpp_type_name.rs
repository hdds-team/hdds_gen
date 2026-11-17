// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The C++ `hdds_type_name` of every generated structure and union is the
//! type name the Rust backend announces for the same IDL type, module
//! scopes included (`inner::deep::Uni`, not `Uni`).

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, Parser};

/// A structure in a module, a union in a nested module and a top-level
/// structure.
const IDL: &str = "
module inner {
  struct Nest { long y; };
  module deep { @mutable union Uni switch (long) { case 1: long z; }; };
};
struct Top { long x; };
";

/// (C++ declaration name, IDL qualified name) of each type of [`IDL`].
const TYPES: [(&str, &str); 3] = [
    ("Nest", "inner::Nest"),
    ("Uni", "inner::deep::Uni"),
    ("Top", "Top"),
];

fn generate(backend: &Backend) -> String {
    let mut p = Parser::try_new(IDL).expect("lexer");
    let ast = p.parse().expect("parse");
    backend.generator().generate(&ast).expect("codegen")
}

/// The text up to the closing quote of every literal that follows `marker`.
fn quoted_after(src: &str, marker: &str) -> Vec<String> {
    src.split(marker)
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap_or_default().to_string())
        .collect()
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::create_dir_all(&d).expect("dir");
    d
}

#[test]
fn cpp_type_name_is_the_rust_backend_type_name() {
    let cpp = generate(&Backend::Cpp);
    let rust = generate(&Backend::Rust);

    // Rust announces a structure in the `type_name` of its TypeDescriptor
    // and every type (structure, union) in its TypeObject detail name.
    assert_eq!(
        quoted_after(&rust, "type_name: \""),
        ["inner::Nest", "Top"],
        "Rust TypeDescriptor names"
    );
    let rust_detail = quoted_after(&rust, "CompleteTypeDetail::new(\"");

    assert_eq!(
        cpp.matches("static constexpr const char* hdds_type_name = ")
            .count(),
        TYPES.len(),
        "one hdds_type_name per generated type"
    );
    for (decl, fqn) in TYPES {
        assert!(
            rust_detail.iter().any(|n| n == fqn),
            "Rust does not announce {fqn}"
        );
        // The first hdds_type_name after the declaration belongs to it.
        let start = cpp
            .find(&format!("struct {decl} {{"))
            .unwrap_or_else(|| panic!("no C++ declaration of {decl}"));
        let line = format!("static constexpr const char* hdds_type_name = \"{fqn}\";");
        let at = cpp[start..]
            .find("static constexpr const char* hdds_type_name = ")
            .map_or_else(|| panic!("{decl}: no hdds_type_name"), |i| start + i);
        assert!(
            cpp[at..].starts_with(&line),
            "{decl}: expected `{line}`, found `{}`",
            cpp[at..].lines().next().unwrap_or_default()
        );
    }

    // The member is a usable constant expression of the generated type.
    let Some(cxx) = ["c++", "g++", "clang++"]
        .into_iter()
        .find(|c| Command::new(c).arg("--version").output().is_ok())
    else {
        eprintln!("no C++ compiler; hdds_type_name not compiled");
        return;
    };
    let d = dir("cpp_type_name");
    fs::write(d.join("types.hpp"), &cpp).expect("header");
    let mut main = String::from(
        "#include \"types.hpp\"\n#include <cstdio>\n#include <string_view>\nint main() {\n",
    );
    for (decl, fqn) in TYPES {
        let ns = fqn
            .strip_suffix(&format!("::{decl}"))
            .map_or_else(String::new, |p| format!("{p}::"));
        let _ = writeln!(
            main,
            "    static_assert(std::string_view({ns}{decl}::hdds_type_name) == std::string_view(\"{fqn}\"), \"{decl}\");"
        );
        let _ = writeln!(main, "    std::puts({ns}{decl}::hdds_type_name);");
    }
    main.push_str("    return 0;\n}\n");
    fs::write(d.join("main.cpp"), main).expect("main");
    let exe = d.join("main");
    let build = Command::new(cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(d.join("main.cpp"))
        .output()
        .expect("spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&exe).output().expect("run");
    let mut expected = String::new();
    for (_, fqn) in TYPES {
        let _ = writeln!(expected, "{fqn}");
    }
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}

/// The `type_name()` of each `FooPubSubType` is the same qualified IDL name
/// as `hdds_type_name` (`XTypes` 1.3 Step 2 compares it between endpoints).
#[test]
fn cpp_pubsub_type_name_is_the_qualified_name() {
    let cpp = generate(&Backend::Cpp);
    // Only structures get a PubSubType (the union `Uni` has none).
    let structs: Vec<(&str, &str)> = TYPES.into_iter().filter(|(d, _)| *d != "Uni").collect();

    for &(decl, fqn) in &structs {
        let start = cpp
            .find(&format!("class {decl}PubSubType {{"))
            .unwrap_or_else(|| panic!("no {decl}PubSubType"));
        let at = cpp[start..]
            .find("static constexpr const char* type_name() noexcept")
            .map_or_else(|| panic!("{decl}PubSubType: no type_name()"), |i| start + i);
        let line =
            format!("static constexpr const char* type_name() noexcept {{ return \"{fqn}\"; }}");
        assert!(
            cpp[at..].starts_with(&line),
            "{decl}PubSubType: expected `{line}`, found `{}`",
            cpp[at..].lines().next().unwrap_or_default()
        );
    }

    // The name is a usable constant expression, equal to hdds_type_name.
    let Some(cxx) = ["c++", "g++", "clang++"]
        .into_iter()
        .find(|c| Command::new(c).arg("--version").output().is_ok())
    else {
        eprintln!("no C++ compiler; PubSubType::type_name not compiled");
        return;
    };
    let d = dir("cpp_pubsub_type_name");
    fs::write(d.join("types.hpp"), &cpp).expect("header");
    let mut main = String::from(
        "#include \"types.hpp\"\n#include <cstdio>\n#include <string_view>\nint main() {\n",
    );
    for &(decl, fqn) in &structs {
        let ns = fqn
            .strip_suffix(&format!("::{decl}"))
            .map_or_else(String::new, |p| format!("{p}::"));
        let _ = writeln!(
            main,
            "    static_assert(std::string_view({ns}{decl}PubSubType::type_name()) == std::string_view({ns}{decl}::hdds_type_name), \"{decl}\");"
        );
        let _ = writeln!(main, "    std::puts({ns}{decl}PubSubType::type_name());");
    }
    main.push_str("    return 0;\n}\n");
    fs::write(d.join("pubsub.cpp"), main).expect("main");
    let exe = d.join("pubsub");
    let build = Command::new(cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(d.join("pubsub.cpp"))
        .output()
        .expect("spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let out = Command::new(&exe).output().expect("run");
    let mut expected = String::new();
    for &(_, fqn) in &structs {
        let _ = writeln!(expected, "{fqn}");
    }
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
}
