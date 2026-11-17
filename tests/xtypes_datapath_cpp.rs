// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated C++ code against a foreign oracle.
//!
//! `fixtures/xtypes_datapath/corpus.idl` is compiled by hddsgen into a C++
//! header, in the default and the FastDDS-compatible flavours; one sample of
//! each type (the values `cyclone_probe.c` writes) is encoded and must give
//! the XCDR2 body Cyclone DDS 0.11 serializes (`cyclone_vectors.txt`, never
//! an hddsgen encoder), one byte short the encoder must fail, and Cyclone's
//! body must decode to the sample. The C++ backend has no XCDR1 encoder: the
//! xcdr1 lines are not used. Rules no sample reaches (unknown members,
//! bounds, invalid booleans, appendable evolution) are checked on bodies
//! derived from Cyclone's by hand (`extras`, `REFUSED`).
//!
//! Every sample of the corpus is expressed; none is excluded. `HoldU_def`
//! (a `default` discriminator of 9) is encoded too: the C++ union keeps its
//! discriminator.
//!
//! XCDR1 (`decode_cdr1_le`): every XCDR1 body of a sample in the foreign
//! and specification-built fixtures (`xcdr1_common`) decodes to the sample
//! (its member checks hold and it encodes to the sample's XCDR2 body); the
//! `Wider` evolution readers, the `_at` origin and the encapsulation
//! dispatch are checked too.
//!
//! Skipped, with a message, without a C++ compiler.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::codegen::cpp::CppGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

mod xcdr1_common;
use xcdr1_common::{
    cyclone_xcdr1, plprobe, spec_xcdr1, with_pid_ignore, xcdr1_bodies, PLPROBE_IDL,
};

const CORPUS: &str = include_str!("fixtures/xtypes_datapath/corpus.idl");
const VECTORS: &str = include_str!("fixtures/xtypes_datapath/cyclone_vectors.txt");
/// @mutable unions, which Cyclone DDS 0.11 does not support, built from the
/// specification: the form hddsgen writes (`spec=`, the M flag on the
/// discriminator header) and another form a reader must accept (`alt=`),
/// see the file header.
const SPEC_MUTABLE_UNION: &str = include_str!("fixtures/xtypes_datapath/spec_mutable_union.txt");
/// The types of `spec_mutable_union.txt`.
const UM_IDL: &str =
    "@mutable union UM switch (long) { case 1: long a; case 2: string s; case 3: double d; };
@final struct HoldUM { octet p; UM u; long z; };
";

/// The @mutable @autoid(HASH) union of `spec_mutable_union_hashed.txt` (case
/// ids 0x0FA5DD70 and 0x047790DA, DDS-XTypes 1.3 Example 2).
const SPEC_MUTABLE_UNION_HASHED: &str =
    include_str!("fixtures/xtypes_datapath/spec_mutable_union_hashed.txt");
const UMH_IDL: &str = "@mutable @autoid(HASH) union UMh switch (long) { case 1: long color; \
    case 2: short shapesize; };\n@final struct HoldUMh { octet p; UMh u; };\n";
const UMH_SAMPLES: &[(&str, &[&str])] = &[
    ("HoldUMh_1", &["p = 1", "u._d = 1", "u._u.color = 0x11"]),
    ("HoldUMh_2", &["p = 1", "u._d = 2", "u._u.shapesize = 0x22"]),
];

/// Reader types the corpus lacks, checked against Cyclone bodies of corpus
/// types with the same wire form (`roundtrips`, `extras`).
const EXTRA_IDL: &str = "
@final struct WStrB { octet p; wchar c; wstring<2> s; wchar d; };
@final struct WStrB1 { octet p; wchar c; wstring<1> s; wchar d; };
module lm { enum LE { LA, LB, LC }; };
@final union UB switch (boolean) { case TRUE: long t; case FALSE: short f; };
@final union UC switch (char) { case 'a': long a; case 'b': case 'c': short bc; };
@final union UN switch (long) { case -1: long neg; case 1: case 2: case 3: octet multi; default: short def; };
@final union UQ switch (lm::LE) { case lm::LA: long a; case ::lm::LB: case lm::LC: short b; };
@final struct HoldLabels { UB b; UC c; UN n; UQ q; };
@mutable struct MutWider { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) long added; };
@appendable struct AppWider { long a; long b; long added; };
@final struct NestAppWider { octet p; AppWider w; long z; };
enum EF { EF_A = 5, EF_B = 6 };
@final struct NIn { long x; EF e; };
@mutable struct MutWiderE { @id(1) long a; @id(2) long b; @id(3) string s; @id(5) EF e; @id(6) NIn n; };
@appendable struct AppWiderE { long a; long b; EF e; @optional long o; NIn n; string t; };
";

/// Samples of `spec_mutable_union.txt` (the members of `UM` are fields of
/// the struct: `|` marks a path segment written as is in both flavours).
const UM_SAMPLES: &[(&str, &[&str])] = &[
    ("HoldUM_1", &["p = 1", "u._d = 1", "u.|a = 10", "z = 9"]),
    (
        "HoldUM_2",
        &[
            "p = 1",
            "u._d = 2",
            "u.|s = std::string(\"hello\")",
            "z = 9",
        ],
    ),
    ("HoldUM_3", &["p = 1", "u._d = 3", "u.|d = 0.25", "z = 9"]),
];

/// (`alt=`, `spec=`) XCDR2 bodies of `name`, without the encapsulation
/// header.
fn spec_mutable_union(name: &str) -> (String, String) {
    alt_spec_lines(SPEC_MUTABLE_UNION, name)
}

/// (`alt=`, `spec=`) XCDR2 bodies of `name` in the fixture text `file`.
fn alt_spec_lines(file: &str, name: &str) -> (String, String) {
    file.lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == name && it.next()? == "xcdr2").then_some(())?;
            let alt = it.next()?.strip_prefix("alt=")?;
            let spec = it.next()?.strip_prefix("spec=")?;
            Some((alt[8..].to_string(), spec[8..].to_string()))
        })
        .unwrap_or_else(|| panic!("no xcdr2 line for {name}"))
}

/// (sample name in `cyclone_vectors.txt`, C++ type, member operations).
///
/// An operation is `path = value`: set on the encoded value, then checked
/// with `==` on the decoded one. `!path = value` is set only, `?path =
/// value` checked only. A path is a member chain (`in8.x`, `v[0].y`); the
/// segments from a `_`-prefixed one on (`u._d`, `u._u.a`) are union fields,
/// and a segment ending with `)` is a method call, both written as they are
/// in either flavour.
const SAMPLES: &[(&str, &str, &[&str])] = &[
    (
        "FinalPrims",
        "FinalPrims",
        &[
            "a = 0x11",
            "b = 0x0102030405060708LL",
            "c = -2",
            "d = 1.5",
            "e = true",
            "f = 0xAABBCCDDu",
        ],
    ),
    (
        "NestFinal",
        "NestFinal",
        &[
            "a = 1",
            "in8.x = 0x1122334455667788LL",
            "in8.y = 7",
            "z = 9",
        ],
    ),
    (
        "NestOdd",
        "NestOdd",
        &["a = 1", "il.x = 0x01020304", "s = 5"],
    ),
    (
        "NestApp",
        "NestApp",
        &[
            "a = 1",
            "app.a = 2",
            "app.s = std::string(\"hi\")",
            "app.b = 3",
            "z = 4",
        ],
    ),
    (
        "MutDefault",
        "MutDefault",
        &["a = 1", "s = std::string(\"abc\")", "d = 2.0", "o = 7"],
    ),
    ("MutIds", "MutIds", &["a = 1", "b = 2", "c = 3", "d = 4"]),
    ("MutHash", "MutHash", &["a = 1", "b = 2", "c = 3", "d = 4"]),
    ("MutHashid", "MutHashid", &["x = 1", "y = 2", "z = 3"]),
    ("MutPrims", "MutPrims", &["x = 1", "y = 2", "z = 3"]),
    ("MutCompact2", "MutCompact2", &["a = 1", "b = 2", "c = 3"]),
    (
        "NestMut",
        "NestMut",
        &["p = 1", "m.x = 1", "m.y = 2", "m.z = 3", "q = 2"],
    ),
    (
        "MutOpt_all",
        "MutOpt",
        &[
            "a = std::optional<int32_t>(5)",
            "b = std::optional<std::string>(\"x\")",
            "c = 6",
        ],
    ),
    (
        "MutOpt_none",
        "MutOpt",
        &["a = std::nullopt", "b = std::nullopt", "c = 6"],
    ),
    (
        "DerivedF",
        "DerivedF",
        &["id = 1", "v = 2.5", "s = 3", "w = 4"],
    ),
    (
        "DerivedA",
        "DerivedA",
        &["id = 1", "name = std::string(\"n\")"],
    ),
    (
        "DerivedM",
        "DerivedM",
        &[
            "id = 1",
            "tag = std::string(\"t\")",
            "extra = 2",
            "s = 3",
            "after = 4",
        ],
    ),
    (
        "Enums",
        "Enums",
        &[
            "a = E8::E8_B",
            "b = E16::E16_B",
            "c = E32::E32_C",
            "x = 0xEE",
            "d = E16::E16_A",
            "e = E8::E8_C",
        ],
    ),
    (
        "MutEnums",
        "MutEnums",
        &["a = E8::E8_C", "b = E16::E16_B", "c = E32::E32_B"],
    ),
    ("AppEnums", "AppEnums", &["p = 1", "b = E16::E16_B"]),
    (
        "Seqs",
        "Seqs",
        &[
            "p = 1",
            "u = std::vector<int64_t>{1, 2}",
            "bd = std::vector<double>{0.5}",
            "q = 2",
            "s = std::vector<int16_t>{3, 4, 5}",
            "bl = std::vector<int64_t>{6}",
        ],
    ),
    (
        "SeqStruct",
        "SeqStruct",
        &[
            "p = 1",
            "!v = std::vector<Inner8>(2)",
            "?v.size() = 2u",
            "v[0].x = 1",
            "v[0].y = 2",
            "v[1].x = 3",
            "v[1].y = 4",
        ],
    ),
    (
        "SeqApp",
        "SeqApp",
        &[
            "p = 1",
            "!v = std::vector<AppStruct>(2)",
            "?v.size() = 2u",
            "v[0].a = 1",
            "v[0].s = std::string(\"a\")",
            "v[0].b = 2",
            "v[1].a = 3",
            "v[1].s = std::string(\"\")",
            "v[1].b = 4",
        ],
    ),
    (
        "SeqStr",
        "SeqStr",
        &[
            "p = 1",
            "v = std::vector<std::string>{\"ab\", \"c\"}",
            "b = std::vector<std::array<char, 4>>{A<4>(\"wxyz\")}",
        ],
    ),
    (
        "Arrs",
        "Arrs",
        &[
            "p = 1",
            "a = std::array<int64_t, 3>{1, 2, 3}",
            "m = std::array<std::array<uint8_t, 3>, 2>{{{1, 2, 3}, {4, 5, 6}}}",
            "s = std::array<std::array<int16_t, 2>, 2>{{{1, 2}, {3, 4}}}",
        ],
    ),
    (
        "ArrStruct",
        "ArrStruct",
        &[
            "p = 1",
            "a[0].x = 1",
            "a[0].y = 2",
            "a[1].x = 3",
            "a[1].y = 4",
        ],
    ),
    (
        "HoldU_1",
        "HoldU",
        &["p = 1", "u._d = 1", "u._u.a = 0x0A", "z = 9"],
    ),
    (
        "HoldU_2",
        "HoldU",
        &["p = 1", "u._d = 2", "u._u.s = A<8>(\"hello\")", "z = 9"],
    ),
    (
        "HoldU_3",
        "HoldU",
        &["p = 1", "u._d = 3", "u._u.d = 0.25", "z = 9"],
    ),
    (
        "HoldU_def",
        "HoldU",
        &["p = 1", "u._d = 9", "u._u.o = 0x7F", "z = 9"],
    ),
    (
        "HoldU_def0",
        "HoldU",
        &["p = 1", "u._d = 0", "u._u.o = 0x7F", "z = 9"],
    ),
    ("HoldUA_1", "HoldUA", &["p = 1", "u._d = 1", "u._u.a = 5"]),
    (
        "HoldUA_2",
        "HoldUA",
        &["p = 1", "u._d = 2", "u._u.s = A<8>(\"ab\")"],
    ),
    (
        "HoldUE_A",
        "HoldUE",
        &["p = 1", "u._d = E8::E8_A", "u._u.a = 6", "z = 2"],
    ),
    (
        "HoldUE_B",
        "HoldUE",
        &["p = 1", "u._d = E8::E8_B", "u._u.b = 7", "z = 2"],
    ),
    (
        "HoldUS_1",
        "HoldUS",
        &["p = 1", "u._d = 1", "u._u.a = 0x11", "q = 2"],
    ),
    (
        "HoldUS_2",
        "HoldUS",
        &["p = 1", "u._d = 2", "u._u.b = 0x2233", "q = 2"],
    ),
    (
        "HoldUS_3",
        "HoldUS",
        &["p = 1", "u._d = 3", "u._u.c = 0x44556677", "q = 2"],
    ),
    (
        "BStr",
        "BStr",
        &["p = 1", "s = A<8>(\"abcdefgh\")", "z = 2"],
    ),
    (
        "Keyed",
        "Keyed",
        &["id = 1", "name = A<8>(\"nm\")", "v = 3"],
    ),
    ("KeyedApp", "KeyedApp", &["id = 1", "v = 2"]),
    (
        "FinalOpt_all",
        "FinalOpt",
        &[
            "t = 1",
            "a = std::optional<int32_t>(5)",
            "b = std::optional<int64_t>(6)",
            "u = 2",
        ],
    ),
    (
        "FinalOpt_none",
        "FinalOpt",
        &["t = 1", "a = std::nullopt", "b = std::nullopt", "u = 2"],
    ),
    (
        "AppOpt_all",
        "AppOpt",
        &["t = 1", "a = std::optional<int32_t>(7)"],
    ),
    ("AppOpt_none", "AppOpt", &["t = 1", "a = std::nullopt"]),
    (
        "MutMix",
        "MutMix",
        &[
            "sl = std::vector<int32_t>{1, 2}",
            "ss = std::vector<std::string>{\"a\"}",
            "fin.x = 3",
            "fin.y = 4",
            "app.a = 5",
            "app.s = std::string(\"b\")",
            "app.b = 6",
            "mut.x = 7",
            "mut.y = 8",
            "mut.z = 9",
            "un._d = 2",
            "un._u.s = A<8>(\"u\")",
            "arr = std::array<int64_t, 2>{10, 11}",
            "bs = A<4>(\"xy\")",
            "en = E16::E16_B",
            "se = std::vector<E8>{E8::E8_B, E8::E8_C}",
        ],
    ),
    (
        "SeqEnum",
        "SeqEnum",
        &[
            "p = 1",
            "a = std::vector<E8>{E8::E8_C, E8::E8_A, E8::E8_B}",
            "b = std::vector<E32>{E32::E32_B}",
            "arr = std::array<E8, 3>{E8::E8_B, E8::E8_C, E8::E8_A}",
        ],
    ),
    (
        "WStr",
        "WStr",
        &[
            "p = 1",
            "c = L'A'",
            "s = std::wstring(L\"h\\x00e9\")",
            "d = static_cast<wchar_t>(0x263A)",
        ],
    ),
    (
        "Aliases",
        "Aliases",
        &[
            "p = 1",
            "m = BM32::BM32_0 | BM32::BM32_5",
            "n = BM8::BM8_1",
            "l = LSeq{7, 8}",
            "ia.x = 9",
            "ia.y = 10",
        ],
    ),
    (
        "ArrStr",
        "ArrStr",
        &[
            "p = 1",
            "a = std::array<std::string, 2>{\"ab\", \"\"}",
            "b = std::array<std::array<char, 3>, 2>{A<3>(\"xyz\"), A<3>(\"q\")}",
        ],
    ),
    (
        "MutLC",
        "MutLC",
        &[
            "s1 = std::vector<uint8_t>{1, 2, 3}",
            "s2 = std::vector<int16_t>{4}",
            "s4 = std::vector<int32_t>{5, 6}",
            "s8 = std::vector<int64_t>{7}",
            "sd = std::vector<double>{0.5, 1.0}",
            "sb = std::vector<bool>{true}",
            "a1 = std::array<uint8_t, 3>{1, 2, 3}",
            "a2 = std::array<int16_t, 2>{4, 5}",
            "a4 = std::array<int32_t, 2>{6, 7}",
            "s = std::string(\"st\")",
            "!si = std::vector<Inner8>(1)",
            "?si.size() = 1u",
            "si[0].x = 1",
            "si[0].y = 2",
            "e = E32::E32_C",
            "m = BM32::BM32_1",
            "o = std::nullopt",
            "a8 = std::array<int64_t, 1>{8}",
            "b = true",
            "s0 = std::vector<uint8_t>{}",
        ],
    ),
    (
        "MutLC2",
        "MutLC2",
        &[
            "ai[0].x = 1",
            "ai[0].y = 2",
            "ae = std::array<E8, 2>{E8::E8_B, E8::E8_C}",
            "sa = std::array<std::string, 1>{\"s\"}",
            "bm = BM8::BM8_0",
            "sbm = std::vector<BM8>{BM8::BM8_1}",
            "ls = LSeq{3}",
            "ia.x = 4",
            "ia.y = 5",
        ],
    ),
    // sequence<char> / sequence<wchar> are sequences (element count, no
    // NUL; rule 11), string<4> a string (rules 3-4).
    (
        "SeqChar",
        "SeqChar",
        &[
            "p = 1",
            "c = std::vector<char>{'x', 'y'}",
            "w = std::vector<wchar_t>{L'A', L'B'}",
            "s = A<4>(\"ab\")",
        ],
    ),
    (
        "MutSeqChar",
        "MutSeqChar",
        &[
            "c = std::vector<char>{'q'}",
            "w = std::vector<wchar_t>{L'Z'}",
            "z = 5",
        ],
    ),
];

const PRELUDE: &str = r#"#include "corpus.hpp"
#include "um.hpp"
#include "extra.hpp"
#include "umh.hpp"
#include <cstdio>
#include <string>
#include <vector>
template <std::size_t N> std::array<char, N> A(const char* s) { std::array<char, N> a{}; std::strncpy(a.data(), s, N); return a; }
template <std::size_t N> std::array<wchar_t, N> W(const wchar_t* s) { std::array<wchar_t, N> a{}; for (std::size_t i = 0; i < N && s[i] != 0; ++i) a[i] = s[i]; return a; }
static std::string hex(const std::uint8_t* b, std::size_t n) {
    std::string s; char t[3];
    for (std::size_t i = 0; i < n; ++i) { std::snprintf(t, sizeof t, "%02x", b[i]); s += t; }
    return s;
}
static std::vector<std::uint8_t> unhex(const char* h) {
    std::vector<std::uint8_t> v;
    for (std::size_t i = 0; h[i] && h[i + 1]; i += 2) { unsigned x = 0; std::sscanf(h + i, "%2x", &x); v.push_back(static_cast<std::uint8_t>(x)); }
    return v;
}
template <class T, class F> static void run(const char* name, const T& v, T& d, const char* exp, F check) {
    std::vector<std::uint8_t> buf(4096, 0xAA);
    int n = v.encode_cdr2_le(buf.data(), buf.size());
    std::string got = n >= 0 ? hex(buf.data(), static_cast<std::size_t>(n)) : std::string("error");
    if (got == exp) std::printf("ENC_OK %s\n", name);
    else std::printf("ENC_FAIL %s\n  got %s\n  exp %s\n", name, got.c_str(), exp);
    std::vector<std::uint8_t> bytes = unhex(exp);
    std::vector<std::uint8_t> shrt(bytes.size() - 1);
    if (v.encode_cdr2_le(shrt.data(), shrt.size()) < 0) std::printf("SHORT_OK %s\n", name);
    else std::printf("SHORT_FAIL %s\n", name);
    int r = d.decode_cdr2_le(bytes.data(), bytes.size());
    if (r == static_cast<int>(bytes.size()) && check()) std::printf("DEC_OK %s\n", name);
    else std::printf("DEC_FAIL %s: read %d of %zu, value %s\n", name, r, bytes.size(), check() ? "equal" : "differs");
    // The decoded value re-encodes to the same bytes (a union keeps the
    // discriminator it read).
    std::vector<std::uint8_t> again(4096, 0xAA);
    int m = d.encode_cdr2_le(again.data(), again.size());
    std::string re = m >= 0 ? hex(again.data(), static_cast<std::size_t>(m)) : std::string("error");
    if (re == exp) std::printf("REENC_OK %s\n", name);
    else std::printf("REENC_FAIL %s\n  got %s\n  exp %s\n", name, re.c_str(), exp);
}
template <class T, class F> static void xdec(const char* name, T& d, const char* h, bool ok, F check) {
    std::vector<std::uint8_t> bytes = unhex(h);
    int r = d.decode_cdr2_le(bytes.data(), bytes.size());
    bool good = ok ? (r == static_cast<int>(bytes.size()) && check()) : r < 0;
    if (good) std::printf("XTRA_OK %s\n", name);
    else std::printf("XTRA_FAIL %s: read %d of %zu\n", name, r, bytes.size());
}
template <class T, class F> static void x1dec(const char* name, T& d, const char* h, const char* exp2, F check) {
    std::vector<std::uint8_t> bytes = unhex(h);
    int r = d.decode_cdr1_le(bytes.data(), bytes.size());
    std::vector<std::uint8_t> again(4096, 0xAA);
    int m = r >= 0 ? d.encode_cdr2_le(again.data(), again.size()) : -1;
    std::string re = m >= 0 ? hex(again.data(), static_cast<std::size_t>(m)) : std::string("error");
    if (r == static_cast<int>(bytes.size()) && check() && (exp2[0] == 0 || re == exp2)) std::printf("X1_OK %s\n", name);
    else std::printf("X1_FAIL %s: read %d of %zu, value %s\n  got %s\n", name, r, bytes.size(), check() ? "equal" : "differs", re.c_str());
}
template <class T> static void x1ref(const char* name, T& d, const char* h) {
    std::vector<std::uint8_t> bytes = unhex(h);
    int r = d.decode_cdr1_le(bytes.data(), bytes.size());
    if (r < 0) std::printf("X1_OK %s\n", name);
    else std::printf("X1_FAIL %s: accepted\n", name);
}
template <class T> static void x1enc(const char* name, T& d, std::uint16_t id, const char* h, bool ok) {
    std::vector<std::uint8_t> bytes = unhex(h);
    int r = d.decode_by_encapsulation(id, bytes.data(), bytes.size());
    if (ok ? r == static_cast<int>(bytes.size()) : r < 0) std::printf("X1ENC_OK %s\n", name);
    else std::printf("X1ENC_FAIL %s %d\n", name, r);
}
template <class T> static void xenc_refused(const char* name, const T& v) {
    std::vector<std::uint8_t> buf(4096, 0);
    if (v.encode_cdr2_le(buf.data(), buf.size()) < 0) std::printf("XTRA_OK %s\n", name);
    else std::printf("XTRA_FAIL %s: encoded\n", name);
}
int main() {
"#;

/// The C++ expression of `path` on `obj`; with `set`, the statement setting
/// it.
fn render(obj: &str, path: &str, compat: bool, set: Option<&str>) -> String {
    let segs: Vec<&str> = path.split('.').collect();
    let mut out = obj.to_string();
    let mut plain = !compat;
    for (i, seg) in segs.iter().enumerate() {
        let last = i + 1 == segs.len();
        if seg.starts_with('_') || seg.ends_with(')') {
            plain = true;
        }
        let seg = &seg.strip_prefix('|').map_or(*seg, |rest| {
            plain = true;
            rest
        });
        if plain {
            let _ = write!(out, ".{seg}");
            continue;
        }
        let (name, index) = seg.find('[').map_or((*seg, ""), |at| seg.split_at(at));
        match set {
            Some(value) if last && index.is_empty() => {
                return format!("{out}.{name}({value});");
            }
            _ => {
                let _ = write!(out, ".{name}(){index}");
            }
        }
    }
    match set {
        Some(value) => format!("{out} = {value};"),
        None => out,
    }
}

/// Cyclone's XCDR2 body of `name`.
fn cyclone(name: &str) -> String {
    VECTORS
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == name && it.next()? == "xcdr2").then_some(())?;
            let _hdr = it.next()?;
            it.next()?.strip_prefix("body=").map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no Cyclone xcdr2 vector for {name}"))
}

/// The statements setting the operations of `ops` on `set_obj` and the
/// expression checking them on `check_obj` (`true` when there is none).
fn ops_code(ops: &[&str], set_obj: &str, check_obj: &str, compat: bool) -> (String, String) {
    let mut sets = String::new();
    let mut checks = Vec::new();
    for op in ops {
        let (path, value) = op.split_once(" = ").expect("op");
        let (path, set, check) = path.strip_prefix('!').map_or_else(
            || {
                path.strip_prefix('?')
                    .map_or((path, true, true), |p| (p, false, true))
            },
            |p| (p, true, false),
        );
        if set {
            let _ = write!(sets, " {}", render(set_obj, path, compat, Some(value)));
        }
        if check {
            checks.push(format!(
                "{} == ({value})",
                render(check_obj, path, compat, None)
            ));
        }
    }
    if checks.is_empty() {
        checks.push("true".to_string());
    }
    (sets, checks.join(" && "))
}

/// Samples of the reader types of `EXTRA_IDL` (or of other headers): (name,
/// type, operations, the XCDR2 body of a Cyclone sample with the same wire
/// form), encoded and decoded as `SAMPLES`.
fn roundtrips() -> Vec<(&'static str, &'static str, &'static [&'static str], String)> {
    vec![
        // wstring<2> holding "h\u00e9" is WStr's wstring on the wire.
        (
            "WStrB",
            "WStrB",
            &[
                "p = 1",
                "c = L'A'",
                "s = W<2>(L\"h\\x00e9\")",
                "d = static_cast<wchar_t>(0x263A)",
            ],
            cyclone("WStr"),
        ),
        // Union labels TRUE / FALSE, characters, negative integers, scoped
        // enumerators, several labels on a case and `default` (rules 26
        // and 29: the discriminator at its width, then the selected
        // member; bytes from those rules). The discriminator decoded is
        // the one re-encoded (2 of `case 1: case 2: case 3:`, 'c' of
        // `case 'b': case 'c':`, lm::LC of `case ::lm::LB: case lm::LC:`).
        (
            "labels_multi",
            "HoldLabels",
            &[
                "b._d = false",
                "b._u.f = 0x0102",
                "c._d = 'c'",
                "c._u.bc = 0x0304",
                "n._d = 2",
                "n._u.multi = 5",
                "q._d = lm::LE::LC",
                "q._u.b = 0x0607",
            ],
            "00000201630004030200000005000000020000000706".to_string(),
        ),
        (
            "labels_first",
            "HoldLabels",
            &[
                "b._d = true",
                "b._u.t = 0x11223344",
                "c._d = 'a'",
                "c._u.a = 0x05060708",
                "n._d = -1",
                "n._u.neg = 0x0a",
                "q._d = lm::LE::LA",
                "q._u.a = 0x0b",
            ],
            "0100000044332211610000000807060\
             5ffffffff0a000000000000000b000000"
                .to_string(),
        ),
        // `default` selected by 9, which no label names: 9 is kept.
        (
            "labels_default",
            "HoldLabels",
            &[
                "b._d = false",
                "b._u.f = 0",
                "c._d = 'b'",
                "c._u.bc = 0",
                "n._d = 9",
                "n._u.def = 0x0c0d",
                "q._d = lm::LE::LB",
                "q._u.b = 0",
            ],
            "000000006200000009000000\
             0d0c0000010000000000"
                .to_string(),
        ),
    ]
}

/// A decode the specification decides without an encoder: (name, type,
/// operations set on the object before decoding, body, whether it must
/// decode, operations checked after).
type Extra = (
    &'static str,
    &'static str,
    &'static [&'static str],
    String,
    bool,
    &'static [&'static str],
);

/// Rules the Cyclone samples do not reach, each with bytes derived from a
/// Cyclone body by hand.
fn extras() -> Vec<Extra> {
    let seqs = cyclone("Seqs");
    let final_prims = cyclone("FinalPrims");
    let final_opt = cyclone("FinalOpt_all");
    let enums = cyclone("Enums");
    let (_, um1) = spec_mutable_union("HoldUM_1");
    vec![
        // An enumeration value that is none of its literals: Enums.c (E32,
        // bytes 4-7) = 7...
        (
            "enum_not_a_literal_rejected",
            "Enums",
            &[],
            format!("{}07000000{}", &enums[..8], &enums[16..]),
            false,
            &[],
        ),
        // ...and the E8 discriminator of HoldUE (byte 1) = 5.
        (
            "enum_discriminator_not_a_literal_rejected",
            "HoldUE",
            &[],
            "0105070002000000".to_string(),
            false,
            &[],
        ),
        // A MUTABLE member absent from the data takes its default value
        // (MutPrims with x only; y and z held 6 and 7).
        (
            "mutable_absent_member_default",
            "MutPrims",
            &["x = 5", "y = 6", "z = 7"],
            "080000000000002001000000".to_string(),
            true,
            &["x = 1", "y = 0", "z = 0"],
        ),
        // Rule 27: the case member must be the one the discriminator
        // selects (HoldUM_1, spec form, discriminator (byte 12) 3 for member 1)...
        (
            "mutable_union_case_mismatch_rejected",
            "HoldUM",
            &[],
            format!("{}03{}", &um1[..24], &um1[26..]),
            false,
            &[],
        ),
        // ...the discriminator (member 0) is required (an empty union:
        // without the check, `_d` 0 selects no case and nothing is missing)...
        (
            "mutable_union_without_discriminator_rejected",
            "HoldUM",
            &[],
            "010000000000000009000000".to_string(),
            false,
            &[],
        ),
        // ...a member unknown to the reader is skipped (HoldUM_1, spec
        // form, with a member 7 appended inside the union's DHEADER)...
        (
            "mutable_union_unknown_member_skipped",
            "HoldUM",
            &[],
            format!(
                "{}18{}0700002005000000{}",
                &um1[..8],
                &um1[10..48],
                &um1[48..]
            ),
            true,
            &["p = 1", "u._d = 1", "u.|a = 10", "z = 9"],
        ),
        // ...unless it carries the M flag.
        (
            "mutable_union_unknown_must_understand_rejected",
            "HoldUM",
            &[],
            format!(
                "{}18{}070000a005000000{}",
                &um1[..8],
                &um1[10..48],
                &um1[48..]
            ),
            false,
            &[],
        ),
        // 7.4.3.4.2: a member the reader does not know is skipped (the
        // MutPrims ids 0-2 are unknown to MutIds, ids 10, 11, 5, 6)...
        (
            "unknown_member_skipped",
            "MutIds",
            &[],
            cyclone("MutPrims"),
            true,
            &["a = 0", "b = 0", "c = 0", "d = 0"],
        ),
        // ...unless its EMHEADER1 carries the M flag (Keyed id 0 is a key).
        (
            "unknown_must_understand_rejected",
            "MutIds",
            &[],
            cyclone("Keyed"),
            false,
            &[],
        ),
        // string<8> with 9 characters.
        (
            "string_bound_rejected",
            "BStr",
            &[],
            "010000000a00000061626364656667686900000002000000".to_string(),
            false,
            &[],
        ),
        // wstring<1> with 2 characters (WStr's body).
        (
            "wstring_bound_rejected",
            "WStrB1",
            &[],
            cyclone("WStr"),
            false,
            &[],
        ),
        // Type evolution (7.4.1.2.3, spec text lines 6195-6203; Table 9 /
        // lines 3390-3393): a reader whose type has more members reads an
        // older writer's data, the missing members at their default, never
        // what the object held. MutW (Cyclone) read as MutWider...
        (
            "mutable_evolution_default",
            "MutWider",
            &["a = 9", "added = 77"],
            cyclone("MutW"),
            true,
            &["a = 1", "b = 2", "s = std::string(\"w\")", "added = 0"],
        ),
        // ...AppW as AppWider (the members after the DHEADER end)...
        (
            "appendable_evolution_default",
            "AppWider",
            &["added = 77"],
            cyclone("AppW"),
            true,
            &["a = 1", "b = 2", "added = 0"],
        ),
        // ...NestAppW as NestAppWider: the nested reader stops at its
        // DHEADER end, z is read after it...
        (
            "nested_appendable_evolution_default",
            "NestAppWider",
            &["w.added = 77", "z = 3"],
            cyclone("NestAppW"),
            true,
            &["p = 1", "w.a = 3", "w.b = 4", "w.added = 0", "z = 5"],
        ),
        // ...an enumeration takes its first literal (EF_A = 5, not 0), an
        // optional member no value, a nested structure its own defaults.
        (
            "mutable_evolution_enum_and_nested_default",
            "MutWiderE",
            &["e = EF::EF_B", "n.x = 7", "n.e = EF::EF_B"],
            cyclone("MutW"),
            true,
            &["a = 1", "e = EF::EF_A", "n.x = 0", "n.e = EF::EF_A"],
        ),
        (
            "appendable_evolution_enum_optional_nested_default",
            "AppWiderE",
            &[
                "e = EF::EF_B",
                "o = std::optional<int32_t>(4)",
                "n.x = 7",
                "n.e = EF::EF_B",
                "t = std::string(\"old\")",
            ],
            cyclone("AppW"),
            true,
            &[
                "a = 1",
                "b = 2",
                "e = EF::EF_A",
                "o = std::nullopt",
                "n.x = 0",
                "n.e = EF::EF_A",
                "t = std::string()",
            ],
        ),
        // Seqs.bl (sequence<long long, 3>, the last member) with 4 elements.
        (
            "sequence_bound_rejected",
            "Seqs",
            &[],
            format!(
                "{}04000000{}",
                &seqs[..seqs.len() - 24],
                "0600000000000000".repeat(4)
            ),
            false,
            &[],
        ),
        // FinalPrims.e (boolean, byte 24) = 2.
        (
            "boolean_2_rejected",
            "FinalPrims",
            &[],
            format!("{}02{}", &final_prims[..48], &final_prims[50..]),
            false,
            &[],
        ),
        // FinalOpt.a presence flag (byte 1) = 2 (rule 20: a boolean).
        (
            "optional_flag_2_rejected",
            "FinalOpt",
            &[],
            format!("{}02{}", &final_opt[..2], &final_opt[4..]),
            false,
            &[],
        ),
        // A MUTABLE optional member without EMHEADER1 is absent.
        (
            "mutable_absent_optional_resets",
            "MutOpt",
            &[
                "a = std::optional<int32_t>(5)",
                "b = std::optional<std::string>(\"x\")",
            ],
            cyclone("MutOpt_none"),
            true,
            &["a = std::nullopt", "b = std::nullopt", "c = 6"],
        ),
        // Rule 30: an APPENDABLE reader skips what a newer writer appended
        // inside the DHEADER (KeyedApp with 4 more bytes).
        (
            "appendable_trailing_skipped",
            "KeyedApp",
            &[],
            "0c000000010000000200000063000000".to_string(),
            true,
            &["id = 1", "v = 2"],
        ),
    ]
}

/// Encodes the specification refuses: (name, type, operations).
const REFUSED: &[(&str, &str, &[&str])] = &[
    // Seqs.bl is sequence<long long, 3>.
    (
        "sequence_bound_refused",
        "Seqs",
        &["bl = std::vector<int64_t>{6, 7, 8, 9}"],
    ),
    // wchar is one UTF-16 code unit.
    (
        "wchar_outside_bmp_refused",
        "WStr",
        &["c = static_cast<wchar_t>(0x1F600)"],
    ),
];

fn program(compat: bool) -> String {
    let mut body = String::from(PRELUDE);
    for (name, ty, ops) in SAMPLES {
        let hex = cyclone(name);
        let (sets, checks) = ops_code(ops, "v", "d", compat);
        let _ = writeln!(
            body,
            "    {{ {ty} v{{}};{sets} {ty} d{{}}; run(\"{name}\", v, d, \"{hex}\", [&] {{ return {checks}; }}); }}"
        );
    }
    for (name, ty, ops, hex) in roundtrips() {
        let (sets, checks) = ops_code(ops, "v", "d", compat);
        let _ = writeln!(
            body,
            "    {{ {ty} v{{}};{sets} {ty} d{{}}; run(\"{name}\", v, d, \"{hex}\", [&] {{ return {checks}; }}); }}"
        );
    }
    for (name, ops) in UM_SAMPLES {
        // Encode to the specification's form; read it and the `alt=` form.
        let (alt, spec) = spec_mutable_union(name);
        let (sets, checks) = ops_code(ops, "v", "d", compat);
        let _ = writeln!(
            body,
            "    {{ HoldUM v{{}};{sets} HoldUM d{{}}; run(\"{name}\", v, d, \"{spec}\", [&] {{ return {checks}; }}); }}"
        );
        let (_, checks) = ops_code(ops, "d", "d", compat);
        let _ = writeln!(
            body,
            "    {{ HoldUM d{{}}; xdec(\"{name}/alt\", d, \"{alt}\", true, [&] {{ return {checks}; }}); }}"
        );
    }
    for (name, ops) in UMH_SAMPLES {
        // Encode to the specification's form; read it and the `alt=` form.
        let (alt, spec) = alt_spec_lines(SPEC_MUTABLE_UNION_HASHED, name);
        let (sets, checks) = ops_code(ops, "v", "d", compat);
        let _ = writeln!(
            body,
            "    {{ HoldUMh v{{}};{sets} HoldUMh d{{}}; run(\"{name}\", v, d, \"{spec}\", [&] {{ return {checks}; }}); }}"
        );
        let (_, checks) = ops_code(ops, "d", "d", compat);
        let _ = writeln!(
            body,
            "    {{ HoldUMh d{{}}; xdec(\"{name}/alt\", d, \"{alt}\", true, [&] {{ return {checks}; }}); }}"
        );
    }
    for (name, ty, pre, hex, ok, post) in extras() {
        let (pre_sets, _) = ops_code(pre, "d", "d", compat);
        let (_, checks) = ops_code(post, "d", "d", compat);
        let _ = writeln!(
            body,
            "    {{ {ty} d{{}};{pre_sets} xdec(\"{name}\", d, \"{hex}\", {ok}, [&] {{ return {checks}; }}); }}"
        );
    }
    for (name, ty, ops) in REFUSED {
        let (sets, _) = ops_code(ops, "v", "v", compat);
        let _ = writeln!(
            body,
            "    {{ {ty} v{{}};{sets} xenc_refused(\"{name}\", v); }}"
        );
    }
    xcdr1_program(&mut body, compat);
    body.push_str("    return 0;\n}\n");
    body
}

/// The number of XCDR1 checks `xcdr1_program` emits.
fn xcdr1_checks() -> usize {
    let samples: usize = SAMPLES
        .iter()
        .map(|(n, _, _)| xcdr1_bodies(n).len())
        .chain(UM_SAMPLES.iter().map(|(n, _)| xcdr1_bodies(n).len()))
        .chain(UMH_SAMPLES.iter().map(|(n, _)| xcdr1_bodies(n).len()))
        .sum();
    samples + xcdr1_evolution().len() + 1 + plprobe().iter().filter(|p| p.xcdr1).count()
}

/// The checks of the `spec_plprobe.txt` sample.
const PLPROBE_OPS: &[&str] = &[
    "id = 0x11223344",
    "s = -2",
    "name = std::string(\"hdds\")",
    "big = 0x0102030405060708LL",
    "tag = 0xab",
];

/// XCDR1 type evolution: (label, body, reader type, checks on `d`).
fn xcdr1_evolution() -> Vec<(String, String, &'static str, &'static [&'static str])> {
    const WIDER: &[&str] = &["a = 1", "b = 2", "s = std::string(\"w\")", "added = 0"];
    const APP: &[&str] = &["a = 1", "b = 2", "added = 0"];
    vec![
        (
            "MutW/cyclone".into(),
            cyclone_xcdr1("MutW"),
            "MutWider",
            WIDER,
        ),
        ("MutW/spec".into(), spec_xcdr1("MutW"), "MutWider", WIDER),
        (
            "AppW/cyclone".into(),
            cyclone_xcdr1("AppW"),
            "AppWider",
            APP,
        ),
    ]
}

/// The XCDR1 checks of the program, appended to `body`.
fn xcdr1_program(body: &mut String, compat: bool) {
    let all = SAMPLES
        .iter()
        .map(|(n, t, o)| (*n, *t, *o, cyclone(n)))
        .chain(
            UM_SAMPLES
                .iter()
                .map(|(n, o)| (*n, "HoldUM", *o, spec_mutable_union(n).1)),
        )
        .chain(UMH_SAMPLES.iter().map(|(n, o)| {
            (
                *n,
                "HoldUMh",
                *o,
                alt_spec_lines(SPEC_MUTABLE_UNION_HASHED, n).1,
            )
        }));
    for (name, ty, ops, exp2) in all {
        let (_, checks) = ops_code(ops, "d", "d", compat);
        for (label, hex) in xcdr1_bodies(name) {
            let _ = writeln!(
                body,
                "    {{ {ty} d{{}}; x1dec(\"{label}\", d, \"{hex}\", \"{exp2}\", [&] {{ return {checks}; }}); }}"
            );
        }
    }
    for (label, hex, ty, ops) in xcdr1_evolution() {
        let (_, checks) = ops_code(ops, "d", "d", compat);
        let _ = writeln!(
            body,
            "    {{ {ty} d{{}}; x1dec(\"{label} as {ty}\", d, \"{hex}\", \"\", [&] {{ return {checks}; }}); }}"
        );
    }
    let (_, checks) = ops_code(&["x = 1", "y = 2", "z = 3"], "d", "d", compat);
    let _ = writeln!(
        body,
        "    {{ MutPrims d{{}}; x1dec(\"MutPrims/pid_ignore\", d, \"{}\", \"\", [&] {{ return {checks}; }}); }}",
        with_pid_ignore(&cyclone_xcdr1("MutPrims"))
    );
    let (_, checks) = ops_code(PLPROBE_OPS, "d", "d", compat);
    for p in plprobe().into_iter().filter(|p| p.xcdr1) {
        let (label, hex) = (&p.label, &p.body);
        if p.accept {
            let _ = writeln!(
                body,
                "    {{ PlProbe d{{}}; x1dec(\"plprobe {label}\", d, \"{hex}\", \"\", [&] {{ return {checks}; }}); }}"
            );
        } else {
            let _ = writeln!(
                body,
                "    {{ PlProbe d{{}}; x1ref(\"plprobe {label}\", d, \"{hex}\"); }}"
            );
        }
    }
    // `_at`: alignment from the start of the buffer, 8-byte members at 8:
    // Inner8 read at offset 1 of NestFinal's XCDR1 body.
    let nest = cyclone_xcdr1("NestFinal");
    let (x, y) = if compat {
        ("d.x()", "d.y()")
    } else {
        ("d.x", "d.y")
    };
    let _ = writeln!(
        body,
        "    {{ Inner8 d{{}}; std::vector<std::uint8_t> b = unhex(\"{nest}\"); std::size_t off = 1;\n\
         \x20     int r = d.decode_cdr1_le_at(b.data(), b.size(), off);\n\
         \x20     if (r == 0 && off == 20 && {x} == 0x1122334455667788LL && {y} == 7) std::printf(\"X1AT_OK\\n\"); else std::printf(\"X1AT_FAIL %d %zu\\n\", r, off); }}"
    );
    for (label, ty, hex, id, ok) in [
        (
            "cdr_le",
            "NestFinal",
            cyclone_xcdr1("NestFinal"),
            "0x0001u",
            true,
        ),
        (
            "pl_cdr_le",
            "MutPrims",
            cyclone_xcdr1("MutPrims"),
            "0x0003u",
            true,
        ),
        (
            "cdr2_le",
            "NestFinal",
            cyclone("NestFinal"),
            "0x0007u",
            true,
        ),
        (
            "pl_cdr2_le",
            "MutPrims",
            cyclone("MutPrims"),
            "0x000bu",
            true,
        ),
        (
            "cdr_be",
            "NestFinal",
            cyclone_xcdr1("NestFinal"),
            "0x0000u",
            false,
        ),
        (
            "pl_cdr2_be",
            "MutPrims",
            cyclone("MutPrims"),
            "0x000au",
            false,
        ),
    ] {
        let _ = writeln!(
            body,
            "    {{ {ty} d{{}}; x1enc(\"{label}\", d, {id}, \"{hex}\", {ok}); }}"
        );
    }
}

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

fn compile_and_run(cxx: &str, dir: &Path, compat: bool) -> String {
    let flavour = if compat { "compat" } else { "plain" };
    let dir = dir.join(flavour);
    fs::create_dir_all(&dir).expect("dir");
    let generator = if compat {
        CppGenerator::with_fastdds_compat()
    } else {
        CppGenerator::new()
    };
    fs::write(dir.join("corpus.hpp"), generate(&generator, CORPUS)).expect("header");
    fs::write(dir.join("um.hpp"), generate(&generator, UM_IDL)).expect("header");
    fs::write(
        dir.join("extra.hpp"),
        generate(&generator, &format!("{EXTRA_IDL}{PLPROBE_IDL}")),
    )
    .expect("header");
    fs::write(dir.join("umh.hpp"), generate(&generator, UMH_IDL)).expect("header");
    let src = dir.join("main.cpp");
    fs::write(&src, program(compat)).expect("program");
    let exe = dir.join("main");
    let out = Command::new(cxx)
        .args(["-std=c++17", "-Wall", "-Wextra", "-Werror", "-O1", "-o"])
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("c++");
    assert!(
        out.status.success(),
        "{flavour}: compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "{flavour}:\n{stdout}");
    stdout
}

#[test]
fn cpp_backend_matches_cyclone_on_the_corpus() {
    let Some(cxx) = find_cxx() else {
        eprintln!("no C++ compiler found; C++ data path test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_cpp");
    for compat in [false, true] {
        let out = compile_and_run(cxx, &dir, compat);
        let count = |p: &str| out.lines().filter(|l| l.starts_with(p)).count();
        let fails = out.lines().filter(|l| l.contains("_FAIL")).count();
        let samples = SAMPLES.len() + UM_SAMPLES.len() + UMH_SAMPLES.len() + roundtrips().len();
        assert!(
            fails == 0
                && count("ENC_OK ") == samples
                && count("SHORT_OK ") == samples
                && count("DEC_OK ") == samples
                && count("REENC_OK ") == samples
                && count("XTRA_OK ")
                    == extras().len() + REFUSED.len() + UM_SAMPLES.len() + UMH_SAMPLES.len()
                && count("X1_OK ") == xcdr1_checks()
                && count("X1AT_OK") == 1
                && count("X1ENC_OK ") == 6,
            "compat={compat}\n{out}"
        );
    }
}
