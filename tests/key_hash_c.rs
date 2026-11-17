// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated C headers, generated, compiled and run
//! (RTPS 2.5 9.6.4.8). Skipped, with a message, without a C compiler.
//!
//! Expected values: the vectors of `tests/key_hash_rust.rs` (SPEC and
//! REF), the extended set whole (`Colls` with its two-dimensional
//! `oct2`). A bounded string is a `char[N+1]`, a bounded wstring a
//! `wchar_t[N+1]`. The types of the fixtures live in modules: their C names
//! carry the module path (`P_LongKey`, `p_long_key_compute_key`).

#![allow(
    clippy::expect_used,
    clippy::needless_raw_string_hashes,
    clippy::needless_pass_by_value
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::codegen::c::CGenerator;
use hddsgen::codegen::CStandard;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

const GOLDEN_IDL: &str = include_str!("fixtures/key_hash_golden.idl");
const EXT_IDL: &str = include_str!("fixtures/key_hash_ext.idl");
const INHERITANCE_IDL: &str = "
struct Base { @key long id; long payload; };
struct Derived : Base { @key short sub; long more; };
";

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
int main(void) {
    char blue[] = "BLUE";
    P_LongKey lk = {0x12345678, 9};
    P_SpecEx2 e2; P_SpecEx3 e3; P_StrKey sk; P_StrKey8 s8; P_Str11 s11; P_Str12 s12; P_ShapeType sh;
    P_TwoKey tk = {1, 2, 3}; P_TwoKeyId tki = {1, 2, 3};
    P_Outer o; P_Outer ou; P_OuterNoKey onk; P_AlignKey ak = {1, 2}; P_Key16 k16 = {1, 2}; P_CapKey ck = {1, 2, 3}; P_BigKey bk = {1, 2, 3};
    strcpy(e2.label, blue); e2.id = (int64_t)0x123456789abcdef0ULL; e2.x = 1; e2.y = 2;
    strcpy(e3.label, blue); e3.m_nested.m_long = 0x12345678; e3.m_nested.u = 1; e3.m_nested.w = 2; e3.x = 1; e3.y = 2;
    sk.label = blue; sk.x = 1;
    strcpy(s8.label, blue); s8.x = 1;
    strcpy(s11.label, blue); s11.x = 1;
    strcpy(s12.label, blue); s12.x = 1;
    strcpy(sh.color, blue); sh.x = 1; sh.y = 2; sh.shapesize = 3;
    o.in_.k = 0x11223344; o.in_.u = 7; o.tag = 0x55; o.x = 1;
    ou = o; ou.in_.u = -7; ou.x = 2;
    onk.p.a = 1; onk.p.b = 2; onk.x = 1;
    CHECK("LongKey", p_long_key_compute_key, lk, "12345678000000000000000000000000");
    CHECK("SpecEx2", p_spec_ex2_compute_key, e2, "f91a59e32e4535d9a69cd5d9f5b6e36e");
    CHECK("SpecEx3", p_spec_ex3_compute_key, e3, "374b96e2e727237f016cc4cebb6eb71e");
    CHECK("StrKey", p_str_key_compute_key, sk, "cac217c318363f8ef1160eeedef9e886");
    CHECK("StrKey8", p_str_key8_compute_key, s8, "00000005424c55450000000000000000");
    CHECK("Str11", p_str11_compute_key, s11, "00000005424c55450000000000000000");
    CHECK("Str12", p_str12_compute_key, s12, "cac217c318363f8ef1160eeedef9e886");
    CHECK("ShapeType", p_shape_type_compute_key, sh, "cac217c318363f8ef1160eeedef9e886");
    CHECK("TwoKey", p_two_key_compute_key, tk, "00000001000200000000000000000000");
    CHECK("TwoKeyId", p_two_key_id_compute_key, tki, "00020000000000010000000000000000");
    CHECK("Outer", p_outer_compute_key, o, "11223344550000000000000000000000");
    CHECK("OuterU", p_outer_compute_key, ou, "11223344550000000000000000000000");
    CHECK("OuterNoKey", p_outer_no_key_compute_key, onk, "00000001000000020000000000000000");
    CHECK("AlignKey", p_align_key_compute_key, ak, "01000000000000000000000200000000");
    CHECK("Key16", p_key16_compute_key, k16, "00000000000000010000000000000002");
    CHECK("CapKey", p_cap_key_compute_key, ck, "01000000000000000000000200000003");
    CHECK("BigKey", p_big_key_compute_key, bk, "8898488e44b5e511051f83ce5e917bb8");
    {
        static char q[61];
        static const struct { int n; const char* h; } longs[] = {{50, "fa67225c93f8071a5bbcf6df1bf9bfcd"}, {51, "424bcbae3ad0fa3c57426e73ad1261af"}, {59, "4b60eb0dd74deb107a2c9cad4e781130"}, {60, "e7c43e8aa8fbb48e72bc38d01eb741a5"}};
        int i;
        for (i = 0; i < 4; ++i) {
            memset(q, 'Q', (size_t)longs[i].n); q[longs[i].n] = 0;
            sk.label = q;
            CHECK("StrKeyLong", p_str_key_compute_key, sk, longs[i].h);
        }
    }
    return fails ? 1 : 0;
}
"#;
const EXT_C: &str = r#"#include "HEADER"
#include "common.h"
int main(void) {
    X_Scalars sc; X_Enums en; X_Colls co; X_Unions un; X_Hashed ha; X_FinalIds fi; X_MutSeq ms; X_Defaults df; MH_ModHashed mh;
    X_SeqChars scs; X_SeqCharU scu; char cxy[2] = {'x', 'y'}; wchar_t wab[2] = {L'A', L'B'};
    int32_t sq[2] = {1, 2}; uint8_t bsq[3] = {9, 8, 7};
    char abc[] = "abc"; char xy[] = "xy"; char q[] = "Q";
    sc.b = true; sc.c = 'A'; sc.wc = L'Z'; sc.i8v = -2; sc.s = 0x0102; sc.d = 1.5; sc.us = 0xbeef; sc.f = -1.0f; sc.nk = 9;
    en.c = X_COLOR_BLUE; en.sm = X_SMALL_S2; en.md = X_MID_M1; en.f8 = 3; en.f32 = 2; en.f64 = (uint64_t)1 << 40; en.ml = -5;
    co.sq.data = sq; co.sq.len = 2; co.bsq.data = bsq; co.bsq.len = 3; co.arr[0] = 1; co.arr[1] = 2; co.arr[2] = 3;
    co.oct2[0][0] = 1; co.oct2[0][1] = 2; co.oct2[1][0] = 3; co.oct2[1][1] = 4;
    co.ws[0] = 0x68; co.ws[1] = 0xe9; co.ws[2] = 0; strcpy(co.n6, abc);
    un.u._d = 2; un.u._u.b = xy; un.uk._d = X_COLOR_GREEN; un.uk._u.g = 2.5; un.tail = 0x77;
    ha.color = 0x11; ha.shapesize = 0x22;
    fi.a = 0x0a0b0c0d; fi.b = 1;
    ms.a = 1; ms.b = 2; ms.c = q;
    CHECK("Scalars", x_scalars_compute_key, sc, "777ac52171394c4abb7947cd8f237982");
    CHECK("Enums", x_enums_compute_key, en, "e2265fcd85db8fc8343f51c33f672a8b");
    CHECK("Colls", x_colls_compute_key, co, "7b44e4a1ef2ab87d5bc2b6b8d253beca");
    CHECK("Unions", x_unions_compute_key, un, "45ec7942ba3bf7038d5ebc3abfc11459");
    CHECK("Hashed", x_hashed_compute_key, ha, "00000022000000110000000000000000");
    CHECK("FinalIds", x_final_ids_compute_key, fi, "010000000a0b0c0d0000000000000000");
    CHECK("MutSeq", x_mut_seq_compute_key, ms, "ae5426ae810065bd695fddf3baf7b5c1");
    scs.c.data = cxy; scs.c.len = 2; scs.w.data = wab; scs.w.len = 2; scs.x = 1;
    CHECK("SeqChars", x_seq_chars_compute_key, scs, "7cfc7a60d95c3037ff7785cfd6822d1c");
    scu.c.data = cxy; scu.c.len = 2; scu.x = 1;
    CHECK("SeqCharU", x_seq_char_u_compute_key, scu, "62a27c5288ec9d48152240ad0670d151");
    mh.color = 0x11; mh.shapesize = 0x22;
    CHECK("ModHashed", m_h_mod_hashed_compute_key, mh, "00000022000000110000000000000000");
    df.u._d = 1; df.u._u.a = 5;
    CHECK("DefaultsCase", x_defaults_compute_key, df, "00010000000000050000000000000000");
    df.u._d = 7; df.u._u.o = 0x42;
    CHECK("DefaultsDefault", x_defaults_compute_key, df, "00074200000000000000000000000000");
    return fails ? 1 : 0;
}
"#;
const INHERITANCE_C: &str = r#"#include "HEADER"
#include "common.h"
int main(void) {
    Derived d; uint8_t a[16], b[16];
    d.base.id = 7; d.base.payload = 1; d.sub = 3; d.more = 4;
    CHECK("Derived", derived_compute_key, d, "00000007000300000000000000000000");
    d.more = 5; d.base.payload = 9;
    derived_compute_key(&d, a); d.sub = 4; derived_compute_key(&d, b);
    if (memcmp(a, b, 16) == 0) { printf("FAIL key member did not move the hash\n"); fails++; }
    if (derived_has_key() != 1) { printf("FAIL has_key\n"); fails++; }
    return fails ? 1 : 0;
}
"#;

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn generate(idl: &str, standard: CStandard) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    CGenerator::with_standard(standard)
        .generate(&ast)
        .expect("codegen")
}

/// Compile `program` (which includes `header`) and run it; return the
/// number of `OK` lines.
fn compile_and_run(
    cc: &str,
    dir: &Path,
    name: &str,
    std: &str,
    header: (&str, String),
    program: &str,
) -> usize {
    fs::write(dir.join(header.0), header.1).expect("header");
    fs::write(dir.join("common.h"), COMMON_H).expect("common");
    let src = dir.join(format!("{name}.c"));
    fs::write(&src, program.replace("HEADER", header.0)).expect("program");
    let exe = dir.join(name);
    let out = Command::new(cc)
        .arg(format!("-std={std}"))
        .args(["-Wall", "-Werror=implicit-function-declaration", "-o"])
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
fn c_key_hash_matches_the_golden_vectors() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C KeyHash test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key_hash_c");
    fs::create_dir_all(&dir).expect("dir");

    let golden = ("kh_golden.h", generate(GOLDEN_IDL, CStandard::C99));
    assert_eq!(
        compile_and_run(cc, &dir, "golden", "c99", golden, GOLDEN_C),
        21
    );
    let ext = ("kh_ext.h", generate(EXT_IDL, CStandard::C99));
    assert_eq!(compile_and_run(cc, &dir, "ext", "c99", ext, EXT_C), 12);
    let inh = ("kh_inh.h", generate(INHERITANCE_IDL, CStandard::C99));
    assert_eq!(
        compile_and_run(cc, &dir, "inh", "c99", inh, INHERITANCE_C),
        1
    );
}
