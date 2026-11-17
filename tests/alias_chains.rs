// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Typedefs reached from another module (`tests/fixtures/roundtrip/
//! alias_chains.idl`): sequence, array, map and optional elements, alias
//! chains across modules, in unions and in key members. A backend that
//! unrolls an alias must name each type as seen from the module it writes
//! into: `A::PList` used in `B` once generated `P x{};` in C++.
//!
//! The generated C++ (`-Werror`) and C compile, Python parses, and the C++
//! and Python encodings of the same samples are byte-identical and decode
//! into each other. The Rust check needs an HDDS checkout:
//! `HDDSGEN_HDDS_DIR=<hdds> cargo test --test alias_chains -- --ignored`.

#![allow(clippy::expect_used, clippy::panic, clippy::needless_pass_by_value)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, IdlFile, Parser};

const IDL: &str = include_str!("fixtures/roundtrip/alias_chains.idl");
const BOUNDED: &str = include_str!("fixtures/roundtrip/alias_chains_bounded.idl");

/// The reduced form of the report: one alias, two modules.
const MINIMAL: &str = "module A { struct P { long v; }; typedef sequence<P> PList; };
module B { @mutable struct M { A::PList items; }; @final struct F { A::PList items; }; };";

fn parse(idl: &str) -> IdlFile {
    Parser::try_new(idl)
        .and_then(|mut p| p.parse())
        .unwrap_or_else(|e| panic!("parse: {e}"))
}

fn generate(backend: Backend, idl: &str) -> String {
    backend
        .generator()
        .generate(&parse(idl))
        .unwrap_or_else(|e| panic!("{e}"))
}

fn work_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("alias_chains")
        .join(name);
    fs::create_dir_all(&dir).expect("work dir");
    dir
}

fn found(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

fn run(tool: &str, args: &[&str], dir: &Path) -> String {
    let out = Command::new(tool)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("tool");
    assert!(
        out.status.success(),
        "{tool} {args:?} failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// No backend spells a type of `A` unqualified inside `B`.
#[test]
fn minimal_report_is_qualified_in_cpp() {
    let out = generate(Backend::Cpp, MINIMAL);
    assert!(!out.contains(" P x"), "unqualified P in B");
    assert!(!out.contains("sizeof(P)"), "unqualified P in B");
}

const CPP_MAIN: &str = r#"#include "alias.hpp"
#include <cstdio>
#include <cstring>
static void hex(const char* tag, const std::uint8_t* b, int n) {
    std::printf("%s ", tag);
    for (int i = 0; i < n; ++i) std::printf("%02x", b[i]);
    std::printf("\n");
}
int main() {
    B::M m;
    m.items = {A::P{1}, A::P{2}};
    m.pair = {A::P{3}, A::P{4}};
    m.by_id[7] = A::P{8};
    m.maybe = A::PList{A::P{9}};
    m.chained = {A::P{10}};
    A::Inner::Q q; q.id = 5; std::memcpy(q.label.data(), "x", 2);
    m.qs = {q};
    m.kind = A::Kind::K_TWO;
    m.kind2 = A::Kind::K_ONE;
    std::uint8_t buf[512];
    int n = m.encode_cdr2_le(buf, sizeof buf);
    if (n <= 0) return 2;
    hex("M", buf, n);
    B::M back;
    if (back.decode_cdr2_le(buf, static_cast<std::size_t>(n)) <= 0) return 3;
    if (back.items.size() != 2 || back.items[1].v != 2 || back.pair[1].v != 4) return 4;
    if (back.by_id.at(7).v != 8 || !back.maybe || (*back.maybe)[0].v != 9) return 5;
    if (back.qs.size() != 1 || back.qs[0].id != 5 || back.kind != A::Kind::K_TWO) return 6;
    B::F f;
    f.items = {A::P{11}};
    f.pair = {A::P{12}, A::P{13}};
    f.qs = {q};
    std::memcpy(f.name.data(), "nm", 3);
    n = f.encode_cdr2_le(buf, sizeof buf);
    if (n <= 0) return 7;
    hex("F", buf, n);
    B::M js;
    if (!B::MPubSubType::populate_from_json(&js, R"({"kind":"K_TWO"})")) return 9;
    if (js.kind != A::Kind::K_TWO) return 10;
    B::U u;
    u._d = A::Kind::K_ONE;
    u.items = {A::P{14}};
    B::HoldsUnions h;
    h.id = 1;
    h.u = u;
    n = h.encode_cdr2_le(buf, sizeof buf);
    if (n <= 0) return 8;
    hex("H", buf, n);
    return 0;
}
"#;

const PY_MAIN: &str = r#"import sys
import alias as g
lines = dict(l.split() for l in open(sys.argv[1]))
q = g.A_Inner_Q(id=5, label="x")
m = g.B_M(items=[g.A_P(1), g.A_P(2)], pair=[g.A_P(3), g.A_P(4)], by_id={7: g.A_P(8)},
          maybe=[g.A_P(9)], chained=[g.A_P(10)], qs=[q], kind=g.A_Kind.K_TWO,
          kind2=g.A_Kind.K_ONE)
f = g.B_F(items=[g.A_P(11)], pair=[g.A_P(12), g.A_P(13)], qs=[q], name="nm")
for tag, value, cls in (("M", m, g.B_M), ("F", f, g.B_F)):
    assert value.encode_cdr2_le().hex() == lines[tag], (tag, value.encode_cdr2_le().hex(), lines[tag])
    back, _ = cls.decode_cdr2_le(bytes.fromhex(lines[tag]))
    assert back == value, (tag, back, value)
h, _ = g.B_HoldsUnions.decode_cdr2_le(bytes.fromhex(lines["H"]))
assert h.encode_cdr2_le().hex() == lines["H"], "H"
print("ok")
"#;

#[test]
fn cpp_and_python_agree() {
    let Some(cxx) = ["c++", "g++", "clang++"].into_iter().find(|c| found(c)) else {
        eprintln!("no C++ compiler: skipped");
        return;
    };
    let dir = work_dir("cpp");
    fs::write(dir.join("alias.hpp"), generate(Backend::Cpp, IDL)).expect("header");
    fs::write(dir.join("main.cpp"), CPP_MAIN).expect("main");
    run(
        cxx,
        &["-std=c++17", "-Wall", "-Werror", "main.cpp", "-o", "main"],
        &dir,
    );
    let hex = run(&dir.join("main").to_string_lossy(), &[], &dir);
    fs::write(dir.join("vectors.txt"), &hex).expect("vectors");
    if !found("python3") {
        eprintln!("no python3: the Python comparison skipped");
        return;
    }
    fs::write(dir.join("alias.py"), generate(Backend::Python, IDL)).expect("module");
    fs::write(dir.join("check.py"), PY_MAIN).expect("script");
    let out = run("python3", &["check.py", "vectors.txt"], &dir);
    assert!(out.contains("ok"), "{out}");
}

/// Spellings that mean another type where an alias is unrolled: a
/// qualified `A::P` read inside `B`, which has its own module `A`; a root
/// `R` read inside `B`, which declares its own `R`. Also a union whose only
/// non-trivial branch is an alias of an array of strings.
const SHADOWED: &str = "struct R { long root; };
module A { struct P { long v; }; };
module C { typedef sequence<A::P> L; typedef sequence<R> RL; typedef string Strs[2]; };
module B {
    module A { struct Z { long z; }; };
    struct R { short inner; };
    struct M { C::L items; C::RL roots; };
    union V switch (long) { case 1: C::Strs s; case 2: long x; };
    struct H { V v; };
};";

#[test]
fn shadowed_spellings_compile_in_cpp() {
    let Some(cxx) = ["c++", "g++", "clang++"].into_iter().find(|c| found(c)) else {
        eprintln!("no C++ compiler: skipped");
        return;
    };
    let dir = work_dir("shadowed");
    fs::write(dir.join("shadowed.hpp"), generate(Backend::Cpp, SHADOWED)).expect("header");
    fs::write(
        dir.join("main.cpp"),
        "#include \"shadowed.hpp\"\n\
         int main() {\n\
             B::M m; m.items = {A::P{1}}; m.roots = {::R{2}};\n\
             std::uint8_t buf[256];\n\
             int n = m.encode_cdr2_le(buf, sizeof buf);\n\
             B::M back;\n\
             if (n <= 0 || back.decode_cdr2_le(buf, static_cast<std::size_t>(n)) <= 0) return 1;\n\
             if (back.items[0].v != 1 || back.roots[0].root != 2) return 2;\n\
             B::H h; h.v._d = 1; h.v.s = {std::string(\"a\"), std::string(\"b\")};\n\
             n = h.encode_cdr2_le(buf, sizeof buf);\n\
             B::H hb;\n\
             if (n <= 0 || hb.decode_cdr2_le(buf, static_cast<std::size_t>(n)) <= 0) return 3;\n\
             return hb.v.s[1] == \"b\" ? 0 : 4;\n\
         }\n",
    )
    .expect("main");
    run(
        cxx,
        &["-std=c++17", "-Wall", "-Werror", "main.cpp", "-o", "main"],
        &dir,
    );
    run(&dir.join("main").to_string_lossy(), &[], &dir);
}

#[test]
fn c_compiles() {
    let Some(cc) = ["cc", "gcc", "clang"].into_iter().find(|c| found(c)) else {
        eprintln!("no C compiler: skipped");
        return;
    };
    let dir = work_dir("c");
    fs::write(dir.join("alias.h"), generate(Backend::C, IDL)).expect("header");
    fs::write(
        dir.join("main.c"),
        "#include \"alias.h\"\nint main(void) {\n    B_M m = {0}; B_F f = {0}; B_K k = {0}; B_HoldsUnions h = {0};\n    (void)m; (void)f; (void)k; (void)h;\n    return 0;\n}\n",
    )
    .expect("main");
    run(
        cc,
        &["-std=c11", "-Wall", "-Werror", "main.c", "-o", "main"],
        &dir,
    );
}

#[test]
fn c_micro_compiles() {
    let Some(cc) = ["cc", "gcc", "clang"].into_iter().find(|c| found(c)) else {
        eprintln!("no C compiler: skipped");
        return;
    };
    let dir = work_dir("c_micro");
    fs::write(dir.join("alias.h"), generate(Backend::CMicro, BOUNDED)).expect("header");
    fs::write(
        dir.join("main.c"),
        "#include \"alias.h\"\nint main(void) {\n    B_M m; B_F f; B_K k; B_HoldsUnions h;\n    (void)m; (void)f; (void)k; (void)h;\n    return 0;\n}\n",
    )
    .expect("main");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    run(
        cc,
        &[
            "-std=c99",
            "-Wall",
            "-Werror",
            "-I",
            &include.to_string_lossy(),
            "main.c",
            "-o",
            "main",
        ],
        &dir,
    );
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test alias_chains -- --ignored"]
fn micro_compiles() {
    let hdds = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        hdds.join("crates/hdds-micro/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={} must name an HDDS checkout: the micro check did NOT run",
        hdds.display()
    );
    let dir = work_dir("micro");
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"alias_micro\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
             [dependencies]\nhdds-micro = {{ path = \"{}\", features = [\"heapless\"] }}\nheapless = \"0.8\"\n[workspace]\n",
            hdds.join("crates/hdds-micro").display()
        ),
    )
    .expect("manifest");
    fs::write(
        dir.join("src/generated.rs"),
        generate(Backend::Micro, BOUNDED),
    )
    .expect("generated");
    fs::write(
        dir.join("src/main.rs"),
        "#![allow(warnings)]\nmod generated;\nfn main() {}\n",
    )
    .expect("main");
    let out = Command::new(env!("CARGO"))
        .args(["build", "--offline", "--quiet"])
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .expect("cargo build");
    assert!(
        out.status.success(),
        "generated micro failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn typescript_generates() {
    let out = generate(Backend::TypeScript, IDL);
    assert!(out.contains("B_M"), "no B::M");
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test alias_chains -- --ignored"]
fn rust_agrees_with_cpp() {
    let hdds = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        hdds.join("crates/hdds/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={} must name an HDDS checkout: the Rust check did NOT run",
        hdds.display()
    );
    let dir = work_dir("rust");
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            "[package]\nname = \"alias\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
             [dependencies]\nhdds = {{ path = \"{}\" }}\n[workspace]\n",
            hdds.join("crates/hdds").display()
        ),
    )
    .expect("manifest");
    fs::write(dir.join("src/generated.rs"), generate(Backend::Rust, IDL)).expect("generated");
    fs::write(
        dir.join("src/shadowed.rs"),
        generate(Backend::Rust, SHADOWED),
    )
    .expect("shadowed");
    fs::write(
        dir.join("src/main.rs"),
        "#![allow(warnings)]\n\
         mod generated;\n\
         mod shadowed;\n\
         use generated::*;\n\
         use hdds::{Cdr2Decode, Cdr2Encode};\n\
         fn hex(b: &[u8]) -> String { b.iter().map(|x| format!(\"{x:02x}\")).collect() }\n\
         fn main() {\n\
             let p = |v| A::P { v };\n\
             let mut label = String::from(\"x\");\n\
             let q = A::Inner::Q { id: 5, label };\n\
             let mut by_id = std::collections::HashMap::new(); by_id.insert(7, p(8));\n\
             let m = B::M { items: vec![p(1), p(2)], pair: [p(3), p(4)], by_id: by_id.into_iter().collect(), maybe: Some(vec![p(9)]), chained: vec![p(10)], qs: vec![q.clone()], kind: A::Kind::K_TWO, kind2: A::Kind::K_ONE };\n\
             let f = B::F { items: vec![p(11)], pair: [p(12), p(13)], qs: vec![q], name: \"nm\".into() };\n\
             let mut buf = [0u8; 512];\n\
             let n = m.encode_cdr2_le(&mut buf).unwrap();\n\
             println!(\"M {}\", hex(&buf[..n]));\n\
             assert_eq!(B::M::decode_cdr2_le(&buf[..n]).unwrap().0, m);\n\
             let n = f.encode_cdr2_le(&mut buf).unwrap();\n\
             println!(\"F {}\", hex(&buf[..n]));\n\
             assert_eq!(B::F::decode_cdr2_le(&buf[..n]).unwrap().0, f);\n\
             let sm = shadowed::B::M { items: vec![shadowed::A::P { v: 1 }], roots: vec![shadowed::R { root: 2 }] };\n\
             let n = sm.encode_cdr2_le(&mut buf).unwrap();\n\
             assert_eq!(shadowed::B::M::decode_cdr2_le(&buf[..n]).unwrap().0, sm);\n\
         }\n",
    )
    .expect("main");
    let out = Command::new(env!("CARGO"))
        .args(["run", "--offline", "--quiet"])
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .expect("cargo run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "generated Rust failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Byte-identical to the C++ encoding when the C++ test ran first.
    let cpp = work_dir("cpp").join("vectors.txt");
    if let Ok(vectors) = fs::read_to_string(cpp) {
        for line in stdout.lines() {
            assert!(
                vectors.contains(line),
                "Rust {line} differs from C++:\n{vectors}"
            );
        }
    }
}
