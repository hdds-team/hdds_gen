// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The C `<type>_decode_cdr2_le` entry point takes no allocator. A type that
//! needs one (a string, sequence or map member) carries a generated comment
//! above that entry point naming the `_alloc` variant to use; a type that
//! needs none does not. The runtime check confirms the comment: without an
//! allocator the decoder refuses a value holding storage with
//! `-CDR_OUT_OF_MEMORY`, and the named variant decodes the same bytes.
//! Skipped, with a message, without a C compiler.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::codegen::c::CGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

const IDL: &str = "
struct Plain { long a; double b; };
struct Strs { string name; };
struct Seqs { sequence<long> v; };
struct Bounded { sequence<long, 4> v; };
struct Nested { Plain p; Strs s; };
struct BoundedStr { string<8> s; };
union UStr switch (long) { case 1: long a; case 2: string s; };
union UPlain switch (long) { case 1: long a; case 2: short b; };
";

/// Types whose `_decode_cdr2_le` gets the comment (a sequence, a map or an
/// unbounded string, directly or through a member).
const NEEDS_ALLOC: [&str; 5] = ["strs", "seqs", "bounded", "nested", "ustr"];
/// Types whose `_decode_cdr2_le` gets no comment.
const NO_ALLOC: [&str; 3] = ["plain", "boundedstr", "uplain"];

const NOTE: &str = "This type needs an allocator:";

fn generate() -> String {
    let mut p = Parser::try_new(IDL).expect("lexer");
    let ast = p.parse().expect("parse");
    CGenerator::new().generate(&ast).expect("codegen")
}

/// The text directly above the definition of `f`'s entry point (the lines
/// up to the blank line that precedes it).
fn above_entry(src: &str, f: &str) -> String {
    let sig = format!("static inline int {f}_decode_cdr2_le(");
    let at = src
        .find(&sig)
        .unwrap_or_else(|| panic!("no {f}_decode_cdr2_le"));
    let head = &src[..at];
    let start = head.rfind("\n\n").map_or(0, |i| i + 2);
    head[start..].to_string()
}

#[test]
fn decode_entry_without_allocator_names_the_alloc_variant_when_needed() {
    let c = generate();

    for f in NEEDS_ALLOC {
        let block = above_entry(&c, f);
        assert!(block.contains(NOTE), "{f}: no allocator note\n{block}");
        assert!(
            block.contains("CDR_OUT_OF_MEMORY"),
            "{f}: note does not state the error\n{block}"
        );
        assert!(
            block.contains(&format!("{f}_decode_cdr2_le_alloc")),
            "{f}: note does not name {f}_decode_cdr2_le_alloc\n{block}"
        );
        assert!(
            block.trim_end().ends_with("*/"),
            "{f}: note is not a comment directly above the entry\n{block}"
        );
    }

    for f in NO_ALLOC {
        let block = above_entry(&c, f);
        assert!(
            !block.contains(NOTE),
            "{f}: unexpected allocator note\n{block}"
        );
    }

    assert_eq!(
        c.matches(NOTE).count(),
        NEEDS_ALLOC.len(),
        "one note per type that needs an allocator, and no other"
    );
}

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    fs::create_dir_all(&d).expect("dir");
    d
}

/// Decodes the bytes a type's wire form (XCDR2 little endian) without and
/// with an allocator, against the outcomes the note promises.
const PROGRAM: &str = r#"
#include "types.h"
#include <stdio.h>
#include <stdlib.h>

static void* arena(void* ctx, size_t n) { (void)ctx; return calloc(1, n); }

/* APPENDABLE (no annotation): each body is preceded by its DHEADER. */
static const uint8_t str_b[] = { 6, 0, 0, 0, 2, 0, 0, 0, 'a', 0 };
static const uint8_t seq_b[] = { 8, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0 };
static const uint8_t plain_b[] = { 12, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 };

int main(void) {
    int fails = 0;
    {
        Strs v;
        int r = strs_decode_cdr2_le(&v, str_b, sizeof str_b);
        if (r != -CDR_OUT_OF_MEMORY) { printf("FAIL strs no-alloc %d\n", r); fails++; }
        r = strs_decode_cdr2_le_alloc(&v, str_b, sizeof str_b, arena, NULL);
        if (r != (int)sizeof str_b) { printf("FAIL strs alloc %d\n", r); fails++; }
    }
    {
        Seqs v;
        int r = seqs_decode_cdr2_le(&v, seq_b, sizeof seq_b);
        if (r != -CDR_OUT_OF_MEMORY) { printf("FAIL seqs no-alloc %d\n", r); fails++; }
        r = seqs_decode_cdr2_le_alloc(&v, seq_b, sizeof seq_b, arena, NULL);
        if (r != (int)sizeof seq_b) { printf("FAIL seqs alloc %d\n", r); fails++; }
    }
    {
        Bounded v;
        int r = bounded_decode_cdr2_le(&v, seq_b, sizeof seq_b);
        if (r != -CDR_OUT_OF_MEMORY) { printf("FAIL bounded no-alloc %d\n", r); fails++; }
        r = bounded_decode_cdr2_le_alloc(&v, seq_b, sizeof seq_b, arena, NULL);
        if (r != (int)sizeof seq_b) { printf("FAIL bounded alloc %d\n", r); fails++; }
    }
    {
        Plain v;
        int r = plain_decode_cdr2_le(&v, plain_b, sizeof plain_b);
        if (r != (int)sizeof plain_b) { printf("FAIL plain %d\n", r); fails++; }
    }
    printf("fails=%d\n", fails);
    return fails;
}
"#;

#[test]
fn comment_matches_runtime_refusal() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler; allocator note not run");
        return;
    };
    let d = dir("c_decode_alloc_note");
    fs::write(d.join("types.h"), generate()).expect("header");
    let src = d.join("note.c");
    fs::write(&src, PROGRAM).expect("program");
    let exe = d.join("note");
    let build = Command::new(cc)
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("spawn");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "{out}");
    assert!(out.contains("fails=0"), "{out}");
}
