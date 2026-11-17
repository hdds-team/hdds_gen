// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The extensibility marker of every generated C, C++, Python and
//! TypeScript structure and union.
//!
//! The encapsulation identifier of a sample names the extensibility of its
//! type (DDS-XTypes 1.3 7.6.3.1.2, Table 60): `CDR2_LE` FINAL, `D_CDR2_LE`
//! APPENDABLE (the body starts with a DHEADER), `PL_CDR2_LE` MUTABLE. The
//! hdds SDK writers are handed encoded bodies and learn the extensibility
//! from the generated type: C `<NAME>_EXTENSIBILITY` and
//! `<name>_extensibility()` (the value of `HDDS_EXTENSIBILITY_*`, 0 / 1 /
//! 2), C++ `T::hdds_extensibility` (the same numbers), Python
//! `T.HDDS_EXTENSIBILITY` and TypeScript `extensibility<Name>` (`"final"`,
//! `"appendable"`, `"mutable"`). Each marker is read back by the language
//! itself where its toolchain is installed (skipped with a message
//! otherwise), and checked in the generated text in every case.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::codegen::data_model::apply_default_extensibility;
use hddsgen::types::ExtensibilityKind;
use hddsgen::{Backend, Parser};

/// FINAL, APPENDABLE by default (no annotation) and explicitly, MUTABLE; a
/// derived structure whose extensibility is its own; a scoped structure and
/// a scoped union.
const IDL: &str = "
@final struct Fin { long x; };
struct App { long x; };
@mutable struct Mut { long x; };
@appendable struct Base { long b; };
@mutable struct Der : Base { long d; };
module inner { @final struct Nest { long y; };
  @mutable union Uni switch (long) { case 1: long z; }; };
";

/// (C prefix, C++ name, Python name, TypeScript name, extensibility).
const TYPES: [(&str, &str, &str, &str, &str); 7] = [
    ("fin", "Fin", "Fin", "Fin", "final"),
    ("app", "App", "App", "App", "appendable"),
    ("mut", "Mut", "Mut", "Mut", "mutable"),
    ("base", "Base", "Base", "Base", "appendable"),
    ("der", "Der", "Der", "Der", "mutable"),
    (
        "inner_nest",
        "inner::Nest",
        "inner.Nest",
        "inner_Nest",
        "final",
    ),
    (
        "inner_uni",
        "inner::Uni",
        "inner.Uni",
        "inner_Uni",
        "mutable",
    ),
];

fn code(kind: &str) -> u32 {
    match kind {
        "final" => 0,
        "appendable" => 1,
        "mutable" => 2,
        other => panic!("{other}"),
    }
}

fn generate(backend: &Backend, idl: &str, default: Option<ExtensibilityKind>) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let mut ast = p.parse().expect("parse");
    if let Some(kind) = default {
        apply_default_extensibility(&mut ast, kind);
    }
    backend.generator().generate(&ast).expect("codegen")
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn found(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("spawn");
    assert!(
        out.status.success(),
        "{cmd:?}: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

fn expected_lines() -> String {
    let mut out = String::new();
    for t in TYPES {
        let _ = writeln!(out, "{}", t.4);
    }
    out
}

#[test]
fn c_types_carry_their_extensibility() {
    let src = generate(&Backend::C, IDL, None);
    // Once per file: the opaque hdds C ABI entities and the two functions
    // the per-type writer-creation helpers below call, never an #include of
    // hdds.h (this generator's header stays self-contained and compiles
    // stand-alone, see writer_create_prelude).
    assert_eq!(
        src.matches("struct HddsParticipant;").count(),
        1,
        "writer_create_prelude must be emitted exactly once per file"
    );
    assert!(src.contains(
        "struct HddsDataWriter *hdds_writer_create_with_extensibility(struct HddsParticipant *aParticipant,"
    ));
    assert!(src.contains(
        "struct HddsDataWriter *hdds_publisher_create_writer_with_type(struct HddsPublisher *aPublisher,"
    ));
    // Every helper announces the type's IDL name: a NULL name would make the
    // writer adopt whatever name its topic announces (XTypes Step 2).
    assert!(
        !src.contains("aTopicName, NULL,"),
        "a helper passes a NULL type name"
    );
    for (prefix, idl, _, _, kind) in TYPES {
        let upper = prefix.to_ascii_uppercase();
        let n = code(kind);
        assert!(
            src.contains(&format!("#define {upper}_EXTENSIBILITY {n}u\n")),
            "{prefix}: {kind}"
        );
        assert!(src.contains(&format!(
            "static inline uint32_t {prefix}_extensibility(void) {{\n    return {upper}_EXTENSIBILITY;\n}}"
        )));
        // The natural way to create a DataWriter of this type: every
        // sample labelled by construction, never FINAL by omission.
        assert!(
            src.contains(&format!(
                "static inline struct HddsDataWriter *{prefix}_create_writer(struct HddsParticipant *aParticipant,"
            )),
            "{prefix}: create_writer helper"
        );
        assert!(src.contains(&format!("{prefix}_extensibility());")));
        assert!(
            src.contains(&format!(
                "aTopicName, \"{idl}\", aQos, {prefix}_extensibility());"
            )),
            "{prefix}: helper announces its IDL name"
        );
        assert!(
            src.contains(&format!(
                "static inline struct HddsDataWriter *{prefix}_create_writer_from_publisher(struct HddsPublisher *aPublisher,"
            )),
            "{prefix}: create_writer_from_publisher helper"
        );
    }
    let cc = ["cc", "gcc", "clang"].into_iter().find(|c| found(c));
    let Some(cc) = cc else {
        eprintln!("no C compiler; generated C marker not compiled");
        return;
    };
    let d = dir("extensibility_marker_c");
    fs::write(d.join("types.h"), &src).expect("header");
    let mut main = String::from(
        "#include \"types.h\"\n#include <stdio.h>\n\
         static const char* kw(uint32_t v) { return v == 0u ? \"final\" : v == 1u ? \"appendable\" : v == 2u ? \"mutable\" : \"?\"; }\n\
         int main(void) {\n",
    );
    for (prefix, ..) in TYPES {
        let upper = prefix.to_ascii_uppercase();
        let _ = write!(main,
            "    if ({prefix}_extensibility() != {upper}_EXTENSIBILITY) {{ return 1; }}\n    puts(kw({prefix}_extensibility()));\n"
        );
    }
    main.push_str("    return 0;\n}\n");
    fs::write(d.join("main.c"), main).expect("main");
    let exe = d.join("main");
    run(Command::new(cc)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(d.join("main.c")));
    assert_eq!(run(&mut Command::new(&exe)), expected_lines());
}

#[test]
fn cpp_types_carry_their_extensibility() {
    let src = generate(&Backend::Cpp, IDL, None);
    let count = src
        .matches("static constexpr std::uint32_t hdds_extensibility = ")
        .count();
    assert_eq!(count, TYPES.len());
    let cxx = ["c++", "g++", "clang++"].into_iter().find(|c| found(c));
    let Some(cxx) = cxx else {
        eprintln!("no C++ compiler; generated C++ marker not compiled");
        return;
    };
    let d = dir("extensibility_marker_cpp");
    fs::write(d.join("types.hpp"), &src).expect("header");
    let mut main = String::from(
        "#include \"types.hpp\"\n#include <cstdio>\n#include <type_traits>\n\
         static const char* kw(std::uint32_t v) { return v == 0u ? \"final\" : v == 1u ? \"appendable\" : v == 2u ? \"mutable\" : \"?\"; }\n\
         int main() {\n",
    );
    for (_, name, _, _, kind) in TYPES {
        let _ = write!(main,
            "    static_assert({name}::hdds_extensibility == {}u, \"{name}\");\n\
             static_assert(std::is_same<decltype({name}::hdds_extensibility), const std::uint32_t>::value, \"{name} type\");\n\
             std::puts(kw({name}::hdds_extensibility));\n",
            code(kind)
        );
    }
    main.push_str("    return 0;\n}\n");
    fs::write(d.join("main.cpp"), main).expect("main");
    let exe = d.join("main");
    run(Command::new(cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(d.join("main.cpp")));
    assert_eq!(run(&mut Command::new(&exe)), expected_lines());
}

#[test]
fn python_types_carry_their_extensibility() {
    let src = generate(&Backend::Python, IDL, None);
    assert_eq!(
        src.matches("    HDDS_EXTENSIBILITY = \"").count(),
        TYPES.len()
    );
    if !found("python3") {
        eprintln!("python3 not found; generated Python marker not run");
        return;
    }
    let d = dir("extensibility_marker_python");
    fs::write(d.join("ext_types.py"), &src).expect("module");
    let mut script = String::from(
        "import dataclasses, sys\nsys.path.insert(0, sys.argv[1])\nimport ext_types as m\n",
    );
    for (_, _, name, _, _) in TYPES {
        // A class attribute, not a dataclass field: the constructor and the
        // encoder do not see it.
        let _ =
            write!(script,
            "assert 'HDDS_EXTENSIBILITY' not in [f.name for f in dataclasses.fields(m.{name})]\n\
             assert m.{name}().HDDS_EXTENSIBILITY == m.{name}.HDDS_EXTENSIBILITY\n\
             print(m.{name}.HDDS_EXTENSIBILITY)\n"
        );
    }
    fs::write(d.join("check.py"), script).expect("script");
    let out = run(Command::new("python3").arg(d.join("check.py")).arg(&d));
    assert_eq!(out, expected_lines());
}

#[test]
fn typescript_types_carry_their_extensibility() {
    let src = generate(&Backend::TypeScript, IDL, None);
    for (_, _, _, name, kind) in TYPES {
        assert!(
            src.contains(&format!(
                "export const extensibility{name} = \"{kind}\" as const;\n"
            )),
            "{name}: {kind}"
        );
    }
    if !found("node") {
        eprintln!("node not found; generated TypeScript marker not run");
        return;
    }
    let d = dir("extensibility_marker_ts");
    fs::write(d.join("ext_types.ts"), &src).expect("module");
    let mut script = String::from("import * as m from \"./ext_types.ts\";\n");
    for (_, _, _, name, _) in TYPES {
        let _ = writeln!(script, "console.log(m.extensibility{name});");
    }
    fs::write(d.join("check.ts"), script).expect("script");
    let out = Command::new("node")
        .args(["--experimental-transform-types", "--no-warnings"])
        .arg(d.join("check.ts"))
        .output()
        .expect("node");
    if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("bad option") {
        eprintln!("this node does not run TypeScript; generated marker not run");
        return;
    }
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), expected_lines());
}

/// `--default-extensibility final` reaches the marker of an un-annotated
/// type, and only of that one.
#[test]
fn the_default_extensibility_reaches_the_marker() {
    let idl = "struct Plain { long x; }; @mutable struct Annotated { long x; };";
    let c = generate(&Backend::C, idl, Some(ExtensibilityKind::Final));
    assert!(c.contains("#define PLAIN_EXTENSIBILITY 0u\n"));
    assert!(c.contains("#define ANNOTATED_EXTENSIBILITY 2u\n"));
    let cpp = generate(&Backend::Cpp, idl, Some(ExtensibilityKind::Final));
    assert!(cpp.contains("static constexpr std::uint32_t hdds_extensibility = 0u;"));
    assert!(cpp.contains("static constexpr std::uint32_t hdds_extensibility = 2u;"));
    let py = generate(&Backend::Python, idl, Some(ExtensibilityKind::Final));
    assert!(py.contains("HDDS_EXTENSIBILITY = \"final\""));
    assert!(py.contains("HDDS_EXTENSIBILITY = \"mutable\""));
    let ts = generate(&Backend::TypeScript, idl, Some(ExtensibilityKind::Final));
    assert!(ts.contains("export const extensibilityPlain = \"final\" as const;"));
    assert!(ts.contains("export const extensibilityAnnotated = \"mutable\" as const;"));
}
