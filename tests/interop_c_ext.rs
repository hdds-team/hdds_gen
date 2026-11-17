// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn find_cc() -> Option<String> {
    for cc in &["clang", "gcc"] {
        if Command::new(cc).arg("--version").output().is_ok() {
            return Some((*cc).to_string());
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

fn workspace_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hdds_gen_it_ext_{}_{}", std::process::id(), 1));
    let _ = fs::create_dir_all(&dir);
    dir
}

fn write_idl_file(dir: &Path) -> PathBuf {
    let idl = r"
        bitset B {
            bitfield<3> mode;
            bitfield<5> value;
        };

        bitmask Flags { A, B };

        struct Inner {
            sequence<int32_t> nums;
            sequence<string> names;
            map<string, int32_t> dict;
        };

        struct Arr {
            string  names[2];
        };

        // `sequence<wchar>` is read as a wstring (the parser holds
        // `wstring<N>` as a sequence of the wchar keyword, see
        // data_model::is_string_like); a sequence of wide characters is
        // a sequence of a typedef of wchar.
        typedef wchar Glyph;

        struct WideStuff {
            wchar   glyph;
            wstring title;
            sequence<Glyph> glyphs;
            fixed<10, 2> amount;
        };

        struct Nested {
            Inner inner;
            B     bits;
            Flags flags;
            Arr   arr;
            WideStuff wide;
        };
    ";
    let idl_path = dir.join("ext.idl");
    fs::write(&idl_path, idl).expect("write idl");
    idl_path
}

fn generate_header(idl_path: &Path, out_h: &Path) {
    assert!(
        run_idl_gen(&[
            "gen",
            "c",
            idl_path.to_str().expect("idl path"),
            "-o",
            out_h.to_str().expect("header path")
        ]),
        "hddsgen C failed"
    );
}

fn write_driver(dir: &Path) -> PathBuf {
    let c_prog = r#"
    #include <stdint.h>
    #include <stdlib.h>
    #include <string.h>
    #include <wchar.h>
    #include "ext.h"

    static void* xmalloc(void* ctx, size_t n) { (void)ctx; return malloc(n); }

    int main(void) {
        Nested n = {0};
        // Fill sequences
        static int32_t nums[3] = {1,2,3};
        n.inner.nums.data = nums; n.inner.nums.len = 3;
        static char *names_in[2];
        names_in[0] = (char*)"aa"; names_in[1] = (char*)"bb";
        n.inner.names.data = names_in; n.inner.names.len = 2;
        // Map: 2 entries
        static char *keys[2];
        static int32_t vals[2];
        keys[0] = (char*)"k1"; keys[1] = (char*)"k2";
        vals[0] = 10; vals[1] = 20;
        n.inner.dict.keys = keys; n.inner.dict.values = vals; n.inner.dict.len = 2;

        // Bitset
        B_set_mode(&n.bits, 5);
        B_set_value(&n.bits, 17);
        // Bitmask
        n.flags = FLAGS_A | FLAGS_B;

        // Arrays
        n.arr.names[0] = (char*)"x";
        n.arr.names[1] = (char*)"y";

        // Wide / fixed data
        n.wide.glyph = L'Ω';
        n.wide.title = (wchar_t*)L"Phase8";
        static wchar_t glyphs_in[2] = {L'Ω', L'β'};
        n.wide.glyphs.data = glyphs_in;
        n.wide.glyphs.len = 2;
        n.wide.amount.high = 0;
        n.wide.amount.low = 123456789ULL;

        uint8_t buf[4096];
        int enc = nested_encode_cdr2_le(&n, buf, sizeof(buf));
        if (enc < 0) return 1;

        Nested out = {0};
        // The decoder takes the storage of every string, sequence and map
        // from the allocator (malloc here; the process exits right after).
        int dec = nested_decode_cdr2_le_alloc(&out, buf, (size_t)enc, xmalloc, NULL);
        if (dec != enc) return 2;
        // Check sequences
        if (out.inner.nums.len != 3 || out.inner.nums.data[0]!=1 || out.inner.nums.data[2]!=3) return 3;
        if (strcmp(out.inner.names.data[0],"aa")!=0 || strcmp(out.inner.names.data[1],"bb")!=0) return 4;
        // Check map
        if (strcmp(out.inner.dict.keys[0],"k1")!=0 || out.inner.dict.values[1]!=20) return 5;
        // Check bitset/mask
        if (B_get_mode(&out.bits) != 5 || B_get_value(&out.bits) != 17) return 6;
        if ((uint64_t)out.flags == 0) return 7;
        // Check arrays
        if (strcmp(out.arr.names[0],"x")!=0 || strcmp(out.arr.names[1],"y")!=0) return 8;
        if (out.wide.glyph != L'Ω') return 9;
        if (wcscmp(out.wide.title, L"Phase8") != 0) return 10;
        if (out.wide.glyphs.len != 2 || out.wide.glyphs.data[1] != L'β') return 11;
        if (out.wide.amount.high != 0 || out.wide.amount.low != 123456789ULL) return 12;

        return 0;
    }
    "#;
    let c_path = dir.join("ext.c");
    fs::write(&c_path, c_prog).expect("write c");
    c_path
}

fn compile_and_run(cc: &str, dir: &Path, c_path: &Path, exe: &Path) {
    let status = Command::new(cc)
        .args(["-std=c11", "-Wall", "-Wextra"])
        .arg("-I")
        .arg(dir)
        .arg(c_path)
        .arg("-o")
        .arg(exe)
        .status()
        .expect("compile c");
    assert!(status.success(), "C compile failed");

    let out = Command::new(exe).status().expect("run c exe");
    assert!(out.success(), "interop extended C program failed");
}

#[test]
fn interop_c_extended() {
    let Some(cc) = find_cc() else {
        eprintln!("C compiler not found; skipping interop C ext test");
        return;
    };

    let dir = workspace_dir();
    let idl_path = write_idl_file(&dir);
    let header_path = dir.join("ext.h");
    generate_header(&idl_path, &header_path);

    let c_path = write_driver(&dir);
    let exe = dir.join("ext_bin");
    compile_and_run(&cc, &dir, &c_path, &exe);
}
