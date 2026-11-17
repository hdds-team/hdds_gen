// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! `hddsgen gen` refuses IDL the validator rejects (warnings pass), and a
//! missing `#include` is an error; the C++ namespace option keeps absolute
//! names working.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn dir() -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_gen_checks");
    fs::create_dir_all(&d).expect("dir");
    d
}

fn hddsgen(args: &[&str], idl: &str, name: &str) -> Output {
    let file = dir().join(name);
    fs::write(&file, idl).expect("idl");
    Command::new(env!("CARGO_BIN_EXE_hddsgen"))
        .args(args)
        .arg(&file)
        .output()
        .expect("hddsgen")
}

#[test]
fn gen_refuses_invalid_idl() {
    let out = hddsgen(&["gen", "c"], "struct S { Missing m; };", "invalid.idl");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("error: Unresolved type reference: Missing"),
        "{err}"
    );
    assert!(err.contains("nothing generated"), "{err}");
    assert!(out.stdout.is_empty());

    // A warning (an undeclared custom annotation) does not stop it.
    let out = hddsgen(&["gen", "c"], "struct S { @mine long a; };", "warn.idl");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: "));
}

#[test]
fn missing_include_is_an_error() {
    for cmd in [&["check"][..], &["gen", "cpp"][..]] {
        let out = hddsgen(
            cmd,
            "#include \"nowhere.idl\"\nstruct S { long a; };\n",
            "inc.idl",
        );
        assert!(!out.status.success(), "{cmd:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("nowhere.idl\": file not found"),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// `--namespace-cpp Outer::In`: the types are declared in the namespace,
/// the runtime outside it, and every absolute name (`::A::P`, emitted for a
/// module type reached through an alias) is written `::Outer::In::A::P`.
#[test]
fn cpp_namespace_keeps_absolute_names() {
    let Some(cxx) = ["c++", "g++", "clang++"]
        .into_iter()
        .find(|c| Command::new(c).arg("--version").output().is_ok())
    else {
        eprintln!("no C++ compiler: skipped");
        return;
    };
    let idl = "struct R { long rv; };
module A { struct P { long v; }; typedef sequence<P> PList; };
module B { @mutable struct M { A::PList items; ::R root; }; };";
    let out_file = dir().join("ns.hpp");
    let out = hddsgen(
        &[
            "gen",
            "cpp",
            "--namespace-cpp",
            "Outer::In",
            "-o",
            &out_file.to_string_lossy(),
        ],
        idl,
        "ns.idl",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let hpp = fs::read_to_string(&out_file).expect("header");
    assert!(
        hpp.contains("::Outer::In::A::P"),
        "absolute names not moved"
    );
    assert!(
        !hpp.contains(" ::A::P"),
        "an absolute name left at the root"
    );
    fs::write(
        dir().join("ns.cpp"),
        "#include \"ns.hpp\"\n\
         int main() {\n\
             Outer::In::B::M m; m.items = {Outer::In::A::P{3}}; m.root.rv = 4;\n\
             std::uint8_t buf[128];\n\
             int n = m.encode_cdr2_le(buf, sizeof buf);\n\
             Outer::In::B::M back;\n\
             if (n <= 0 || back.decode_cdr2_le(buf, static_cast<std::size_t>(n)) <= 0) return 1;\n\
             return back.items[0].v == 3 && back.root.rv == 4 ? 0 : 2;\n\
         }\n",
    )
    .expect("main");
    let build = Command::new(cxx)
        .args(["-std=c++17", "-Wall", "-Werror", "ns.cpp", "-o", "ns"])
        .current_dir(dir())
        .output()
        .expect("c++");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(dir().join("ns")).output().expect("run");
    assert!(run.status.success(), "exit {:?}", run.status.code());
}

/// Function-like macros (spaces in the parameter list, zero parameters),
/// `#` stringizing and `##` token pasting, as `examples/macros` uses them.
#[test]
fn function_like_macros_expand() {
    let src = r#"#define DECL(T, N) T N;
#define ZERO() int32_t
#define STR(x) #x
#define PASTE(base, n) int32_t base##_##n;
struct M {
    DECL(int32_t, a)
    DECL(string,  name)
    ZERO() z;
    PASTE(field, 1)
};
const string S1 = STR(foo "bar"   baz);
"#;
    let out = hddsgen(&["fmt"], src, "macros.idl");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for line in [
        "int32_t a;",
        "string  name;",
        "int32_t z;",
        "int32_t field_1;",
        r#"const string S1 = "foo \"bar\" baz";"#,
    ] {
        assert!(text.contains(line), "missing {line}\n{text}");
    }
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/macros");
    for entry in fs::read_dir(&examples).expect("examples/macros") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "idl") {
            let out = Command::new(env!("CARGO_BIN_EXE_hddsgen"))
                .args(["gen", "cpp"])
                .arg(&path)
                .output()
                .expect("hddsgen");
            assert!(
                out.status.success(),
                "{}: {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}
