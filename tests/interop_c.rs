// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

#![allow(clippy::expect_used)]

use std::fs;
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

#[test]
fn interop_c_struct_and_union() {
    let Some(cc) = find_cc() else {
        eprintln!("C compiler not found; skipping interop C test");
        return;
    };

    let base = std::env::temp_dir();
    let dir = base.join(format!("hdds_gen_it_{}_{}", std::process::id(), 1));
    let _ = fs::create_dir_all(&dir);

    // Minimal IDL exercising structs (string) and union (int/string/default)
    let idl = r"
        struct Message {
            int32_t id;
            string  content;
        };

        union Data switch(int32_t) {
            case 1: int32_t integer_value;
            case 2: string string_value;
            default: uint8_t raw_data;
        };
    ";
    let idl_path = dir.join("interop.idl");
    fs::write(&idl_path, idl).expect("write idl");

    // Generate C header
    let out_h = dir.join("interop.h");
    assert!(
        run_idl_gen(&[
            "gen",
            "c",
            idl_path.to_str().expect("utf-8 path"),
            "-o",
            out_h.to_str().expect("utf-8 path")
        ]),
        "hddsgen C failed"
    );

    // Write C test program
    let c_prog = r#"
    #include <stdint.h>
    #include <stdlib.h>
    #include <string.h>
    #include "interop.h"

    int main(void) {
        // ===== Struct Message roundtrip =====
        Message m = {0};
        m.id = 42;
        m.content = (char*)"Hello";

        uint8_t buf[1024];
        int enc = message_encode_cdr2_le(&m, buf, sizeof(buf));
        if (enc < 0) return 1;

        Message out = {0};
        // the decoder takes the string's storage from an arena
        static uint8_t heap[1024];
        hdds_x2c_arena ar;
        hdds_x2c_arena_init(&ar, heap, sizeof heap);
        int dec = message_decode_cdr2_le_alloc(&out, buf, (size_t)enc, hdds_x2c_arena_alloc, &ar);
        if (dec != enc) return 2;
        if (out.id != 42) return 3;
        if (strcmp(out.content, "Hello") != 0) return 4;

        // ===== Union Data roundtrip (int case) =====
        Data d = {0};
        d._d = 1; // integer_value
        d._u.integer_value = 1234;
        int enc_u = data_encode_cdr2_le(&d, buf, sizeof(buf));
        if (enc_u < 0) return 10;
        Data dout = {0};
        int dec_u = data_decode_cdr2_le(&dout, buf, (size_t)enc_u);
        if (dec_u != enc_u) return 11;
        if (dout._d != 1 || dout._u.integer_value != 1234) return 12;

        // ===== Union Data roundtrip (string case) =====
        d._d = 2;
        d._u.string_value = (char*)"World";
        enc_u = data_encode_cdr2_le(&d, buf, sizeof(buf));
        if (enc_u < 0) return 20;
        Data dout2 = {0};
        // without an allocator, a string has no storage: refused
        if (data_decode_cdr2_le(&dout2, buf, (size_t)enc_u) != -CDR_OUT_OF_MEMORY) return 23;
        hdds_x2c_arena_init(&ar, heap, sizeof heap);
        dec_u = data_decode_cdr2_le_alloc(&dout2, buf, (size_t)enc_u, hdds_x2c_arena_alloc, &ar);
        if (dec_u != enc_u) return 21;
        if (dout2._d != 2 || strcmp(dout2._u.string_value, "World") != 0) return 22;

        // ===== Union Data roundtrip (default case) =====
        d._d = 0;
        d._u.raw_data = 0xAB;
        enc_u = data_encode_cdr2_le(&d, buf, sizeof(buf));
        if (enc_u < 0) return 30;
        Data dout3 = {0};
        dec_u = data_decode_cdr2_le(&dout3, buf, (size_t)enc_u);
        if (dec_u != enc_u) return 31;
        if (dout3._d != 0 || dout3._u.raw_data != 0xAB) return 32;

        return 0;
    }
    "#;
    let c_path = dir.join("interop.c");
    fs::write(&c_path, c_prog).expect("write c");

    // Compile
    let exe = dir.join("interop_bin");
    let status = Command::new(&cc)
        .args(["-std=c11", "-Wall", "-Wextra"]) // strict flags
        .arg("-I")
        .arg(dir)
        .arg(c_path.to_str().expect("utf-8 path"))
        .arg("-o")
        .arg(exe.to_str().expect("utf-8 path"))
        .status()
        .expect("compile c");
    assert!(status.success(), "C compile failed");

    // Run
    let out = Command::new(&exe).status().expect("run c exe");
    assert!(out.success(), "interop C program failed");
}
