// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Regression test for the C++ `--fastdds-compat` mode.
//!
//! In compat mode struct fields are private with accessor methods, so the
//! generated `PubSubType` helpers (`compute_key`, `populate_from_json`, ...) must
//! go through the accessor API instead of direct member access. Because those
//! helpers are inline member functions, any member-access slip makes every
//! generated header fail C++ semantic analysis on include, even uncalled.
//! This test generates the compat header for a nested/keyed IDL and, when a
//! C++ compiler is available, checks it with `-std=c++17 -fsyntax-only`.

#![allow(clippy::expect_used)]

use std::fs;
use std::process::Command;

fn find_cxx() -> Option<String> {
    for cxx in &["clang++", "g++"] {
        if Command::new(cxx).arg("--version").output().is_ok() {
            return Some((*cxx).to_string());
        }
    }
    None
}

fn run_idl_gen(args: &[&str]) -> bool {
    let status = Command::new("cargo")
        .args(["run", "--quiet", "--bin", "hddsgen", "--"])
        .args(args)
        .status();
    matches!(status, Ok(s) if s.success())
}

#[test]
fn fastdds_compat_nested_keyed_header_compiles() {
    let base = std::env::temp_dir();
    let dir = base.join(format!("hdds_gen_fdds_{}_{}", std::process::id(), 1));
    let _ = fs::create_dir_all(&dir);

    // Nested/keyed IDL exercising the PubSubType helpers: @key primitive,
    // @key nested struct (sub-population takes its address), string,
    // sequence and fixed array.
    let idl = r"
        module arq {
            @final
            struct EntityId {
                @key unsigned long id;
                unsigned short site;
            };

            @final
            struct Command {
                @key EntityId target;
                string name;
                sequence<double> params;
                double values[4];
            };
        };
    ";
    let idl_path = dir.join("compat.idl");
    fs::write(&idl_path, idl).expect("write idl");

    // Generate the fastdds-compat C++ header
    let out_h = dir.join("compat.h");
    assert!(
        run_idl_gen(&[
            "gen",
            "cpp",
            "--fastdds-compat",
            idl_path.to_str().expect("utf-8 path"),
            "-o",
            out_h.to_str().expect("utf-8 path"),
        ]),
        "hddsgen gen cpp --fastdds-compat failed"
    );

    // The helpers must use the accessor API, never private members
    let header = fs::read_to_string(&out_h).expect("read generated header");
    assert!(
        header.contains("hdds_kh_be(w, (uint32_t)((*typed).target().id()), 4u);"),
        "compute_key must read key fields through the accessor: {header}"
    );
    assert!(
        header.contains("populate_from_json(&typed->target()"),
        "nested sub-population must take the mutable accessor's address: {header}"
    );

    // Semantic check with a real compiler when one is available
    let Some(cxx) = find_cxx() else {
        eprintln!("C++ compiler not found; skipping fastdds-compat syntax check");
        return;
    };
    let main_cxx = dir.join("main.cxx");
    fs::write(
        &main_cxx,
        "#include \"compat.h\"\nint main() { return 0; }\n",
    )
    .expect("write main.cxx");
    let status = Command::new(&cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg("-I")
        .arg(&dir)
        .arg(&main_cxx)
        .status()
        .expect("run c++ compiler");
    assert!(
        status.success(),
        "fastdds-compat header failed {cxx} -std=c++17 -fsyntax-only"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn qualified_nested_pubsub_type_compiles_in_both_modes() {
    let base = std::env::temp_dir();
    let dir = base.join(format!("hdds_gen_fqn_{}_{}", std::process::id(), 1));
    let _ = fs::create_dir_all(&dir);
    let idl_path = dir.join("qualified.idl");
    fs::write(
        &idl_path,
        "module A { struct X { long a; }; };\nstruct UseOk { A::X ax; };\n",
    )
    .expect("write qualified idl");

    let Some(cxx) = find_cxx() else {
        eprintln!("C++ compiler not found; skipping qualified type syntax check");
        return;
    };

    for (mode, compat) in [("plain", false), ("compat", true)] {
        let header_path = dir.join(format!("{mode}.h"));
        let mut args = vec!["gen", "cpp"];
        if compat {
            args.push("--fastdds-compat");
        }
        args.extend([
            idl_path.to_str().expect("utf-8 path"),
            "-o",
            header_path.to_str().expect("utf-8 path"),
        ]);
        assert!(run_idl_gen(&args), "{mode} C++ generation failed");

        let header = fs::read_to_string(&header_path).expect("read generated header");
        assert!(
            header.contains("A::XPubSubType::populate_from_json"),
            "{mode} output lost the namespace-qualified PubSubType"
        );

        let source_path = dir.join(format!("{mode}.cxx"));
        fs::write(
            &source_path,
            format!("#include \"{mode}.h\"\nint main() {{ return 0; }}\n"),
        )
        .expect("write C++ probe");
        let status = Command::new(&cxx)
            .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
            .arg("-I")
            .arg(&dir)
            .arg(&source_path)
            .status()
            .expect("run C++ compiler");
        assert!(
            status.success(),
            "{mode} qualified header failed to compile"
        );
    }

    let _ = fs::remove_dir_all(&dir);
}
