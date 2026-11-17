// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated C-micro headers, generated, compiled
//! against `sdk/c-micro/include` and run (RTPS 2.5 9.6.4.8). Skipped, with a
//! message, without a C compiler.
//!
//! This backend cannot yet declare module-scoped nested types (a member
//! naming a type of its own module); the golden set is therefore used
//! without its module, and the extended set is that of the Rust micro
//! backend (`MICRO_EXT` of `tests/key_hash_rust.rs`: `Colls` with a
//! `wstring<4>`, `U` with a string case) without typedefs. Expected
//! values: the vectors of `tests/key_hash_rust.rs`.

#![allow(
    clippy::expect_used,
    clippy::needless_raw_string_hashes,
    clippy::needless_pass_by_value
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::{Backend, Parser};

const GOLDEN_IDL: &str = include_str!("fixtures/key_hash_golden.idl");
const EXT_IDL: &str = r#"enum Color { RED, GREEN, BLUE };
@bit_bound(8) enum Small { S0, S1, S2 };
union U switch (long) { case 1: long a; case 2: string b; };
union UK switch (@key Color) { case RED: long r; case GREEN: double g; case BLUE: octet bl; };
@final struct Scalars { @key boolean b; @key char c; @key wchar wc; @key int8 i8v; @key short s; @key double d; @key unsigned short us; @key float f; long nk; };
@final struct Colls { @key sequence<long, 4> sq; @key sequence<octet, 4> bsq; @key long arr[3]; @key wstring<4> ws; };
@final struct Unions { @key U u; @key UK uk; @key octet tail; };
@autoid(HASH) @final struct Hashed { @key long color; @key long shapesize; };
@final struct FinalIds { @id(5) @key long a; @id(1) @key octet b; };
@mutable struct MutSeq { @key long a; @key short b; @key string c; };
@final struct SeqChars { @key sequence<char, 4> c; @key sequence<wchar, 3> w; long x; };
@final struct Enums2 { @key Color c; @key Small sm; };
@autoid(HASH) module MH {
  @final struct ModHashed { @key long color; @key long shapesize; };
};
"#;
const COMMON_H: &str = r#"#include <stdio.h>
static int fails = 0;
static void chk(const char* label, int rc, const uint8_t kh[16], const char* expected) {
    char got[33];
    int i;
    for (i = 0; i < 16; ++i) { snprintf(got + 2 * i, 3, "%02x", kh[i]); }
    if (rc == 1 && strcmp(got, expected) == 0) { printf("OK %s\n", label); }
    else { printf("FAIL %s: rc %d got %s expected %s\n", label, rc, got, expected); fails++; }
}
#define CHECK(label, fn, value, expected) do { uint8_t kh_[16]; int rc_ = fn(&(value), kh_); chk(label, rc_, kh_, expected); } while (0)
"#;
const GOLDEN_C: &str = r#"#include "HEADER"
#include "common.h"
static void set(char* dst, size_t n, const char* s) { memset(dst, 0, n); strncpy(dst, s, n); }
int main(void) {
    LongKey lk = {0x12345678, 9};
    SpecEx2 e2; SpecEx3 e3; StrKey sk; StrKey8 s8; Str11 s11; Str12 s12; ShapeType sh;
    TwoKey tk = {1, 2, 3}; TwoKeyId tki = {1, 2, 3};
    Outer o; Outer ou; OuterNoKey onk; AlignKey ak = {1, 2}; Key16 k16 = {1, 2}; CapKey ck = {1, 2, 3}; BigKey bk = {1, 2, 3};
    set(e2.label, sizeof(e2.label), "BLUE"); e2.id = (int64_t)0x123456789abcdef0ULL; e2.x = 1; e2.y = 2;
    set(e3.label, sizeof(e3.label), "BLUE"); e3.m_nested.m_long = 0x12345678; e3.m_nested.u = 1; e3.m_nested.w = 2; e3.x = 1; e3.y = 2;
    set(sk.label, sizeof(sk.label), "BLUE"); sk.x = 1;
    set(s8.label, sizeof(s8.label), "BLUE"); s8.x = 1;
    set(s11.label, sizeof(s11.label), "BLUE"); s11.x = 1;
    set(s12.label, sizeof(s12.label), "BLUE"); s12.x = 1;
    set(sh.color, sizeof(sh.color), "BLUE"); sh.x = 1; sh.y = 2; sh.shapesize = 3;
    o.in_.k = 0x11223344; o.in_.u = 7; o.tag = 0x55; o.x = 1;
    ou = o; ou.in_.u = -7; ou.x = 2;
    onk.p.a = 1; onk.p.b = 2; onk.x = 1;
    CHECK("LongKey", long_key_compute_key, lk, "12345678000000000000000000000000");
    CHECK("SpecEx2", spec_ex2_compute_key, e2, "f91a59e32e4535d9a69cd5d9f5b6e36e");
    CHECK("SpecEx3", spec_ex3_compute_key, e3, "374b96e2e727237f016cc4cebb6eb71e");
    CHECK("StrKey", str_key_compute_key, sk, "cac217c318363f8ef1160eeedef9e886");
    CHECK("StrKey8", str_key8_compute_key, s8, "00000005424c55450000000000000000");
    CHECK("Str11", str11_compute_key, s11, "00000005424c55450000000000000000");
    CHECK("Str12", str12_compute_key, s12, "cac217c318363f8ef1160eeedef9e886");
    CHECK("ShapeType", shape_type_compute_key, sh, "cac217c318363f8ef1160eeedef9e886");
    CHECK("TwoKey", two_key_compute_key, tk, "00000001000200000000000000000000");
    CHECK("TwoKeyId", two_key_id_compute_key, tki, "00020000000000010000000000000000");
    CHECK("Outer", outer_compute_key, o, "11223344550000000000000000000000");
    CHECK("OuterU", outer_compute_key, ou, "11223344550000000000000000000000");
    CHECK("OuterNoKey", outer_no_key_compute_key, onk, "00000001000000020000000000000000");
    CHECK("AlignKey", align_key_compute_key, ak, "01000000000000000000000200000000");
    CHECK("Key16", key16_compute_key, k16, "00000000000000010000000000000002");
    CHECK("CapKey", cap_key_compute_key, ck, "01000000000000000000000200000003");
    CHECK("BigKey", big_key_compute_key, bk, "8898488e44b5e511051f83ce5e917bb8");
    {
        static const struct { int n; const char* h; } longs[] = {{50, "fa67225c93f8071a5bbcf6df1bf9bfcd"}, {51, "424bcbae3ad0fa3c57426e73ad1261af"}, {59, "4b60eb0dd74deb107a2c9cad4e781130"}, {60, "e7c43e8aa8fbb48e72bc38d01eb741a5"}};
        int i;
        for (i = 0; i < 4; ++i) {
            memset(sk.label, 0, sizeof(sk.label));
            memset(sk.label, 'Q', (size_t)longs[i].n);
            CHECK("StrKeyLong", str_key_compute_key, sk, longs[i].h);
        }
    }
    return fails ? 1 : 0;
}
"#;
const EXT_C: &str = r#"#include "HEADER"
#include "common.h"
int main(void) {
    Scalars sc; Colls co; Unions un; Hashed ha; FinalIds fi; MutSeq ms; Enums2 en;
    sc.b = 1; sc.c = 'A'; sc.wc = 0x5a; sc.i8v = -2; sc.s = 0x0102; sc.d = 1.5; sc.us = 0xbeef; sc.f = -1.0f; sc.nk = 9;
    co.sq.data[0] = 1; co.sq.data[1] = 2; co.sq.count = 2; co.bsq.data[0] = 9; co.bsq.data[1] = 8; co.bsq.data[2] = 7; co.bsq.count = 3;
    co.arr[0] = 1; co.arr[1] = 2; co.arr[2] = 3; co.ws[0] = 0x68; co.ws[1] = 0xe9; co.ws[2] = 0;
    un.u._d = 2; strcpy(un.u._u.b, "xy"); un.uk._d = COLOR_GREEN; un.uk._u.g = 2.5; un.tail = 0x77;
    ha.color = 0x11; ha.shapesize = 0x22;
    fi.a = 0x0a0b0c0d; fi.b = 1;
    ms.a = 1; ms.b = 2; memset(ms.c, 0, sizeof(ms.c)); ms.c[0] = 'Q';
    en.c = COLOR_BLUE; en.sm = SMALL_S2;
    CHECK("Scalars", scalars_compute_key, sc, "777ac52171394c4abb7947cd8f237982");
    CHECK("Colls", colls_compute_key, co, "c7774e0d0f0225342bcd922a9d9d70bb");
    CHECK("Unions", unions_compute_key, un, "45ec7942ba3bf7038d5ebc3abfc11459");
    CHECK("Hashed", hashed_compute_key, ha, "00000022000000110000000000000000");
    CHECK("FinalIds", final_ids_compute_key, fi, "010000000a0b0c0d0000000000000000");
    CHECK("MutSeq", mut_seq_compute_key, ms, "ae5426ae810065bd695fddf3baf7b5c1");
    {
        SeqChars scs; memset(&scs, 0, sizeof(scs)); scs.c.data[0] = 'x'; scs.c.data[1] = 'y'; scs.c.count = 2;
        scs.w.data[0] = 0x41; scs.w.data[1] = 0x42; scs.w.count = 2; scs.x = 1;
        CHECK("SeqChars", seq_chars_compute_key, scs, "7cfc7a60d95c3037ff7785cfd6822d1c");
    }
    CHECK("Enums2", enums2_compute_key, en, "00000002020000000000000000000000");
    {
        MH_ModHashed mh; mh.color = 0x11; mh.shapesize = 0x22;
        CHECK("ModHashed", m_h__mod_hashed_compute_key, mh, "00000022000000110000000000000000");
    }
    return fails ? 1 : 0;
}
"#;

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::CMicro.generator().generate(&ast).expect("codegen")
}

fn compile_and_run(cc: &str, dir: &Path, name: &str, header: String, program: &str) -> usize {
    let header_name = format!("{name}.h");
    fs::write(dir.join(&header_name), header).expect("header");
    fs::write(dir.join("common.h"), COMMON_H).expect("common");
    let src = dir.join(format!("{name}.c"));
    fs::write(&src, program.replace("HEADER", &header_name)).expect("program");
    let exe = dir.join(name);
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let out = Command::new(cc)
        .args(["-std=c99", "-Wall", "-I"])
        .arg(&include)
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "{name}: compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "{name}:\n{stdout}");
    stdout.lines().filter(|l| l.starts_with("OK ")).count()
}

#[test]
fn c_micro_key_hash_matches_the_golden_vectors() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C-micro KeyHash test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key_hash_c_micro");
    fs::create_dir_all(&dir).expect("dir");
    let golden_flat: String = GOLDEN_IDL
        .lines()
        .filter(|l| !l.starts_with("module P {") && *l != "};")
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        compile_and_run(cc, &dir, "golden", generate(&golden_flat), GOLDEN_C),
        21
    );
    assert_eq!(
        compile_and_run(cc, &dir, "ext", generate(EXT_IDL), EXT_C),
        9
    );
}
