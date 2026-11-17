// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! A structure or union member that differs from its type only by case is a
//! warning, and every backend generates it: ROS 2 declares
//! `unique_identifier_msgs::msg::UUID` with a member `uuid`. A member spelled
//! exactly like its type stays an error (IDL 4.2 7.5.2): the C++ backend
//! would declare a member named like its class.
//!
//! The Rust backends compile against an HDDS checkout, so those checks are
//! `#[ignore]`d like `alias_chains`: `HDDSGEN_HDDS_DIR=<hdds> cargo test
//! --test member_case_collision -- --ignored`. The TypeScript check needs a
//! `tsc` (`HDDS_TSC`).

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use hddsgen::validate::Level;
use hddsgen::{Backend, IdlFile, Parser};

/// The IDL rosidl emits for `unique_identifier_msgs/msg/UUID`.
const UUID_IDL: &str =
    "module unique_identifier_msgs { module msg { struct UUID { uint8 uuid[16]; }; }; };\n";

/// Every language of `hddsgen gen`, with the backend it selects.
const BACKENDS: [(Backend, &str); 7] = [
    (Backend::Cpp, "cpp"),
    (Backend::Rust, "rust"),
    (Backend::Python, "python"),
    (Backend::C, "c"),
    (Backend::Micro, "micro"),
    (Backend::CMicro, "c-micro"),
    (Backend::TypeScript, "typescript"),
];

const WARNING: &str = "member 'uuid' differs from its struct only by case (IDL 4.2 7.5.2): \
     accepted for ROS 2 compatibility";

fn parse(idl: &str) -> IdlFile {
    Parser::try_new(idl)
        .and_then(|mut p| p.parse())
        .unwrap_or_else(|e| panic!("parse: {e}\n{idl}"))
}

fn errors(idl: &str) -> Vec<String> {
    hddsgen::validate(&parse(idl))
        .into_iter()
        .filter(|d| matches!(d.level, Level::Error))
        .map(|d| d.message)
        .collect()
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("member_case_collision")
        .join(name);
    fs::create_dir_all(&d).expect("dir");
    d
}

/// The IDL file of `idl`, one per caller: tests run in parallel, and a
/// shared file rewritten by one test (`fs::write` truncates first) was read
/// empty by another test's hddsgen, which then succeeded without the warning.
fn idl_file(idl: &str, lang: &str, out_name: &str) -> PathBuf {
    let file = dir(lang).join(format!("{out_name}.idl"));
    fs::write(&file, idl).expect("idl");
    file
}

/// `hddsgen gen <lang> <idl> -o <out>`, its output and its path.
fn gen(lang: &str, out_name: &str) -> (Output, PathBuf) {
    let out_file = dir(lang).join(out_name);
    let out = Command::new(env!("CARGO_BIN_EXE_hddsgen"))
        .args(["gen", lang])
        .arg(idl_file(UUID_IDL, lang, out_name))
        .arg("-o")
        .arg(&out_file)
        .output()
        .expect("hddsgen");
    (out, out_file)
}

/// The generated file of `lang`, which must be produced with the warning.
fn generated(lang: &str, out_name: &str) -> PathBuf {
    let (out, file) = gen(lang, out_name);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{lang}: {err}");
    assert!(
        err.contains(&format!("warning: struct UUID: {WARNING}")),
        "{lang}: {err}"
    );
    assert!(
        fs::metadata(&file).expect("output").len() > 0,
        "{lang}: empty"
    );
    file
}

fn first_compiler(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find(|c| Command::new(c).arg("--version").output().is_ok())
        .map(|c| (*c).to_string())
}

#[test]
fn case_only_member_is_a_warning_not_an_error() {
    let ast = parse(UUID_IDL);
    let diags = hddsgen::validate(&ast);
    let errs: Vec<&str> = diags
        .iter()
        .filter(|d| matches!(d.level, Level::Error))
        .map(|d| d.message.as_str())
        .collect();
    assert!(errs.is_empty(), "{errs:?}");
    let warnings: Vec<&str> = diags
        .iter()
        .filter(|d| matches!(d.level, Level::Warning))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].starts_with("struct UUID: ") && warnings[0].contains(WARNING),
        "{}",
        warnings[0]
    );

    // A union case differing only by case is the same warning.
    let union = "union U switch (long) { case 1: long u; };\n";
    assert!(errors(union).is_empty());
    assert!(hddsgen::validate(&parse(union)).iter().any(|d| {
        matches!(d.level, Level::Warning)
            && d.message
                .contains("union U: member 'u' differs from its union only by case")
    }));

    // Every backend generates the structure, and the CLI prints the warning
    // and keeps going.
    for (backend, lang) in BACKENDS {
        let code = backend
            .generator()
            .generate(&ast)
            .unwrap_or_else(|e| panic!("{lang}: {e}"));
        assert!(code.contains("uuid") && code.contains("UUID"), "{lang}");
        generated(lang, &format!("uuid_warn.{lang}"));
    }
}

#[test]
fn exact_spelling_stays_an_error() {
    for idl in [
        "struct B { long B; };",
        "union U switch (long) { case 1: long U; };",
        "module m { struct UUID { uint8 UUID[16]; }; };",
    ] {
        let errs = errors(idl);
        assert!(
            errs.iter().any(|e| e.contains("is named like its")),
            "{idl}: {errs:?}"
        );
    }

    // The CLI refuses it for every language, generating nothing.
    let file = dir("exact").join("same.idl");
    fs::write(&file, "struct B { long B; };\n").expect("idl");
    for (_, lang) in BACKENDS {
        let out = Command::new(env!("CARGO_BIN_EXE_hddsgen"))
            .args(["gen", lang])
            .arg(&file)
            .output()
            .expect("hddsgen");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{lang} accepted it");
        assert!(
            err.contains("error: struct B: member 'B' is named like its struct"),
            "{lang}: {err}"
        );
        assert!(out.stdout.is_empty(), "{lang}: output written");
    }

    // The other collision rules are not relaxed: two members differing only
    // by case, and a type and a constant of one scope.
    for idl in [
        "struct S { long a; long A; };",
        "struct Ab { long x; }; const long AB = 1;",
    ] {
        let errs = errors(idl);
        assert!(
            errs.iter().any(|e| e.contains("collides")),
            "{idl}: {errs:?}"
        );
    }
}

#[test]
fn c_compiles_the_case_only_member() {
    let Some(cc) = first_compiler(&["cc", "gcc", "clang"]) else {
        eprintln!("no C compiler: skipped");
        return;
    };
    let header = generated("c", "uuid.h");
    let d = dir("c_build");
    let src = d.join("main.c");
    fs::write(
        &src,
        "#include \"uuid.h\"\nint main(void) {\n    unique_identifier_msgs_msg_UUID u = {{0}};\n    u.uuid[0] = 1;\n    return u.uuid[0] == 1 ? 0 : 1;\n}\n",
    )
    .expect("main");
    fs::copy(&header, d.join("uuid.h")).expect("header");
    let exe = d.join("uuid_c");
    let out = Command::new(&cc)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(&d)
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(Command::new(&exe).status().expect("run").success());
}

#[test]
fn cpp_compiles_the_case_only_member() {
    let Some(cxx) = first_compiler(&["c++", "g++", "clang++"]) else {
        eprintln!("no C++ compiler: skipped");
        return;
    };
    let header = generated("cpp", "uuid.hpp");
    let d = dir("cpp_build");
    fs::copy(&header, d.join("uuid.hpp")).expect("header");
    let src = d.join("main.cpp");
    fs::write(
        &src,
        "#include \"uuid.hpp\"\nint main() {\n    unique_identifier_msgs::msg::UUID u{};\n    u.uuid[0] = 1;\n    return u.uuid[0] == 1 ? 0 : 1;\n}\n",
    )
    .expect("main");
    let exe = d.join("uuid_cpp");
    let out = Command::new(&cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(&d)
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("c++");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(Command::new(&exe).status().expect("run").success());
}

#[test]
fn c_micro_compiles_the_case_only_member() {
    let Some(cc) = first_compiler(&["cc", "gcc", "clang"]) else {
        eprintln!("no C compiler: skipped");
        return;
    };
    let header = generated("c-micro", "uuid.h");
    let d = dir("c_micro_build");
    fs::copy(&header, d.join("uuid.h")).expect("header");
    let src = d.join("main.c");
    fs::write(
        &src,
        "#include \"uuid.h\"\nint main(void) {\n    unique_identifier_msgs_msg_UUID u = {{0}};\n    u.uuid[0] = 1;\n    return u.uuid[0] == 1 ? 0 : 1;\n}\n",
    )
    .expect("main");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let exe = d.join("uuid_c_micro");
    let out = Command::new(&cc)
        .args([
            "-std=c99",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
        ])
        .arg(&d)
        .arg("-I")
        .arg(&include)
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(Command::new(&exe).status().expect("run").success());
}

#[test]
fn python_output_is_valid_python() {
    let Some(_) = first_compiler(&["python3"]) else {
        eprintln!("no python3: skipped");
        return;
    };
    let file = generated("python", "uuid.py");
    let out = Command::new("python3")
        .args([
            "-I",
            "-c",
            "import sys; compile(open(sys.argv[1], encoding='utf-8').read(), sys.argv[1], 'exec')",
        ])
        .arg(&file)
        .output()
        .expect("python3");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Rust: `hdds` is the checkout's crate, the same for the `micro` backend
/// with `hdds-micro`. Ignored by default (it needs an HDDS checkout).
fn cargo_build_against(name: &str, main_rs: &str, dep: &str) {
    let hdds = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        hdds.join("crates").join(dep).join("Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={} must name an HDDS checkout: the Rust check did NOT run",
        hdds.display()
    );
    let lang = if dep == "hdds" { "rust" } else { "micro" };
    let d = dir(&format!("{name}_crate"));
    fs::create_dir_all(d.join("src")).expect("src");
    let features = if dep == "hdds-micro" {
        ", features = [\"heapless\"]"
    } else {
        ""
    };
    fs::write(
        d.join("Cargo.toml"),
        format!(
            "[package]\nname = \"uuid_{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
             [dependencies]\n{dep} = {{ path = \"{}\"{features} }}\nheapless = \"0.8\"\n[workspace]\n",
            hdds.join("crates").join(dep).display()
        ),
    )
    .expect("manifest");
    fs::copy(generated(lang, "generated.rs"), d.join("src/generated.rs")).expect("generated");
    fs::write(d.join("src/main.rs"), main_rs).expect("main");
    let out = Command::new(env!("CARGO"))
        .args(["build", "--offline", "--quiet"])
        .current_dir(&d)
        .env("CARGO_TARGET_DIR", d.join("target"))
        .output()
        .expect("cargo build");
    assert!(
        out.status.success(),
        "generated {lang} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test member_case_collision -- --ignored"]
fn rust_compiles_the_case_only_member() {
    cargo_build_against(
        "rust",
        "#![allow(warnings)]\nmod generated;\nfn main() {}\n",
        "hdds",
    );
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test member_case_collision -- --ignored"]
fn micro_compiles_the_case_only_member() {
    cargo_build_against(
        "micro",
        "#![allow(warnings)]\nmod generated;\nfn main() {}\n",
        "hdds-micro",
    );
}

#[test]
#[ignore = "needs tsc: HDDS_TSC=<tsc> cargo test --test member_case_collision -- --ignored"]
fn typescript_passes_tsc_strict() {
    let tsc = std::env::var_os("HDDS_TSC").map_or_else(
        || panic!("HDDS_TSC not set: the strict type check did NOT run"),
        PathBuf::from,
    );
    let mut src = fs::read_to_string(generated("typescript", "uuid.ts")).expect("ts");
    src.push_str(
        "\nconst _u: unique_identifier_msgs_msg_UUID = { uuid: new Array<number>(16).fill(0) };\n\
         export const _d = defaultunique_identifier_msgs_msg_UUID();\n\
         export const _e = encodeunique_identifier_msgs_msg_UUID;\n\
         export const _u2 = _u;\n",
    );
    let d = dir("typescript_build");
    let file = d.join("uuid_check.ts");
    fs::write(&file, src).expect("ts");
    let out = Command::new(&tsc)
        .args(["--strict", "--noEmit", "--target", "es2020"])
        .arg(&file)
        .output()
        .expect("tsc");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
