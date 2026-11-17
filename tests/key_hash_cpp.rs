// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated C++ headers (`PubSubType::compute_key`),
//! generated, compiled and run (RTPS 2.5 9.6.4.8), in the default and the
//! FastDDS-compatible flavours. Skipped, with a message, without `c++`.
//!
//! Expected values: the vectors of `tests/key_hash_rust.rs` (SPEC and
//! REF), `Colls` included: a bounded sequence is a `std::vector` holding
//! 0 to N elements (it was a `std::array` of N, which forced its own vector).

#![allow(
    clippy::expect_used,
    clippy::needless_raw_string_hashes,
    clippy::needless_pass_by_value
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::codegen::cpp::CppGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

const GOLDEN_IDL: &str = include_str!("fixtures/key_hash_golden.idl");
const EXT_IDL: &str = include_str!("fixtures/key_hash_ext.idl");
const INHERITANCE_IDL: &str = "
struct Base { @key long id; long payload; };
struct Derived : Base { @key short sub; long more; };
";

const COMMON_HPP: &str = r#"#include <array>
#include <cstdio>
#include <cstring>
static int fails = 0;
template <std::size_t N> std::array<char, N> A(const char* s) { std::array<char, N> a{}; std::strncpy(a.data(), s, N); return a; }
static void chk(const char* label, bool rc, const std::uint8_t kh[16], const char* expected) {
    char got[33];
    for (int i = 0; i < 16; ++i) std::snprintf(got + 2 * i, 3, "%02x", kh[i]);
    if (rc && std::strcmp(got, expected) == 0) std::printf("OK %s\n", label);
    else { std::printf("FAIL %s: rc %d got %s expected %s\n", label, int(rc), got, expected); ++fails; }
}
#define CHECK(label, Type, value, expected) do { std::uint8_t kh_[16]; bool rc_ = Type##PubSubType::compute_key(&(value), kh_); chk(label, rc_, kh_, expected); } while (0)
"#;
const GOLDEN_CPP: &str = r#"#include "HEADER"
#include "common.hpp"
using namespace P;
int main() {
    LongKey lk; lk.id = 0x12345678; lk.x = 9;
    SpecEx2 e2; e2.label = A<12>("BLUE"); e2.id = static_cast<int64_t>(0x123456789abcdef0ULL); e2.x = 1; e2.y = 2;
    SpecEx3 e3; e3.label = A<12>("BLUE"); e3.m_nested.m_long = 0x12345678; e3.m_nested.u = 1; e3.m_nested.w = 2; e3.x = 1; e3.y = 2;
    StrKey sk; sk.label = "BLUE"; sk.x = 1;
    StrKey8 s8; s8.label = A<8>("BLUE"); s8.x = 1;
    Str11 s11; s11.label = A<11>("BLUE"); s11.x = 1;
    Str12 s12; s12.label = A<12>("BLUE"); s12.x = 1;
    ShapeType sh; sh.color = A<128>("BLUE"); sh.x = 1; sh.y = 2; sh.shapesize = 3;
    TwoKey tk; tk.a = 1; tk.b = 2; tk.x = 3;
    TwoKeyId tki; tki.a = 1; tki.b = 2; tki.x = 3;
    Outer o; o.in_.k = 0x11223344; o.in_.u = 7; o.tag = 0x55; o.x = 1;
    Outer ou = o; ou.in_.u = -7; ou.x = 2;
    OuterNoKey onk; onk.p.a = 1; onk.p.b = 2; onk.x = 1;
    AlignKey ak; ak.t = 1; ak.v = 2;
    Key16 k16; k16.a = 1; k16.b = 2;
    CapKey ck; ck.t = 1; ck.v = 2; ck.c = 3;
    BigKey bk; bk.a = 1; bk.b = 2; bk.c = 3;
    CHECK("LongKey", LongKey, lk, "12345678000000000000000000000000");
    CHECK("SpecEx2", SpecEx2, e2, "f91a59e32e4535d9a69cd5d9f5b6e36e");
    CHECK("SpecEx3", SpecEx3, e3, "374b96e2e727237f016cc4cebb6eb71e");
    CHECK("StrKey", StrKey, sk, "cac217c318363f8ef1160eeedef9e886");
    CHECK("StrKey8", StrKey8, s8, "00000005424c55450000000000000000");
    CHECK("Str11", Str11, s11, "00000005424c55450000000000000000");
    CHECK("Str12", Str12, s12, "cac217c318363f8ef1160eeedef9e886");
    CHECK("ShapeType", ShapeType, sh, "cac217c318363f8ef1160eeedef9e886");
    CHECK("TwoKey", TwoKey, tk, "00000001000200000000000000000000");
    CHECK("TwoKeyId", TwoKeyId, tki, "00020000000000010000000000000000");
    CHECK("Outer", Outer, o, "11223344550000000000000000000000");
    CHECK("OuterU", Outer, ou, "11223344550000000000000000000000");
    CHECK("OuterNoKey", OuterNoKey, onk, "00000001000000020000000000000000");
    CHECK("AlignKey", AlignKey, ak, "01000000000000000000000200000000");
    CHECK("Key16", Key16, k16, "00000000000000010000000000000002");
    CHECK("CapKey", CapKey, ck, "01000000000000000000000200000003");
    CHECK("BigKey", BigKey, bk, "8898488e44b5e511051f83ce5e917bb8");
    {
        const int ns[] = {50, 51, 59, 60};
        const char* hs[] = {"fa67225c93f8071a5bbcf6df1bf9bfcd", "424bcbae3ad0fa3c57426e73ad1261af", "4b60eb0dd74deb107a2c9cad4e781130", "e7c43e8aa8fbb48e72bc38d01eb741a5"};
        for (int i = 0; i < 4; ++i) {
            StrKey lk2; lk2.label = std::string(ns[i], 'Q'); lk2.x = 1;
            CHECK("StrKeyLong", StrKey, lk2, hs[i]);
        }
    }
    return fails ? 1 : 0;
}
"#;
const EXT_CPP: &str = r#"#include "HEADER"
#include "common.hpp"
using namespace X;
int main() {
    Scalars sc; sc.b = true; sc.c = 'A'; sc.wc = L'Z'; sc.i8v = -2; sc.s = 0x0102; sc.d = 1.5; sc.us = 0xbeef; sc.f = -1.0f; sc.nk = 9;
    Enums en; en.c = Color::BLUE; en.sm = Small::S2; en.md = Mid::M1; en.f8 = static_cast<Flags8>(3); en.f32 = static_cast<Flags32>(2); en.f64 = static_cast<Flags64>(std::uint64_t(1) << 40); en.ml = -5;
    Colls co; co.sq = {1, 2}; co.bsq = {9, 8, 7}; co.arr = {1, 2, 3}; co.oct2 = {{{1, 2}, {3, 4}}}; co.ws = {L'h', static_cast<wchar_t>(0xe9), 0, 0}; co.n6 = A<6>("abc");
    Unions un; un.u._d = 2; un.u.b = "xy"; un.uk._d = Color::GREEN; un.uk._u.g = 2.5; un.tail = 0x77;
    Hashed ha; ha.color = 0x11; ha.shapesize = 0x22;
    FinalIds fi; fi.a = 0x0a0b0c0d; fi.b = 1;
    MutSeq ms; ms.a = 1; ms.b = 2; ms.c = "Q";
    CHECK("Scalars", Scalars, sc, "777ac52171394c4abb7947cd8f237982");
    CHECK("Enums", Enums, en, "e2265fcd85db8fc8343f51c33f672a8b");
    CHECK("Colls", Colls, co, "7b44e4a1ef2ab87d5bc2b6b8d253beca");
    CHECK("Unions", Unions, un, "45ec7942ba3bf7038d5ebc3abfc11459");
    CHECK("Hashed", Hashed, ha, "00000022000000110000000000000000");
    CHECK("FinalIds", FinalIds, fi, "010000000a0b0c0d0000000000000000");
    CHECK("MutSeq", MutSeq, ms, "ae5426ae810065bd695fddf3baf7b5c1");
    SeqChars scs; scs.c = {'x', 'y'}; scs.w = {L'A', L'B'}; scs.x = 1;
    CHECK("SeqChars", SeqChars, scs, "7cfc7a60d95c3037ff7785cfd6822d1c");
    SeqCharU scu; scu.c = {'x', 'y'}; scu.x = 1;
    CHECK("SeqCharU", SeqCharU, scu, "62a27c5288ec9d48152240ad0670d151");
    MH::ModHashed mh; mh.color = 0x11; mh.shapesize = 0x22;
    CHECK("ModHashed", MH::ModHashed, mh, "00000022000000110000000000000000");
    return fails ? 1 : 0;
}
"#;
const INHERITANCE_CPP: &str = r#"#include "HEADER"
#include "common.hpp"
int main() {
    Derived d; d.id = 7; d.payload = 1; d.sub = 3; d.more = 4;
    CHECK("Derived", Derived, d, "00000007000300000000000000000000");
    if (!DerivedPubSubType::has_key()) { std::printf("FAIL has_key\n"); ++fails; }
    return fails ? 1 : 0;
}
"#;
const COMPAT_CPP: &str = r#"#include "HEADER"
#include "common.hpp"
using namespace P;
int main() {
    LongKey lk; lk.id(0x12345678); lk.x(9);
    Inner in; in.k(0x11223344); in.u(7);
    Outer o; o.in_(in); o.tag(0x55); o.x(1);
    SpecEx3 e3; Nested n; n.m_long(0x12345678); n.u(1); n.w(2); e3.label(A<12>("BLUE")); e3.m_nested(n); e3.x(1); e3.y(2);
    StrKey sk; sk.label("BLUE"); sk.x(1);
    CHECK("LongKey", LongKey, lk, "12345678000000000000000000000000");
    CHECK("Outer", Outer, o, "11223344550000000000000000000000");
    CHECK("SpecEx3", SpecEx3, e3, "374b96e2e727237f016cc4cebb6eb71e");
    CHECK("StrKey", StrKey, sk, "cac217c318363f8ef1160eeedef9e886");
    return fails ? 1 : 0;
}
"#;

fn find_cxx() -> Option<&'static str> {
    ["c++", "g++", "clang++"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn generate(generator: &CppGenerator, idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    generator.generate(&ast).expect("codegen")
}

fn compile_and_run(cxx: &str, dir: &Path, name: &str, header: String, program: &str) -> usize {
    let header_name = format!("{name}.hpp");
    fs::write(dir.join(&header_name), header).expect("header");
    fs::write(dir.join("common.hpp"), COMMON_HPP).expect("common");
    let src = dir.join(format!("{name}.cpp"));
    fs::write(&src, program.replace("HEADER", &header_name)).expect("program");
    let exe = dir.join(name);
    let out = Command::new(cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("c++");
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
fn cpp_key_hash_matches_the_golden_vectors() {
    let Some(cxx) = find_cxx() else {
        eprintln!("no C++ compiler found; C++ KeyHash test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key_hash_cpp");
    fs::create_dir_all(&dir).expect("dir");
    let plain = CppGenerator::new();
    let compat = CppGenerator::with_fastdds_compat();
    let golden = generate(&plain, GOLDEN_IDL);
    assert_eq!(compile_and_run(cxx, &dir, "golden", golden, GOLDEN_CPP), 21);
    let ext = generate(&plain, EXT_IDL);
    assert_eq!(compile_and_run(cxx, &dir, "ext", ext, EXT_CPP), 10);
    let inh = generate(&plain, INHERITANCE_IDL);
    assert_eq!(compile_and_run(cxx, &dir, "inh", inh, INHERITANCE_CPP), 1);
    let golden_compat = generate(&compat, GOLDEN_IDL);
    assert_eq!(
        compile_and_run(cxx, &dir, "compat", golden_compat, COMPAT_CPP),
        4
    );
}
