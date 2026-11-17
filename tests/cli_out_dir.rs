// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! `hddsgen gen --out-dir`: the file written in single-file mode is named
//! for its language (`mod.rs` for the Rust backends, the input's name with
//! the language's extension otherwise), as `--separate` names its files.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::Command;

const IDL: &str = "module Demo { struct Shape { long x; }; };\n";

fn gen(lang: &str, input: &Path, out_dir: &Path, separate: bool) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hddsgen"));
    cmd.args(["gen", lang])
        .arg(input)
        .arg("--out-dir")
        .arg(out_dir);
    if separate {
        cmd.arg("--separate");
    }
    let out = cmd.output().expect("hddsgen");
    assert!(
        out.status.success(),
        "{lang}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("out dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn out_dir_names_the_file_for_its_language() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_out_dir");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root");
    let input = root.join("Shapes.idl");
    fs::write(&input, IDL).expect("idl");
    for (lang, single) in [
        ("cpp", "Shapes.hpp"),
        ("c", "Shapes.h"),
        ("c-micro", "Shapes.h"),
        ("python", "Shapes.py"),
        ("typescript", "Shapes.ts"),
        ("rust", "mod.rs"),
        ("micro", "mod.rs"),
    ] {
        let dir = root.join(lang);
        gen(lang, &input, &dir, false);
        assert_eq!(files(&dir), [single], "{lang}");
        let content = fs::read_to_string(dir.join(single)).expect("output");
        assert!(content.contains("Shape"), "{lang}: no Shape");

        let sep = root.join(format!("{lang}-separate"));
        gen(lang, &input, &sep, true);
        let expected = if single == "mod.rs" {
            "Shapes.rs"
        } else {
            single
        };
        assert_eq!(files(&sep), [expected], "{lang} --separate");
    }
}
