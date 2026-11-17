// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated `no_std` (micro) code against a foreign
//! oracle.
//!
//! The micro backend writes XCDR1 (`hdds_micro`'s cursors align 8-byte
//! values at 8): every corpus sample its types can express encodes to the
//! XCDR1 bytes Cyclone DDS 0.11 serializes (`fixtures/xtypes_datapath/
//! cyclone_vectors.txt`) and Cyclone's bytes decode to the sample. The
//! constructs the backend refuses are refused with a reason.
//!
//! Needs the `hdds-micro` crate of an HDDS checkout: run with
//! `HDDSGEN_HDDS_DIR=<hdds> cargo test --test xtypes_datapath_micro --
//! --ignored`. Ignored by default, failing when requested without one.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, Parser};

const CORPUS: &str = include_str!("fixtures/xtypes_datapath/corpus.idl");
const VECTORS: &str = include_str!("fixtures/xtypes_datapath/cyclone_vectors.txt");
/// XCDR1 parameter lists in the specification's form (exact member
/// lengths, `FLAG_2` on key members), derived from Cyclone's by Cyclone's
/// reader (see the header of the file); the micro encoder writes this form.
const SPEC_XCDR1: &str = include_str!("fixtures/xtypes_datapath/spec_xcdr1_vectors.txt");

/// Corpus types the micro backend refuses, with the reason it gives: it
/// does not map bitmasks.
const REFUSED: &[(&str, &str)] = &[
    ("Aliases", "bitmask"),
    ("MutLC", "bitmask"),
    ("MutLC2", "bitmask"),
];

/// (sample, Rust type, Rust value; `S` and `V` build heapless strings and
/// vectors). The values are those `cyclone_probe.c` writes.
const SAMPLES: &[(&str, &str, &str)] = &[
    ("FinalPrims", "FinalPrims", "FinalPrims { a: 0x11, b: 0x0102_0304_0506_0708, c: -2, d: 1.5, e: true, f: 0xAABB_CCDD }"),
    ("NestFinal", "NestFinal", "NestFinal { a: 1, in8: Inner8 { x: 0x1122_3344_5566_7788, y: 7 }, z: 9 }"),
    ("NestOdd", "NestOdd", "NestOdd { a: 1, il: InnerL { x: 0x0102_0304 }, s: 5 }"),
    ("NestApp", "NestApp", "NestApp { a: 1, app: AppStruct { a: 2, s: S(\"hi\"), b: 3 }, z: 4 }"),
    ("DerivedF", "DerivedF", "DerivedF { base: BaseF { id: 1, v: 2.5 }, s: 3, w: 4 }"),
    ("DerivedA", "DerivedA", "DerivedA { base: BaseA { id: 1 }, name: S(\"n\") }"),
    ("Enums", "Enums", "Enums { a: E8::E8_B, b: E16::E16_B, c: E32::E32_C, x: 0xEE, d: E16::E16_A, e: E8::E8_C }"),
    ("AppEnums", "AppEnums", "AppEnums { p: 1, b: E16::E16_B }"),
    ("Seqs", "Seqs", "Seqs { p: 1, u: V(&[1, 2]), bd: V(&[0.5]), q: 2, s: V(&[3, 4, 5]), bl: V(&[6]) }"),
    ("SeqStruct", "SeqStruct", "SeqStruct { p: 1, v: V(&[Inner8 { x: 1, y: 2 }, Inner8 { x: 3, y: 4 }]) }"),
    ("SeqApp", "SeqApp", "SeqApp { p: 1, v: V(&[AppStruct { a: 1, s: S(\"a\"), b: 2 }, AppStruct { a: 3, s: S(\"\"), b: 4 }]) }"),
    ("SeqStr", "SeqStr", "SeqStr { p: 1, v: V(&[S(\"ab\"), S(\"c\")]), b: V(&[S(\"wxyz\")]) }"),
    ("Arrs", "Arrs", "Arrs { p: 1, a: [1, 2, 3], m: [[1, 2, 3], [4, 5, 6]], s: [[1, 2], [3, 4]] }"),
    ("ArrStruct", "ArrStruct", "ArrStruct { p: 1, a: [Inner8 { x: 1, y: 2 }, Inner8 { x: 3, y: 4 }] }"),
    ("HoldU_1", "HoldU", "HoldU { p: 1, u: UL::A(0x0A), z: 9 }"),
    ("HoldU_2", "HoldU", "HoldU { p: 1, u: UL::S(S(\"hello\")), z: 9 }"),
    ("HoldU_3", "HoldU", "HoldU { p: 1, u: UL::D(0.25), z: 9 }"),
    ("HoldU_def0", "HoldU", "HoldU { p: 1, u: UL::O(0x7F), z: 9 }"),
    ("HoldUA_1", "HoldUA", "HoldUA { p: 1, u: UA::A(5) }"),
    ("HoldUA_2", "HoldUA", "HoldUA { p: 1, u: UA::S(S(\"ab\")) }"),
    ("HoldUE_A", "HoldUE", "HoldUE { p: 1, u: UE::A(6), z: 2 }"),
    ("HoldUE_B", "HoldUE", "HoldUE { p: 1, u: UE::B(7), z: 2 }"),
    ("HoldUS_1", "HoldUS", "HoldUS { p: 1, u: US::A(0x11), q: 2 }"),
    ("HoldUS_2", "HoldUS", "HoldUS { p: 1, u: US::B(0x2233), q: 2 }"),
    ("HoldUS_3", "HoldUS", "HoldUS { p: 1, u: US::C(0x4455_6677), q: 2 }"),
    ("BStr", "BStr", "BStr { p: 1, s: S(\"abcdefgh\"), z: 2 }"),
    ("KeyedApp", "KeyedApp", "KeyedApp { id: 1, v: 2 }"),
    ("MutDefault", "MutDefault", "MutDefault { a: 1, s: S(\"abc\"), d: 2.0, o: 7 }"),
    ("MutIds", "MutIds", "MutIds { a: 1, b: 2, c: 3, d: 4 }"),
    ("MutHash", "MutHash", "MutHash { a: 1, b: 2, c: 3, d: 4 }"),
    ("MutHashid", "MutHashid", "MutHashid { x: 1, y: 2, z: 3 }"),
    ("MutPrims", "MutPrims", "MutPrims { x: 1, y: 2, z: 3 }"),
    ("MutCompact2", "MutCompact2", "MutCompact2 { a: 1, b: 2, c: 3 }"),
    ("NestMut", "NestMut", "NestMut { p: 1, m: MutPrims { x: 1, y: 2, z: 3 }, q: 2 }"),
    ("MutOpt_all", "MutOpt", "MutOpt { a: Some(5), b: Some(S(\"x\")), c: 6 }"),
    ("MutOpt_none", "MutOpt", "MutOpt { a: None, b: None, c: 6 }"),
    ("DerivedM", "DerivedM", "DerivedM { base: BaseM { id: 1, tag: S(\"t\") }, extra: 2, s: 3, after: 4 }"),
    ("MutEnums", "MutEnums", "MutEnums { a: E8::E8_C, b: E16::E16_B, c: E32::E32_B }"),
    ("Keyed", "Keyed", "Keyed { id: 1, name: S(\"nm\"), v: 3 }"),
    ("FinalOpt_all", "FinalOpt", "FinalOpt { t: 1, a: Some(5), b: Some(6), u: 2 }"),
    ("FinalOpt_none", "FinalOpt", "FinalOpt { t: 1, a: None, b: None, u: 2 }"),
    ("AppOpt_all", "AppOpt", "AppOpt { t: 1, a: Some(7) }"),
    ("AppOpt_none", "AppOpt", "AppOpt { t: 1, a: None }"),
    ("MutMix", "MutMix", "MutMix { sl: V(&[1, 2]), ss: V(&[S(\"a\")]), fin: Inner8 { x: 3, y: 4 }, app: AppStruct { a: 5, s: S(\"b\"), b: 6 }, r#mut: MutPrims { x: 7, y: 8, z: 9 }, un: UL::S(S(\"u\")), arr: [10, 11], bs: S(\"xy\"), en: E16::E16_B, se: V(&[E8::E8_B, E8::E8_C]) }"),
    ("SeqEnum", "SeqEnum", "SeqEnum { p: 1, a: V(&[E8::E8_C, E8::E8_A, E8::E8_B]), b: V(&[E32::E32_B]), arr: [E8::E8_B, E8::E8_C, E8::E8_A] }"),
    ("WStr", "WStr", "WStr { p: 1, c: 0x41, s: S(\"h\\u{e9}\"), d: 0x263A }"),
    ("ArrStr", "ArrStr", "ArrStr { p: 1, a: [S(\"ab\"), S(\"\")], b: [S(\"xyz\"), S(\"q\")] }"),
    ("SeqChar", "SeqChar", "SeqChar { p: 1, c: V(&[b'x', b'y']), w: V(&[0x41, 0x42]), s: S(\"ab\") }"),
    ("MutSeqChar", "MutSeqChar", "MutSeqChar { c: V(&[b'q']), w: V(&[0x5A]), z: 5 }"),
    ("MutW", "MutW", "MutW { a: 1, b: 2, s: S(\"w\") }"),
    ("AppW", "AppW", "AppW { a: 1, b: 2 }"),
    ("NestAppW", "NestAppW", "NestAppW { p: 1, w: AppW { a: 3, b: 4 }, z: 5 }"),
    ("HB_T", "HoldUB2", "HoldUB2 { p: 1, u: UB2::T(0x11) }"),
    ("HB_F", "HoldUB2", "HoldUB2 { p: 1, u: UB2::F(0x22) }"),
    ("HE_A", "HoldUE2", "HoldUE2 { p: 1, u: UE2::A(6) }"),
    ("HE_B", "HoldUE2", "HoldUE2 { p: 1, u: UE2::B(7) }"),
    ("HC_A", "HoldUC2", "HoldUC2 { p: 1, u: UC2::X(5) }"),
    ("HC_B", "HoldUC2", "HoldUC2 { p: 1, u: UC2::Y(7) }"),
];

/// @mutable unions built from the specification: the form hddsgen writes
/// (`spec=`) and another form a reader must accept (`alt=`), see the file's
/// header; the micro encoder writes the former and reads both.
const SPEC_UNION: &str = include_str!("fixtures/xtypes_datapath/spec_mutable_union.txt");
const SPEC_UNION_IDL: &str = "@mutable union UM switch (long) { case 1: long a; case 2: string s; \
    case 3: double d; };\n@final struct HoldUM { octet p; UM u; long z; };\n";
const SPEC_UNION_HASHED: &str =
    include_str!("fixtures/xtypes_datapath/spec_mutable_union_hashed.txt");
const SPEC_UNION_HASHED_IDL: &str =
    "@mutable @autoid(HASH) union UMh switch (long) { case 1: long color; \
    case 2: short shapesize; };\n@final struct HoldUMh { octet p; UMh u; };\n";
const SPEC_UNION_HASHED_SAMPLES: &[(&str, &str)] = &[
    ("HoldUMh_1", "HoldUMh { p: 1, u: UMh::Color(0x11) }"),
    ("HoldUMh_2", "HoldUMh { p: 1, u: UMh::Shapesize(0x22) }"),
];
const SPEC_UNION_SAMPLES: &[(&str, &str)] = &[
    ("HoldUM_1", "HoldUM { p: 1, u: UM::A(10), z: 9 }"),
    ("HoldUM_2", "HoldUM { p: 1, u: UM::S(S(\"hello\")), z: 9 }"),
    ("HoldUM_3", "HoldUM { p: 1, u: UM::D(0.25), z: 9 }"),
];

/// Body of an encapsulated buffer: the header and the trailing padding
/// its options count removed.
fn body_of(encapsulated: &str) -> &str {
    let pad = usize::from(u8::from_str_radix(&encapsulated[6..8], 16).unwrap_or(0) & 3);
    &encapsulated[8..encapsulated.len() - 2 * pad]
}

/// Types generated next to the corpus for the evolution checks: readers
/// lacking members of the writer's type, or having members it lacks.
const EVOLUTION_IDL: &str = "
@mutable struct MutDefaultNarrow { long a; @id(3) octet o; };
@mutable struct KeyedNarrow { @id(2) long v; };
@mutable struct MutWider { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) long added; };
@appendable struct AppWider { long a; long b; long added; };
@mutable struct MutWiderAll { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) E8 e; @id(5) UL u; @id(6) UE ue; @id(7) Inner8 i; @id(8) long arr[2]; @id(9) sequence<long> q; @id(11) @optional long o; @id(12) string<4> bs; @id(13) UA ua; @id(15) UZ uz; @id(14) wstring<3> ws; @id(16) boolean flag; };
@final union UZ switch (long) { case 1: long a; case 0: short z; default: octet o; };
@appendable struct AppWiderAll { long a; long b; E8 e; UL u; Inner8 i; @optional long o; };
";

/// (vector, reader type, expected: a value, or `None` for a sample that
/// must be refused). A MUTABLE reader skips a member it does not know
/// unless that member must be understood (7.2.2.4.4.4.6): the
/// specification's form of `Keyed` flags its key members, Cyclone's does
/// not. A member the reader has and the data lacks takes its default
/// (7.4.1.2.3, DDS-XTypes 1.3 text lines 6195-6203; Table 9 line 2310): a
/// MUTABLE member absent from the parameter list, an APPENDABLE member past
/// the end of the data (XCDR1 has no DHEADER, so only the outermost type).
const EVOLUTION: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "MutDefault",
        "cyclone",
        "MutDefaultNarrow",
        Some("MutDefaultNarrow { a: 1, o: 7 }"),
    ),
    (
        "MutW",
        "cyclone",
        "MutWider",
        Some("MutWider { a: 1, b: 2, s: S(\"w\"), added: 0 }"),
    ),
    (
        "MutW",
        "spec",
        "MutWider",
        Some("MutWider { a: 1, b: 2, s: S(\"w\"), added: 0 }"),
    ),
    (
        "MutW",
        "cyclone",
        "MutWiderAll",
        Some("MutWiderAll { a: 1, b: 2, s: S(\"w\"), e: E8::E8_A, u: UL::O(0), ue: UE::A(0), i: Inner8 { x: 0, y: 0 }, arr: [0, 0], q: V(&[]), o: None, bs: S(\"\"), ua: UA::A(0), ws: V(&[]), uz: UZ::Z(0), flag: false }"),
    ),
    (
        "AppW",
        "cyclone",
        "AppWider",
        Some("AppWider { a: 1, b: 2, added: 0 }"),
    ),
    (
        "AppW",
        "cyclone",
        "AppWiderAll",
        Some("AppWiderAll { a: 1, b: 2, e: E8::E8_A, u: UL::O(0), i: Inner8 { x: 0, y: 0 }, o: None }"),
    ),
    ("Keyed", "spec", "KeyedNarrow", None),
    (
        "Keyed",
        "cyclone",
        "KeyedNarrow",
        Some("KeyedNarrow { v: 3 }"),
    ),
];

/// Decoded only: Cyclone writes a `default` discriminator the Rust variant
/// does not keep (it re-encodes the smallest free value, `HoldU_def0`).
const DECODE_ONLY: &[(&str, &str, &str)] =
    &[("HoldU_def", "HoldU", "HoldU { p: 1, u: UL::O(0x7F), z: 9 }")];

fn hdds_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        dir.join("crates/hdds-micro/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={:?} must name an HDDS checkout (crates/hdds-micro): \
         these micro data path tests did NOT run",
        dir.display().to_string()
    );
    dir
}

fn generate(idl: &str) -> hddsgen::error::Result<String> {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::Micro.generator().generate(&ast)
}

/// The corpus without the refused types (one type per line).
fn micro_corpus() -> String {
    let mut out = String::new();
    for l in CORPUS.lines().filter(|l| {
        !REFUSED.iter().any(|(t, _)| {
            l.contains(&format!("struct {t} ")) || l.contains(&format!("struct {t}:"))
        })
    }) {
        let _ = writeln!(out, "{l}");
    }
    out
}

/// The XCDR1 body the encoder must write: the specification's form of a
/// parameter list where Cyclone's departs from it, Cyclone's otherwise.
fn xcdr1(name: &str) -> String {
    SPEC_XCDR1
        .lines()
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == name)
                .then(|| it.nth(1)?.strip_prefix("spec=").map(str::to_string))
                .flatten()
        })
        .unwrap_or_else(|| cyclone_xcdr1(name))
}

/// Cyclone's XCDR1 body.
fn cyclone_xcdr1(name: &str) -> String {
    VECTORS
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == name && it.next()? == "xcdr1")
                .then(|| it.nth(1)?.strip_prefix("body=").map(str::to_string))
                .flatten()
        })
        .unwrap_or_else(|| panic!("no Cyclone xcdr1 vector for {name}"))
}

fn main_rs() -> String {
    let mut body = String::new();
    for (name, ty, value) in SAMPLES {
        let hex = xcdr1(name);
        let _ = writeln!(
            body,
            "    chk::<{ty}>(\"{name}\", &{value}, \"{hex}\", true, |v, e| v.encode(e), |d| {ty}::decode(d));"
        );
    }
    for (name, ty, value) in SAMPLES {
        let cyc = cyclone_xcdr1(name);
        if cyc != xcdr1(name) {
            let _ = writeln!(
                body,
                "    chk::<{ty}>(\"{name}/cyclone\", &{value}, \"{cyc}\", false, |v, e| v.encode(e), |d| {ty}::decode(d));"
            );
        }
    }
    for (name, form, ty, expected) in EVOLUTION {
        let hex = if *form == "spec" {
            xcdr1(name)
        } else {
            cyclone_xcdr1(name)
        };
        let expected = expected.map_or_else(|| "None".to_string(), |v| format!("Some({v})"));
        let _ = writeln!(
            body,
            "    evo::<{ty}>(\"{name} {form} as {ty}\", \"{hex}\", |d| {ty}::decode(d), {expected});"
        );
    }
    for (l, ty, samples) in SPEC_UNION
        .lines()
        .map(|l| (l, "HoldUM", SPEC_UNION_SAMPLES))
        .chain(
            SPEC_UNION_HASHED
                .lines()
                .map(|l| (l, "HoldUMh", SPEC_UNION_HASHED_SAMPLES)),
        )
        .filter(|(l, _, _)| !l.starts_with('#'))
    {
        let mut it = l.split_whitespace();
        let (Some(name), Some("xcdr1"), Some(alt), Some(spec_form)) =
            (it.next(), it.next(), it.next(), it.next())
        else {
            continue;
        };
        let alt = body_of(alt.trim_start_matches("alt="));
        let spec_form = body_of(spec_form.trim_start_matches("spec="));
        let value = samples
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .expect("sample");
        let _ = writeln!(
            body,
            "    chk::<{ty}>(\"{name}/spec\", &{value}, \"{spec_form}\", true, |v, e| v.encode(e), |d| {ty}::decode(d));"
        );
        let _ = writeln!(
            body,
            "    chk::<{ty}>(\"{name}/alt\", &{value}, \"{alt}\", false, |v, e| v.encode(e), |d| {ty}::decode(d));"
        );
        let _ = writeln!(
            body,
            "    size_ok::<{ty}>(\"{name}\", {ty}::ENCODED_SIZE_MAX, \"{spec_form}\");"
        );
    }
    // ENCODED_SIZE_MAX is reached by the largest value: a @mutable union
    // holding a string at its capacity (64): 4 (p, padded) + 12 + 4
    // (discriminator parameter) + 12 + 4 + 65 (string parameter) + 3
    // (padding) + 4 (PID_LIST_END) + 4 (z) = 112.
    body.push_str(
        "    { let v = HoldUM { p: 1, u: UM::S(S(&\"x\".repeat(64))), z: 9 }; let mut buf = [0u8; 512]; let mut e = CdrEncoder::new(&mut buf); \
         match v.encode(&mut e) { Ok(()) if e.position() == 112 && HoldUM::ENCODED_SIZE_MAX == 112 => println!(\"TIGHT_OK HoldUM\"), \
         r => println!(\"TIGHT_FAIL HoldUM {r:?} {} {}\", e.position(), HoldUM::ENCODED_SIZE_MAX) } }\n",
    );
    for (name, ty, value) in DECODE_ONLY {
        let hex = xcdr1(name);
        let _ = writeln!(
            body,
            "    chk::<{ty}>(\"{name}\", &{value}, \"{hex}\", false, |v, e| v.encode(e), |d| {ty}::decode(d));"
        );
    }
    format!(
        "#![allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, clippy::all)]\n\
         #[path = \"generated.rs\"]\nmod generated;\n\
         use generated::*;\n\
         fn S<const N: usize>(s: &str) -> heapless::String<N> {{ heapless::String::try_from(s).expect(\"fits\") }}\n\
         fn V<T: Clone, const N: usize>(v: &[T]) -> heapless::Vec<T, N> {{ heapless::Vec::from_slice(v).expect(\"fits\") }}\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         fn unhex(s: &str) -> Vec<u8> {{ (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }}\n\
         fn chk<T: PartialEq + core::fmt::Debug>(name: &str, v: &T, exp: &str, do_enc: bool,\n\
             enc: impl Fn(&T, &mut CdrEncoder) -> Result<(), CdrError>,\n\
             dec: impl Fn(&mut CdrDecoder) -> Result<T, CdrError>) {{\n\
             if do_enc {{\n\
                 let mut buf = [0xAAu8; 2048];\n\
                 let mut e = CdrEncoder::new(&mut buf);\n\
                 match enc(v, &mut e) {{\n\
                     Ok(()) => {{ let got = hex(e.finish()); if got == exp {{ println!(\"ENC_OK {{name}}\"); }} else {{ println!(\"ENC_FAIL {{name}}\\n  got {{got}}\\n  exp {{exp}}\"); }} }}\n\
                     Err(err) => println!(\"ENC_FAIL {{name}} {{err:?}}\"),\n\
                 }}\n\
             }}\n\
             let bytes = unhex(exp);\n\
             let mut d = CdrDecoder::new(&bytes);\n\
             match dec(&mut d) {{\n\
                 Ok(x) if &x == v => println!(\"DEC_OK {{name}}\"),\n\
                 Ok(x) => println!(\"DEC_FAIL {{name}} got {{x:?}}\"),\n\
                 Err(err) => println!(\"DEC_FAIL {{name}} {{err:?}}\"),\n\
             }}\n\
         }}\n\
         fn evo<T: PartialEq + core::fmt::Debug>(label: &str, hex_in: &str, dec: impl Fn(&mut CdrDecoder) -> Result<T, CdrError>, expected: Option<T>) {{\n\
             let bytes = unhex(hex_in);\n\
             let mut d = CdrDecoder::new(&bytes);\n\
             let got = dec(&mut d).ok();\n\
             if got == expected {{ println!(\"EVO_OK {{label}}\"); }} else {{ println!(\"EVO_FAIL {{label}} got {{got:?}}\"); }}\n\
         }}\n\
         fn size_ok<T>(name: &str, max: usize, exp: &str) {{\n\
             if exp.len() / 2 <= max {{ println!(\"MAX_OK {{name}}\"); }} else {{ println!(\"MAX_FAIL {{name}} {{max}} < {{}}\", exp.len() / 2); }}\n\
         }}\n\
         fn main() {{\n{body}{sizes}}}\n",
        sizes = SAMPLES.iter().fold(String::new(), |mut acc, (name, ty, _)| {
            let _ = writeln!(
                acc,
                "    size_ok::<{ty}>(\"{name}\", {ty}::ENCODED_SIZE_MAX, \"{}\");",
                xcdr1(name)
            );
            acc
        })
    )
}

fn run_crate(name: &str, deps: &str, files: &[(&str, String)]) -> String {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("xtypes_datapath")
        .join(name);
    fs::create_dir_all(root.join("src")).expect("crate dir");
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [dependencies]\n{deps}\n[workspace]\n"
    );
    fs::write(root.join("Cargo.toml"), manifest).expect("manifest");
    for (file, content) in files {
        fs::write(root.join("src").join(file), content).expect("source");
    }
    let output = Command::new(env!("CARGO"))
        .args(["run", "--offline", "--quiet"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .expect("cargo run");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{name} failed\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test xtypes_datapath_micro -- --ignored"]
fn micro_backend_matches_cyclone_xcdr1_on_the_corpus() {
    let hdds = hdds_dir();
    let deps = format!(
        "hdds-micro = {{ path = \"{}\", features = [\"heapless\"] }}\nheapless = \"0.8\"\n",
        hdds.join("crates/hdds-micro").display()
    );
    let generated = generate(&format!(
        "{}{EVOLUTION_IDL}{SPEC_UNION_IDL}{SPEC_UNION_HASHED_IDL}",
        micro_corpus()
    ))
    .expect("micro codegen of the corpus subset");
    let out = run_crate(
        "dp_micro",
        &deps,
        &[("generated.rs", generated), ("main.rs", main_rs())],
    );
    let count = |p: &str| out.lines().filter(|l| l.starts_with(p)).count();
    assert!(
        out.lines().filter(|l| l.contains("_FAIL")).count() == 0
            && count("ENC_OK")
                == SAMPLES.len() + (SPEC_UNION_SAMPLES.len() + SPEC_UNION_HASHED_SAMPLES.len())
            && count("DEC_OK")
                == SAMPLES.len()
                    + DECODE_ONLY.len()
                    + (SPEC_UNION_SAMPLES.len() + SPEC_UNION_HASHED_SAMPLES.len()) * 2
                    + SAMPLES
                        .iter()
                        .filter(|(n, _, _)| cyclone_xcdr1(n) != xcdr1(n))
                        .count()
            && count("MAX_OK")
                == SAMPLES.len() + (SPEC_UNION_SAMPLES.len() + SPEC_UNION_HASHED_SAMPLES.len())
            && count("EVO_OK") == EVOLUTION.len()
            && count("TIGHT_OK") == 1,
        "{out}"
    );
}

/// Every refused corpus type is refused alone, naming itself and the
/// reason (no silently wrong code).
#[test]
fn micro_backend_refuses_what_it_cannot_serialize() {
    for (ty, why) in REFUSED {
        let line = CORPUS
            .lines()
            .find(|l| l.contains(&format!("struct {ty} ")) || l.contains(&format!("struct {ty}:")))
            .expect("corpus line");
        // The refused type first, so that it is the first one generated;
        // the rest of the corpus after it, for the types it references.
        let mut rest = String::new();
        for l in CORPUS.lines().filter(|l| *l != line) {
            let _ = writeln!(rest, "{l}");
        }
        let idl = format!("{line}\n{rest}");
        let err = generate(&idl).expect_err("refused");
        let msg = err.to_string();
        assert!(
            msg.contains(ty) && msg.contains(why),
            "{ty}: expected a refusal naming it and `{why}`, got: {msg}"
        );
    }
}
