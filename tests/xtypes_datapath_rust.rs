// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated Rust code against a foreign oracle.
//!
//! `fixtures/xtypes_datapath/corpus.idl` is compiled by hddsgen; one sample
//! of each type is encoded under XCDR1 and XCDR2 and must give the bytes
//! Cyclone DDS 0.11 serializes for the same sample
//! (`cyclone_vectors.txt`, produced by `cyclone_probe.c`), and Cyclone's
//! bytes must decode to the sample. The `TypeObject` each generated type
//! announces must hash to the complete `TypeObject` Cyclone's idlc emits for
//! it (`cyclone_typeobjects.txt`, extracted from idlc's `TYPE_MAP_CDR` by
//! `extract_typeobjects.py`, every hash checked against the MD5 of its
//! object).
//!
//! The generated code depends on the `hdds` crate of the HDDS repository:
//! run with `HDDSGEN_HDDS_DIR=<hdds checkout> cargo test --test
//! xtypes_datapath_rust -- --ignored`. Ignored by default, failing when
//! requested without the checkout. The checkout must carry the
//! Cyclone-exact `TypeObject` model (`enum_flags` / `bitmask_flags`, alias
//! bodies with annotations, `base_type` as an identifier): the generated
//! code fills those fields, and an older `hdds` does not compile it.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, Parser};

const CORPUS: &str = include_str!("fixtures/xtypes_datapath/corpus.idl");
const VECTORS: &str = include_str!("fixtures/xtypes_datapath/cyclone_vectors.txt");
/// The XCDR1 parameter lists as the specification writes them (exact member
/// lengths, `FLAG_2` on key members), derived from Cyclone's by Cyclone's
/// reader: see the header of the file.
const SPEC_XCDR1: &str = include_str!("fixtures/xtypes_datapath/spec_xcdr1_vectors.txt");
/// A mutable two-key type laid out the ways a writer may lay it out, each
/// with the verdict the specification gives (built by `spec_vectors.py`,
/// see the file's header).
const PLPROBE: &str = include_str!("fixtures/xtypes_datapath/spec_plprobe.txt");
const PLPROBE_IDL: &str = "@mutable struct PlProbe { @key @id(1) long id; @id(2) short s; \
    @id(3) string name; @id(16144) long long big; @key @id(5) octet tag; };\n";
/// @mutable unions, which Cyclone does not support: the form hddsgen writes
/// and another form a reader must accept, both built from the specification
/// (see the file's header).
const SPEC_UNION: &str = include_str!("fixtures/xtypes_datapath/spec_mutable_union.txt");
const SPEC_UNION_IDL: &str = "@mutable union UM switch (long) { case 1: long a; case 2: string s; \
    case 3: double d; };\n@final struct HoldUM { octet p; UM u; long z; };\n";
const SPEC_UNION_SAMPLES: &[(&str, &str)] = &[
    ("HoldUM_1", "HoldUM { p: 1, u: UM::A(10), z: 9 }"),
    ("HoldUM_2", "HoldUM { p: 1, u: UM::S(S(\"hello\")), z: 9 }"),
    ("HoldUM_3", "HoldUM { p: 1, u: UM::D(0.25), z: 9 }"),
];

/// Maps: Fast CDR 2.3.3's bytes (see the file's header).
const FASTCDR_MAPS: &str = include_str!("fixtures/xtypes_datapath/fastcdr_maps.txt");
const FASTCDR_MAPS_IDL: &str =
    "@final struct HoldMap { long p; map<long, string> m; map<long, long> pm; };\n";
const FASTCDR_MAPS_SAMPLES: &[(&str, &str, bool)] = &[
    (
        "one",
        "HoldMap { p: 7, m: [(1, S(\"a\"))].into_iter().collect(), pm: [(3, 4)].into_iter().collect() }",
        true,
    ),
    // Two entries: the Rust map's iteration order is not Fast CDR's, so
    // only decoded.
    (
        "two",
        "HoldMap { p: 7, m: [(1, S(\"a\")), (2, S(\"bc\"))].into_iter().collect(), pm: [(3, 4)].into_iter().collect() }",
        false,
    ),
];

/// The @autoid(HASH) mutable union: the spec form and another one a reader
/// must accept (see the file's header).
const SPEC_UNION_HASHED: &str =
    include_str!("fixtures/xtypes_datapath/spec_mutable_union_hashed.txt");
const SPEC_UNION_HASHED_IDL: &str =
    "@mutable @autoid(HASH) union UMh switch (long) { case 1: long color; \
    case 2: short shapesize; };\n@final struct HoldUMh { octet p; UMh u; };\n";
const SPEC_UNION_HASHED_SAMPLES: &[(&str, &str)] = &[
    ("HoldUMh_1", "HoldUMh { p: 1, u: UMh::Color(0x11) }"),
    ("HoldUMh_2", "HoldUMh { p: 1, u: UMh::Shapesize(0x22) }"),
];

/// Body of an encapsulated buffer: the 4-byte header and the trailing
/// padding its options count removed.
fn body_of(encapsulated: &str) -> &str {
    let pad = usize::from(u8::from_str_radix(&encapsulated[6..8], 16).unwrap_or(0) & 3);
    &encapsulated[8..encapsulated.len() - 2 * pad]
}

const PLPROBE_SAMPLE: &str =
    "PlProbe { id: 0x1122_3344, s: -2, name: S(\"hdds\"), big: 0x0102_0304_0506_0708, tag: 0xab }";
const TYPEOBJECTS: &str = include_str!("fixtures/xtypes_datapath/cyclone_typeobjects.txt");
const TYPEOBJECT_EXTRA_IDL: &str = include_str!("fixtures/xtypes_datapath/typeobject_extra.idl");
const TYPEOBJECTS_EXTRA: &str =
    include_str!("fixtures/xtypes_datapath/cyclone_typeobjects_extra.txt");

/// `typeobject_extra.idl` without the corpus type it repeats.
fn typeobject_extra_idl() -> String {
    TYPEOBJECT_EXTRA_IDL
        .lines()
        .filter(|l| !l.starts_with("@final struct Inner8 "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// (sample name in `cyclone_vectors.txt`, Rust type, Rust value with `S`
/// the string constructor). The values are those `cyclone_probe.c` writes.
const SAMPLES: &[(&str, &str, &str)] = &[
    ("FinalPrims", "FinalPrims", "FinalPrims { a: 0x11, b: 0x0102_0304_0506_0708, c: -2, d: 1.5, e: true, f: 0xAABB_CCDD }"),
    ("NestFinal", "NestFinal", "NestFinal { a: 1, in8: Inner8 { x: 0x1122_3344_5566_7788, y: 7 }, z: 9 }"),
    ("NestOdd", "NestOdd", "NestOdd { a: 1, il: InnerL { x: 0x0102_0304 }, s: 5 }"),
    ("NestApp", "NestApp", "NestApp { a: 1, app: AppStruct { a: 2, s: S(\"hi\"), b: 3 }, z: 4 }"),
    ("MutDefault", "MutDefault", "MutDefault { a: 1, s: S(\"abc\"), d: 2.0, o: 7 }"),
    ("MutIds", "MutIds", "MutIds { a: 1, b: 2, c: 3, d: 4 }"),
    ("MutHash", "MutHash", "MutHash { a: 1, b: 2, c: 3, d: 4 }"),
    ("MutHashid", "MutHashid", "MutHashid { x: 1, y: 2, z: 3 }"),
    ("MutPrims", "MutPrims", "MutPrims { x: 1, y: 2, z: 3 }"),
    ("MutCompact2", "MutCompact2", "MutCompact2 { a: 1, b: 2, c: 3 }"),
    ("NestMut", "NestMut", "NestMut { p: 1, m: MutPrims { x: 1, y: 2, z: 3 }, q: 2 }"),
    ("MutOpt_all", "MutOpt", "MutOpt { a: Some(5), b: Some(S(\"x\")), c: 6 }"),
    ("MutOpt_none", "MutOpt", "MutOpt { a: None, b: None, c: 6 }"),
    ("DerivedF", "DerivedF", "DerivedF { base: BaseF { id: 1, v: 2.5 }, s: 3, w: 4 }"),
    ("DerivedA", "DerivedA", "DerivedA { base: BaseA { id: 1 }, name: S(\"n\") }"),
    ("DerivedM", "DerivedM", "DerivedM { base: BaseM { id: 1, tag: S(\"t\") }, extra: 2, s: 3, after: 4 }"),
    ("Enums", "Enums", "Enums { a: E8::E8_B, b: E16::E16_B, c: E32::E32_C, x: 0xEE, d: E16::E16_A, e: E8::E8_C }"),
    ("MutEnums", "MutEnums", "MutEnums { a: E8::E8_C, b: E16::E16_B, c: E32::E32_B }"),
    ("AppEnums", "AppEnums", "AppEnums { p: 1, b: E16::E16_B }"),
    ("Seqs", "Seqs", "Seqs { p: 1, u: vec![1, 2], bd: vec![0.5], q: 2, s: vec![3, 4, 5], bl: vec![6] }"),
    ("SeqStruct", "SeqStruct", "SeqStruct { p: 1, v: vec![Inner8 { x: 1, y: 2 }, Inner8 { x: 3, y: 4 }] }"),
    ("SeqApp", "SeqApp", "SeqApp { p: 1, v: vec![AppStruct { a: 1, s: S(\"a\"), b: 2 }, AppStruct { a: 3, s: S(\"\"), b: 4 }] }"),
    ("SeqStr", "SeqStr", "SeqStr { p: 1, v: vec![S(\"ab\"), S(\"c\")], b: vec![S(\"wxyz\")] }"),
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
    ("SeqChar", "SeqChar", "SeqChar { p: 1, c: vec!['x', 'y'], w: vec!['A', 'B'], s: S(\"ab\") }"),
    ("MutSeqChar", "MutSeqChar", "MutSeqChar { c: vec!['q'], w: vec!['Z'], z: 5 }"),
    ("HoldUS_1", "HoldUS", "HoldUS { p: 1, u: US::A(0x11), q: 2 }"),
    ("HoldUS_2", "HoldUS", "HoldUS { p: 1, u: US::B(0x2233), q: 2 }"),
    ("HoldUS_3", "HoldUS", "HoldUS { p: 1, u: US::C(0x4455_6677), q: 2 }"),
    ("BStr", "BStr", "BStr { p: 1, s: S(\"abcdefgh\"), z: 2 }"),
    ("Keyed", "Keyed", "Keyed { id: 1, name: S(\"nm\"), v: 3 }"),
    ("KeyedApp", "KeyedApp", "KeyedApp { id: 1, v: 2 }"),
    ("FinalOpt_all", "FinalOpt", "FinalOpt { t: 1, a: Some(5), b: Some(6), u: 2 }"),
    ("FinalOpt_none", "FinalOpt", "FinalOpt { t: 1, a: None, b: None, u: 2 }"),
    ("AppOpt_all", "AppOpt", "AppOpt { t: 1, a: Some(7) }"),
    ("AppOpt_none", "AppOpt", "AppOpt { t: 1, a: None }"),
    ("MutMix", "MutMix", "MutMix { sl: vec![1, 2], ss: vec![S(\"a\")], fin: Inner8 { x: 3, y: 4 }, app: AppStruct { a: 5, s: S(\"b\"), b: 6 }, r#mut: MutPrims { x: 7, y: 8, z: 9 }, un: UL::S(S(\"u\")), arr: [10, 11], bs: S(\"xy\"), en: E16::E16_B, se: vec![E8::E8_B, E8::E8_C] }"),
    ("SeqEnum", "SeqEnum", "SeqEnum { p: 1, a: vec![E8::E8_C, E8::E8_A, E8::E8_B], b: vec![E32::E32_B], arr: [E8::E8_B, E8::E8_C, E8::E8_A] }"),
    ("WStr", "WStr", "WStr { p: 1, c: 'A', s: S(\"h\\u{e9}\"), d: '\\u{263A}' }"),
    ("Aliases", "Aliases", "Aliases { p: 1, m: BM32_BM32_0 | BM32_BM32_5, n: BM8_BM8_1, l: vec![7, 8], ia: Inner8 { x: 9, y: 10 } }"),
    ("ArrStr", "ArrStr", "ArrStr { p: 1, a: [S(\"ab\"), S(\"\")], b: [S(\"xyz\"), S(\"q\")] }"),
    ("MutLC", "MutLC", "MutLC { s1: vec![1, 2, 3], s2: vec![4], s4: vec![5, 6], s8: vec![7], sd: vec![0.5, 1.0], sb: vec![true], a1: [1, 2, 3], a2: [4, 5], a4: [6, 7], s: S(\"st\"), si: vec![Inner8 { x: 1, y: 2 }], e: E32::E32_C, m: BM32_BM32_1, o: None, a8: [8], b: true, s0: vec![] }"),
    ("MutLC2", "MutLC2", "MutLC2 { ai: [Inner8 { x: 1, y: 2 }], ae: [E8::E8_B, E8::E8_C], sa: [S(\"s\")], bm: BM8_BM8_0, sbm: vec![BM8_BM8_1], ls: vec![3], ia: Inner8 { x: 4, y: 5 } }"),
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

/// Samples only decoded: Cyclone writes a `default` discriminator our union
/// representation does not keep (the Rust variant re-encodes the smallest
/// free value, `HoldU_def0`).
const DECODE_ONLY: &[(&str, &str, &str)] =
    &[("HoldU_def", "HoldU", "HoldU { p: 1, u: UL::O(0x7F), z: 9 }")];

/// Types hddsgen generates next to the corpus for the evolution checks: a
/// reader whose type lacks members the writer's type has, or has members
/// the writer's type lacks (the `Wider` readers of `MutW`, `AppW`,
/// `NestAppW`).
const EVOLUTION_IDL: &str = "
@mutable struct MutDefaultNarrow { long a; @id(3) octet o; };
@appendable struct KeyedAppNarrow { long id; };
@mutable struct KeyedNarrow { @id(2) long v; };
@appendable struct AppStructNarrow { long a; };
@final struct NestAppNarrow { octet a; AppStructNarrow app; long z; };
@mutable struct MutWider { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) long added; };
@appendable struct AppWider { long a; long b; long added; };
@final struct NestAppWider { octet p; AppWider w; long z; };
@mutable struct MutWiderAll { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) E8 e; @id(5) UL u; @id(6) UE ue; @id(7) Inner8 i; @id(8) long arr[2]; @id(9) sequence<long> q; @id(10) BM8 m; @id(11) @optional long o; @id(12) string<4> bs; @id(13) UA ua; @id(15) UZ uz; @id(16) boolean flag; };
@final union UZ switch (long) { case 1: long a; case 0: short z; default: octet o; };
@appendable struct AppWiderAll { long a; long b; E8 e; UL u; Inner8 i; @optional long o; };
";

/// `@try_construct` on struct and union members (DDS-XTypes 1.3
/// 7.3.1.2.1.12) and the `MemberFlag` `TRY_CONSTRUCT` bits each member
/// must carry (Annex B, spec text lines 12325-12326: `DISCARD` 01,
/// `USE_DEFAULT` 10, `TRIM` 11). No foreign oracle: Cyclone DDS 0.11 idlc
/// ignores the annotation (it warns so).
const TRY_CONSTRUCT_IDL: &str = "
@final struct TryC { long a; @try_construct(DISCARD) string<4> d; @try_construct(USE_DEFAULT) string<4> u; @try_construct string<4> n; @try_construct(TRIM) string<4> t; };
@appendable union TryU switch (long) { case 1: long a; case 2: @try_construct(TRIM) string<4> t; case 3: @try_construct(USE_DEFAULT) short u; };
";
const TRY_CONSTRUCT: &[(&str, &str)] = &[("TryC", "1, 1, 2, 2, 3"), ("TryU", "1, 3, 2")];

/// (Cyclone sample, version, reader type, expected decode: a value, or
/// `Err` for a sample that must be refused). Spec: a MUTABLE reader skips a
/// member it does not know unless its must-understand flag is set
/// (7.2.2.4.4.4.6), then the whole sample is discarded; an APPENDABLE
/// reader skips to the end of the DHEADER. A member the reader has and
/// the data lacks takes its default (7.4.1.2.3, DDS-XTypes 1.3 text lines
/// 6195-6203; Table 9 line 2310): a MUTABLE member absent from the
/// parameter list, an APPENDABLE member past the end of the DHEADER (of
/// the buffer under XCDR1). The `spec` form of `Keyed` under
/// XCDR1 carries `FLAG_2` on its key members, Cyclone's does not.
const EVOLUTION: &[(&str, &str, &str, &str)] = &[
    (
        "MutDefault",
        "xcdr2",
        "MutDefaultNarrow",
        "MutDefaultNarrow { a: 1, o: 7 }",
    ),
    (
        "MutDefault",
        "xcdr1",
        "MutDefaultNarrow",
        "MutDefaultNarrow { a: 1, o: 7 }",
    ),
    (
        "KeyedApp",
        "xcdr2",
        "KeyedAppNarrow",
        "KeyedAppNarrow { id: 1 }",
    ),
    (
        "NestApp",
        "xcdr2",
        "NestAppNarrow",
        "NestAppNarrow { a: 1, app: AppStructNarrow { a: 2 }, z: 4 }",
    ),
    (
        "MutW",
        "xcdr2",
        "MutWider",
        "MutWider { a: 1, b: 2, s: S(\"w\"), added: 0 }",
    ),
    (
        "MutW",
        "xcdr1",
        "MutWider",
        "MutWider { a: 1, b: 2, s: S(\"w\"), added: 0 }",
    ),
    (
        "MutW",
        "spec1",
        "MutWider",
        "MutWider { a: 1, b: 2, s: S(\"w\"), added: 0 }",
    ),
    (
        "MutW",
        "xcdr2",
        "MutWiderAll",
        "MutWiderAll { a: 1, b: 2, s: S(\"w\"), e: E8::E8_A, u: UL::O(0), ue: UE::A(0), i: Inner8 { x: 0, y: 0 }, arr: [0, 0], q: vec![], m: 0, o: None, bs: S(\"\"), ua: UA::A(0), uz: UZ::Z(0), flag: false }",
    ),
    (
        "MutW",
        "xcdr1",
        "MutWiderAll",
        "MutWiderAll { a: 1, b: 2, s: S(\"w\"), e: E8::E8_A, u: UL::O(0), ue: UE::A(0), i: Inner8 { x: 0, y: 0 }, arr: [0, 0], q: vec![], m: 0, o: None, bs: S(\"\"), ua: UA::A(0), uz: UZ::Z(0), flag: false }",
    ),
    ("AppW", "xcdr2", "AppWider", "AppWider { a: 1, b: 2, added: 0 }"),
    (
        "AppW",
        "xcdr2",
        "AppWiderAll",
        "AppWiderAll { a: 1, b: 2, e: E8::E8_A, u: UL::O(0), i: Inner8 { x: 0, y: 0 }, o: None }",
    ),
    (
        "AppW",
        "xcdr1",
        "AppWiderAll",
        "AppWiderAll { a: 1, b: 2, e: E8::E8_A, u: UL::O(0), i: Inner8 { x: 0, y: 0 }, o: None }",
    ),
    ("AppW", "xcdr1", "AppWider", "AppWider { a: 1, b: 2, added: 0 }"),
    (
        "NestAppW",
        "xcdr2",
        "NestAppWider",
        "NestAppWider { p: 1, w: AppWider { a: 3, b: 4, added: 0 }, z: 5 }",
    ),
    ("Keyed", "xcdr2", "KeyedNarrow", "Err"),
    ("Keyed", "spec1", "KeyedNarrow", "Err"),
    ("Keyed", "xcdr1", "KeyedNarrow", "KeyedNarrow { v: 3 }"),
];

/// The types whose generated `TypeObject` hashes to the one Cyclone's
/// idlc emits.
const TYPEOBJECT_EXACT: &[&str] = &[
    "FinalPrims",
    "Inner8",
    "NestFinal",
    "InnerL",
    "NestOdd",
    "AppStruct",
    "NestApp",
    "MutDefault",
    "MutIds",
    "MutHash",
    "MutPrims",
    "MutCompact2",
    "NestMut",
    "MutOpt",
    "BaseF",
    "BaseA",
    "BaseM",
    "Seqs",
    "SeqStruct",
    "SeqApp",
    "SeqStr",
    "Arrs",
    "ArrStruct",
    "BStr",
    "Keyed",
    "KeyedApp",
    "FinalOpt",
    "AppOpt",
    "ArrStr",
    "WStr",
    "SeqChar",
    "MutSeqChar",
    "E8",
    "E16",
    "E32",
    "BM32",
    "BM8",
    "Enums",
    "MutEnums",
    "AppEnums",
    "SeqEnum",
    "MutLC",
    "MutLC2",
    "DerivedF",
    "DerivedA",
    "DerivedM",
    "MutHashid",
    "Aliases",
    "EApp",
    "BMApp",
    "MyLong",
    "MyLong2",
    "LArr",
    "InnerSeq",
    "AliasMore",
    "ArrAlias",
];

/// The types whose `TypeObject` differs from Cyclone's because Cyclone does
/// not follow the specification: it numbers union cases from 0 where the
/// discriminator is member 0 and the cases start at 1 (DDS-XTypes 1.3
/// 7.3.1.2.1.1, spec text lines 3815-3826).
const TYPEOBJECT_CYCLONE_LIMITED: &[(&str, &str)] = &[
    ("UL", "union case ids from 0"),
    ("HoldU", "a member union's case ids from 0"),
    ("UA", "union case ids from 0"),
    ("HoldUA", "a member union's case ids from 0"),
    ("US", "union case ids from 0"),
    ("HoldUS", "a member union's case ids from 0"),
    ("UE", "union case ids from 0"),
    ("HoldUE", "a member union's case ids from 0"),
    ("MutMix", "a member union's case ids from 0"),
    ("UB2", "union case ids from 0"),
    ("HoldUB2", "a member union's case ids from 0"),
    ("UE2", "union case ids from 0"),
    ("HoldUE2", "a member union's case ids from 0"),
    ("UC2", "union case ids from 0"),
    ("HoldUC2", "a member union's case ids from 0"),
];

fn hdds_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        dir.join("crates/hdds/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={:?} must name an HDDS checkout (crates/hdds): \
         these data path tests did NOT run",
        dir.display().to_string()
    );
    dir
}

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::Rust.generator().generate(&ast).expect("codegen")
}

/// (sample, version, body hex) of every vector line.
fn vectors() -> Vec<(String, String, String)> {
    VECTORS
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            let ver = it.next()?;
            let _hdr = it.next()?;
            let body = it.next()?.strip_prefix("body=")?;
            Some((name.to_string(), ver.to_string(), body.to_string()))
        })
        .collect()
}

/// (sample, spec XCDR1 body hex).
fn spec_xcdr1() -> Vec<(String, String)> {
    SPEC_XCDR1
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let name = it.next()?;
            let _ver = it.next()?;
            let hex = it.next()?.strip_prefix("spec=")?;
            Some((name.to_string(), hex.to_string()))
        })
        .collect()
}

fn main_rs() -> String {
    let vecs = vectors();
    let spec = spec_xcdr1();
    let mut body = String::new();
    for (name, ty, value) in SAMPLES {
        for (ver, method) in [("xcdr1", "xcdr1"), ("xcdr2", "xcdr2")] {
            let hex = vecs
                .iter()
                .find(|(n, v, _)| n == name && v == ver)
                .map_or_else(
                    || panic!("no Cyclone vector for {name} {ver}"),
                    |(_, _, h)| h.clone(),
                );
            let spec_hex = spec
                .iter()
                .find(|(n, _)| ver == "xcdr1" && n == name)
                .map(|(_, h)| h.clone());
            // Encode to the specification's form where Cyclone departs from
            // it; read Cyclone's form as well.
            let expected = spec_hex.clone().unwrap_or_else(|| hex.clone());
            let _ = writeln!(
                body,
                "    chk(\"{name}\", \"{ver}\", &{value}, \"{expected}\", |v: &{ty}, b: &mut [u8]| v.encode_{method}_le(b), |v: &{ty}| v.max_{method}_size(), |b: &[u8]| {ty}::decode_{method}_le(b), true);"
            );
            if spec_hex.is_some() {
                let _ = writeln!(
                    body,
                    "    chk(\"{name}/cyclone\", \"{ver}\", &{value}, \"{hex}\", |v: &{ty}, b: &mut [u8]| v.encode_{method}_le(b), |v: &{ty}| v.max_{method}_size(), |b: &[u8]| {ty}::decode_{method}_le(b), false);"
                );
            }
        }
    }
    for (name, ty, value) in DECODE_ONLY {
        for ver in ["xcdr1", "xcdr2"] {
            let hex = vecs
                .iter()
                .find(|(n, v, _)| n == name && v == ver)
                .map(|(_, _, h)| h.clone())
                .expect("vector");
            let _ = writeln!(
                body,
                "    chk(\"{name}\", \"{ver}\", &{value}, \"{hex}\", |v: &{ty}, b: &mut [u8]| v.encode_{ver}_le(b), |v: &{ty}| v.max_{ver}_size(), |b: &[u8]| {ty}::decode_{ver}_le(b), false);"
            );
        }
    }
    for (sample, ver, ty, expected) in EVOLUTION {
        let hex = if *ver == "spec1" {
            spec.iter()
                .find(|(n, _)| n == sample)
                .map(|(_, h)| h.clone())
                .expect("spec vector")
        } else {
            vecs.iter()
                .find(|(n, v, _)| n == sample && v == ver)
                .map(|(_, _, h)| h.clone())
                .expect("vector")
        };
        let method = if *ver == "xcdr2" { "xcdr2" } else { "xcdr1" };
        let _ = writeln!(
            body,
            "    evo(\"{sample} {ver} as {ty}\", \"{hex}\", |b: &[u8]| {ty}::decode_{method}_le(b), {});",
            if *expected == "Err" {
                "None".to_string()
            } else {
                format!("Some({expected})")
            }
        );
    }
    // PlProbe: decode every layout with the specification's verdict; under
    // XCDR2 our bytes are X2-S1 (the must-understand flag on the two key
    // members, LC=5 for the string).
    for l in PLPROBE.lines().filter(|l| !l.starts_with('#')) {
        let mut it = l.split_whitespace();
        let (Some(label), Some(verdict), Some(hex)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let method = if hex.starts_with("0003") {
            "xcdr1"
        } else {
            "xcdr2"
        };
        let payload = &hex[8..];
        let _ = writeln!(
            body,
            "    evo(\"plprobe {label}\", \"{payload}\", |b: &[u8]| PlProbe::decode_{method}_le(b), {});",
            if verdict == "REJECT" {
                "None".to_string()
            } else {
                format!("Some({PLPROBE_SAMPLE})")
            }
        );
        if label == "X2-S1" {
            let _ = writeln!(
                body,
                "    chk(\"plprobe X2-S1\", \"xcdr2\", &{PLPROBE_SAMPLE}, \"{payload}\", |v: &PlProbe, b: &mut [u8]| v.encode_xcdr2_le(b), |v: &PlProbe| v.max_xcdr2_size(), |b: &[u8]| PlProbe::decode_xcdr2_le(b), true);"
            );
        }
    }
    // @mutable unions: encode to the specification's form, read it and the
    // alternative form.
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
        let (Some(name), Some(ver), Some(alt), Some(spec_form)) =
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
            "    chk(\"{name}/spec\", \"{ver}\", &{value}, \"{spec_form}\", |v: &{ty}, b: &mut [u8]| v.encode_{ver}_le(b), |v: &{ty}| v.max_{ver}_size(), |b: &[u8]| {ty}::decode_{ver}_le(b), true);"
        );
        let _ = writeln!(
            body,
            "    chk(\"{name}/alt\", \"{ver}\", &{value}, \"{alt}\", |v: &{ty}, b: &mut [u8]| v.encode_{ver}_le(b), |v: &{ty}| v.max_{ver}_size(), |b: &[u8]| {ty}::decode_{ver}_le(b), false);"
        );
    }
    // Maps, against Fast CDR.
    for l in FASTCDR_MAPS.lines().filter(|l| !l.starts_with('#')) {
        let mut it = l.split_whitespace();
        let (Some(which), Some(ver), Some(hex)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let (_, value, enc) = FASTCDR_MAPS_SAMPLES
            .iter()
            .find(|(w, _, _)| *w == which)
            .expect("sample");
        let _ = writeln!(
            body,
            "    chk(\"HoldMap {which}\", \"{ver}\", &{value}, \"{hex}\", |v: &HoldMap, b: &mut [u8]| v.encode_{ver}_le(b), |v: &HoldMap| v.max_{ver}_size(), |b: &[u8]| HoldMap::decode_{ver}_le(b), {enc});"
        );
    }
    // The `_at` entry points align against the start of the buffer: Inner8
    // encoded at offset 1 behind an octet lands where Cyclone puts NestFinal's
    // member, and decodes from there.
    for ver in ["xcdr1", "xcdr2"] {
        let hex = vecs
            .iter()
            .find(|(n, v, _)| n == "NestFinal" && v == ver)
            .map(|(_, _, h)| h.clone())
            .expect("NestFinal vector");
        let _ = writeln!(
            body,
            "    at(\"{ver}\", \"{hex}\", |v: &Inner8, b: &mut [u8], o: &mut usize| v.encode_{ver}_le_at(b, o), |b: &[u8], o: &mut usize| Inner8::decode_{ver}_le_at(b, o));"
        );
    }
    // A value that cannot be encoded (`bl` over its bound of 3):
    // `try_xcdrN_size` names the error, and encoding into `max_xcdrN_size`
    // bytes fails with the same one; a valid value has both sizes equal.
    // `string<8>` holding 9 characters: refused on encode, and its bytes
    // (length 10 with the NUL, laid out the same under both encodings)
    // refused on decode.
    for ver in ["xcdr1", "xcdr2"] {
        let _ = writeln!(
            body,
            "    bound(\"{ver}\", |v: &BStr, b: &mut [u8]| v.encode_{ver}_le(b), |b: &[u8]| BStr::decode_{ver}_le(b));"
        );
    }
    for (ty, want) in TRY_CONSTRUCT {
        let _ = writeln!(
            body,
            "    tc(\"{ty}\", __hdds_type_object_{ty}(), &[{want}]);"
        );
    }
    for ver in ["xcdr1", "xcdr2"] {
        let _ = writeln!(
            body,
            "    try_size(\"{ver}\", |v: &Seqs| v.max_{ver}_size(), |v: &Seqs| v.try_{ver}_size(), |v: &Seqs, b: &mut [u8]| v.encode_{ver}_le(b));"
        );
    }
    // The objects a type announces next to its own: every named type it
    // reaches, the alias objects (and an alias's own target) included.
    let _ = writeln!(
        body,
        "    {{ let mut n: Vec<String> = <AliasMore as DDS>::nested_type_objects().iter().filter_map(|o| o.type_name().map(str::to_string)).collect(); n.sort();\n\
         \x20     if n == [\"BMApp\", \"EApp\", \"Inner8\", \"InnerSeq\", \"LArr\", \"MyLong\", \"MyLong2\"] {{ println!(\"NESTED_OK\"); }} else {{ println!(\"NESTED_FAIL {{n:?}}\"); }} }}"
    );
    for ty in TYPEOBJECT_EXACT
        .iter()
        .chain(TYPEOBJECT_CYCLONE_LIMITED.iter().map(|(t, _)| t))
    {
        let _ = writeln!(body, "    to_object(\"{ty}\", __hdds_type_object_{ty}());");
    }
    let mut tos = String::new();
    for l in TYPEOBJECTS.lines().chain(TYPEOBJECTS_EXTRA.lines()) {
        let mut it = l.split_whitespace();
        let (Some(h), Some(_ok), Some(obj)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        let _ = writeln!(tos, "    (\"{h}\", \"{obj}\"),");
    }
    format!(
        "#![allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, clippy::all)]\n\
         mod generated {{ include!(\"generated.rs\"); }}\n\
         use generated::*;\n\
         use hdds::api::DDS;\n\
         use hdds::Cdr2Decode;\n\
         use std::collections::HashMap;\n\
         fn S(s: &str) -> String {{ s.to_string() }}\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         fn unhex(s: &str) -> Vec<u8> {{ (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }}\n\
         fn chk<T: PartialEq + std::fmt::Debug>(name: &str, ver: &str, v: &T, exp: &str,\n\
             enc: impl Fn(&T, &mut [u8]) -> Result<usize, hdds::CdrError>,\n\
             size: impl Fn(&T) -> usize,\n\
             dec: impl Fn(&[u8]) -> Result<(T, usize), hdds::CdrError>, do_enc: bool) {{\n\
             if do_enc {{\n\
                 let mut buf = vec![0xAAu8; 4096];\n\
                 match enc(v, &mut buf) {{\n\
                     Ok(n) => {{\n\
                         let got = hex(&buf[..n]);\n\
                         if got == exp && size(v) == n {{ println!(\"ENC_OK {{name}} {{ver}}\"); }}\n\
                         else {{ println!(\"ENC_FAIL {{name}} {{ver}} size={{}}\\n  got {{got}}\\n  exp {{exp}}\", size(v)); }}\n\
                     }}\n\
                     Err(e) => println!(\"ENC_FAIL {{name}} {{ver}} error {{e:?}}\"),\n\
                 }}\n\
                 let need = exp.len() / 2;\n\
                 if need > 0 {{\n\
                     let mut short = vec![0u8; need - 1];\n\
                     match enc(v, &mut short) {{\n\
                         Err(hdds::CdrError::BufferTooSmall) => println!(\"SHORT_OK {{name}} {{ver}}\"),\n\
                         other => println!(\"SHORT_FAIL {{name}} {{ver}} {{other:?}}\"),\n\
                     }}\n\
                 }}\n\
             }}\n\
             let bytes = unhex(exp);\n\
             match dec(&bytes) {{\n\
                 Ok((d, n)) if &d == v && n == bytes.len() => println!(\"DEC_OK {{name}} {{ver}}\"),\n\
                 Ok((d, n)) => println!(\"DEC_FAIL {{name}} {{ver}} used {{n}} of {{}}\\n  got {{d:?}}\", bytes.len()),\n\
                 Err(e) => println!(\"DEC_FAIL {{name}} {{ver}} error {{e:?}}\"),\n\
             }}\n\
         }}\n\
         fn evo<T: PartialEq + std::fmt::Debug>(label: &str, hex_in: &str,\n\
             dec: impl Fn(&[u8]) -> Result<(T, usize), hdds::CdrError>, expected: Option<T>) {{\n\
             let got = dec(&unhex(hex_in)).ok().map(|(v, _)| v);\n\
             if got == expected {{ println!(\"EVO_OK {{label}}\"); }}\n\
             else {{ println!(\"EVO_FAIL {{label}} got {{got:?}} expected {{expected:?}}\"); }}\n\
         }}\n\
         fn at<T: PartialEq + std::fmt::Debug>(ver: &str, nest_final: &str,\n\
             enc: impl Fn(&T, &mut [u8], &mut usize) -> Result<(), hdds::CdrError>,\n\
             dec: impl Fn(&[u8], &mut usize) -> Result<T, hdds::CdrError>) where T: From<(i64, i32)> {{\n\
             let v = T::from((0x1122_3344_5566_7788, 7));\n\
             let exp = unhex(nest_final);\n\
             let mut buf = vec![0xAAu8; 64];\n\
             buf[0] = 1;\n\
             let mut off = 1usize;\n\
             let ok_enc = enc(&v, &mut buf, &mut off).is_ok() && off == exp.len() - 1 && buf[..off] == exp[..off];\n\
             let mut off2 = 1usize;\n\
             let ok_dec = dec(&exp, &mut off2).map(|d| d == v).unwrap_or(false) && off2 == exp.len() - 1;\n\
             if ok_enc && ok_dec {{ println!(\"AT_OK {{ver}}\"); }} else {{ println!(\"AT_FAIL {{ver}} enc {{ok_enc}} dec {{ok_dec}} off {{off}} got {{}}\", hex(&buf[..off])); }}\n\
         }}\n\
         fn bound(ver: &str, enc: impl Fn(&BStr, &mut [u8]) -> Result<usize, hdds::CdrError>,\n\
             dec: impl Fn(&[u8]) -> Result<(BStr, usize), hdds::CdrError>) {{\n\
             let over = BStr {{ p: 1, s: S(\"abcdefghi\"), z: 2 }};\n\
             let mut b = vec![0u8; 64];\n\
             let bytes = unhex(\"010000000a0000006162636465666768690000000200000000\");\n\
             let (e, d) = (enc(&over, &mut b), dec(&bytes[..24]));\n\
             if e.is_err() && d.is_err() {{ println!(\"BOUND_OK {{ver}}\"); }} else {{ println!(\"BOUND_FAIL {{ver}} {{e:?}} {{d:?}}\"); }}\n\
         }}\n\
         fn try_size(ver: &str, max: impl Fn(&Seqs) -> usize,\n\
             tr: impl Fn(&Seqs) -> Result<usize, hdds::CdrError>,\n\
             enc: impl Fn(&Seqs, &mut [u8]) -> Result<usize, hdds::CdrError>) {{\n\
             let over = Seqs {{ p: 1, u: vec![], bd: vec![], q: 0, s: vec![], bl: vec![1, 2, 3, 4] }};\n\
             let ok = Seqs {{ bl: vec![1], ..over.clone() }};\n\
             let mut b = vec![0u8; max(&over)];\n\
             let (t, e) = (tr(&over), enc(&over, &mut b));\n\
             if t.is_err() && format!(\"{{t:?}}\") == format!(\"{{e:?}}\") && tr(&ok).ok() == Some(max(&ok)) {{ println!(\"TRY_OK {{ver}}\"); }}\n\
             else {{ println!(\"TRY_FAIL {{ver}} {{t:?}} {{e:?}}\"); }}\n\
         }}\n\
         impl From<(i64, i32)> for Inner8 {{ fn from(p: (i64, i32)) -> Self {{ Inner8 {{ x: p.0, y: p.1 }} }} }}\n\
         const CYCLONE_TO: &[(&str, &str)] = &[\n{tos}];\n\
         /// Cyclone's object of type `name`: the one holding its name as a
         /// CDR string (length with the NUL, the bytes, the NUL).
         fn cyclone(name: &str) -> Option<(&'static str, &'static str)> {{\n\
             let mut marker = hex(&((name.len() + 1) as u32).to_le_bytes());\n\
             marker.push_str(&hex(name.as_bytes()));\n\
             marker.push_str(\"00\");\n\
             CYCLONE_TO.iter().copied().find(|(_, obj)| obj[10..].find(&marker).is_some_and(|i| i < 64))\n\
         }}\n\
         fn to_object(name: &str, ours: hdds::xtypes::CompleteTypeObject) {{\n\
             let h = ours.compute_equivalence_hash().expect(\"hash\");\n\
             let ours_hex = hex(h.as_bytes());\n\
             match cyclone(name) {{\n\
                 Some((ch, _)) if ch == ours_hex => println!(\"TO_OK {{name}}\"),\n\
                 Some((ch, obj)) => {{\n\
                     let bytes = unhex(obj);\n\
                     let mut off = 5usize;\n\
                     let c = hdds::xtypes::CompleteTypeObject::decode_cdr2_le_at(&bytes, &mut off);\n\
                     // A union whose only difference is Cyclone numbering\n\
                     // the cases from 0 (we number them from 1).\n\
                     let case_ids_only = match (&c, &ours) {{\n\
                         (Ok(hdds::xtypes::CompleteTypeObject::Union(cu)), hdds::xtypes::CompleteTypeObject::Union(_)) => {{\n\
                             let mut shifted = cu.clone();\n\
                             for m in &mut shifted.member_seq {{ m.common.member_id += 1; }}\n\
                             hdds::xtypes::CompleteTypeObject::Union(shifted) == ours\n\
                         }}\n\
                         _ => false,\n\
                     }};\n\
                     println!(\"TO_DIFF {{name}} ours {{ours_hex}} cyclone {{ch}} same_decoded={{}} case_ids_only={{case_ids_only}}\\n  ours    {{ours:?}}\\n  cyclone {{c:?}}\", c.as_ref().map(|c| *c == ours).unwrap_or(false));\n\
                 }}\n\
                 None => println!(\"TO_FAIL {{name}}: no Cyclone object\"),\n\
             }}\n\
         }}\n\
         fn tc(name: &str, o: hdds::xtypes::CompleteTypeObject, want: &[u16]) {{\n\
             let got: Vec<u16> = match &o {{\n\
                 hdds::xtypes::CompleteTypeObject::Struct(s) => s.member_seq.iter().map(|m| m.common.member_flags.0 & 3).collect(),\n\
                 hdds::xtypes::CompleteTypeObject::Union(u) => u.member_seq.iter().map(|m| m.common.member_flags.0 & 3).collect(),\n\
                 _ => Vec::new(),\n\
             }};\n\
             if got == want {{ println!(\"TC_OK {{name}}\"); }} else {{ println!(\"TC_FAIL {{name}} {{got:?}}\"); }}\n\
         }}\n\
         fn to<T: DDS>(name: &str) {{\n\
             to_object(name, T::get_type_object().expect(\"type object\"));\n\
         }}\n\
         fn main() {{\n\
         {body}\
         }}\n"
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
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test xtypes_datapath_rust -- --ignored"]
fn rust_backend_matches_cyclone_on_the_corpus() {
    let hdds = hdds_dir();
    let deps = format!(
        "hdds = {{ path = \"{}\" }}\n",
        hdds.join("crates/hdds").display()
    );
    let out = run_crate(
        "dp_rust",
        &deps,
        &[
            (
                "generated.rs",
                generate(&format!(
                    "{CORPUS}{EVOLUTION_IDL}{PLPROBE_IDL}{SPEC_UNION_IDL}{SPEC_UNION_HASHED_IDL}{FASTCDR_MAPS_IDL}{TRY_CONSTRUCT_IDL}{}",
                    typeobject_extra_idl()
                )),
            ),
            ("main.rs", main_rs()),
        ],
    );
    let fails = out.lines().filter(|l| l.contains("_FAIL")).count();
    let count = |p: &str| out.lines().filter(|l| l.starts_with(p)).count();
    let expected_codec = SAMPLES.len() * 2;
    assert!(
        fails == 0
            && count("ENC_OK")
                == expected_codec
                    + 1
                    + (SPEC_UNION_SAMPLES.len() + SPEC_UNION_HASHED_SAMPLES.len()) * 2
                    + 2
            && count("SHORT_OK")
                == expected_codec
                    + 1
                    + (SPEC_UNION_SAMPLES.len() + SPEC_UNION_HASHED_SAMPLES.len()) * 2
                    + 2
            && count("DEC_OK")
                == expected_codec
                    + DECODE_ONLY.len() * 2
                    + spec_xcdr1().len()
                    + 1
                    + (SPEC_UNION_SAMPLES.len() + SPEC_UNION_HASHED_SAMPLES.len()) * 4
                    + 4
            && count("TO_OK") == TYPEOBJECT_EXACT.len()
            && count("EVO_OK")
                == EVOLUTION.len() + PLPROBE.lines().filter(|l| !l.starts_with('#')).count()
            && count("AT_OK") == 2
            && count("TRY_OK") == 2
            && count("BOUND_OK") == 2
            && count("NESTED_OK") == 1
            && count("TC_OK") == TRY_CONSTRUCT.len(),
        "{out}"
    );
    for (ty, why) in TYPEOBJECT_CYCLONE_LIMITED {
        let line = out
            .lines()
            .find(|l| l.starts_with(&format!("TO_DIFF {ty} ")))
            .unwrap_or_else(|| {
                panic!("{ty} ({why}) no longer differs: move it to TYPEOBJECT_EXACT\n{out}")
            });
        // Cyclone's object decodes, and differs from ours (its case ids).
        assert!(line.contains("same_decoded=false"), "{line}");
        // For the unions themselves, the case ids are the ONLY difference:
        // Cyclone's object with each case id + 1 equals ours, byte for
        // byte once encoded (the holders differ by the union's hash).
        if why.starts_with("union case ids") {
            assert!(line.contains("case_ids_only=true"), "{line}");
        }
    }
}

/// Collections whose announced count would make the decoder reserve or
/// fill far more memory than the input, or loop without consuming input:
/// elements that take 1 byte on the wire but a 64 KiB value in memory
/// (an absent `@optional` array, a union of an array) and elements that
/// take no byte at all (an empty structure, an array of them).
const HOSTILE_IDL: &str = "
@final struct EmptyR {};
@final struct OptMidR { @optional long v[16]; };
@final struct OptBigR { @optional long a[16384]; };
@final union UBigR switch (long) { case 1: long a[16384]; };
@final struct HOM { sequence<OptMidR> q; };
@final struct HOB { sequence<OptBigR> q; };
@final struct HUB { sequence<UBigR> q; };
@final struct HE { sequence<EmptyR> q; };
@final struct HEE { sequence<sequence<EmptyR>> q; };
@final struct WrapR { EmptyR e[1048576]; };
@final struct HW { sequence<WrapR> q; };
@final struct E1 { EmptyR a; EmptyR b; };
@final struct E2 { E1 a; E1 b; };
@final struct E3 { E2 a; E2 b; };
@final struct E4 { E3 a; E3 b; };
@final struct E5 { E4 a; E4 b; };
@final struct E6 { E5 a; E5 b; };
@final struct E7 { E6 a; E6 b; };
@final struct E8 { E7 a; E7 b; };
@final struct E9 { E8 a; E8 b; };
@final struct E10 { E9 a; E9 b; };
@final struct E11 { E10 a; E10 b; };
@final struct E12 { E11 a; E11 b; };
@final struct HQQE12 { sequence<sequence<E12>> q; };
@final struct HOBB { sequence<sequence<OptBigR>> q; };
@final struct OptLL { @optional long long a; };
@final struct HOLL { sequence<OptLL> q; };
@final struct HMAP { sequence<map<long, long>> q; };
@appendable struct AppOld { long x; };
@appendable struct AppNew { long x; long a[9]; };
@final struct HAPP { sequence<AppNew> q; };
@final struct HAPPOLD { sequence<AppOld> q; };
@final struct S1 { long v; };
@final struct S2 { S1 s; };
@final struct S3 { S2 s; };
@final struct S4 { S3 s; };
@final struct S5 { S4 s; };
@final struct S6 { S5 s; };
@final struct S7 { S6 s; };
@final struct S8 { S7 s; };
@final struct S9 { S8 s; };
@final struct S10 { S9 s; };
@final struct S11 { S10 s; };
@final struct S12 { S11 s; };
@final struct S13 { S12 s; };
@final struct S14 { S13 s; };
@final struct S15 { S14 s; };
@final struct S16 { S15 s; };
@final struct S17 { S16 s; };
@final struct S18 { S17 s; };
@final struct S19 { S18 s; };
@final struct S20 { S19 s; };
@final struct S21 { S20 s; };
@final struct S22 { S21 s; };
@final struct S23 { S22 s; };
@final struct S24 { S23 s; };
@final struct S25 { S24 s; };
@final struct S26 { S25 s; };
@final struct S27 { S26 s; };
@final struct S28 { S27 s; };
@final struct S29 { S28 s; };
@final struct S30 { S29 s; };
@final struct S31 { S30 s; };
@final struct S32 { S31 s; };
@final struct S33 { S32 s; };
@final struct S34 { S33 s; };
@final struct HS34 { sequence<S34> q; };
";

/// The hostile checks: each decode under a counting allocator whose
/// largest single request must stay within `MAX_ALLOC_RATIO` (64) times
/// the body plus 64 KiB (the temporary of one `long[16384]` value), and
/// within a time bound.
const HOSTILE_MAIN: &str = r#"#![allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, clippy::all)]
mod generated { include!("generated.rs"); }
use generated::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Counting;
static MAX_REQ: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn grow(n: usize) { let l = LIVE.fetch_add(n, Ordering::Relaxed) + n; PEAK.fetch_max(l, Ordering::Relaxed); }
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { MAX_REQ.fetch_max(l.size(), Ordering::Relaxed); grow(l.size()); System.alloc(l) }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { LIVE.fetch_sub(l.size(), Ordering::Relaxed); System.dealloc(p, l) }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        MAX_REQ.fetch_max(n, Ordering::Relaxed);
        LIVE.fetch_sub(l.size(), Ordering::Relaxed); grow(n);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static A: Counting = Counting;
fn le(v: &[u32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }
fn check<T>(label: &str, body: &[u8], want_ok: bool, dec: impl Fn(&[u8]) -> Result<(T, usize), hdds::CdrError>) {
    MAX_REQ.store(0, Ordering::Relaxed);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let t = std::time::Instant::now();
    let r = dec(body);
    let el = t.elapsed();
    let ok = r.is_ok();
    drop(r);
    let req = MAX_REQ.load(Ordering::Relaxed);
    let peak = PEAK.load(Ordering::Relaxed) - base;
    let limit = 64 * body.len() + 65536 + 4096;
    if ok == want_ok && req <= limit && peak <= limit && el.as_secs() < 5 { println!("HOSTILE_OK {label}"); }
    else { println!("HOSTILE_FAIL {label} ok={ok} want={want_ok} max_request={req} peak={peak} limit={limit} elapsed={el:?}"); }
}
fn main() {
    let mut b = vec![0u8; 1 << 20];
    let n = HOM { q: vec![OptMidR { v: None }] }.encode_xcdr1_le(&mut b).unwrap();
    check("OptMid one absent", &b[..n], true, HOM::decode_xcdr1_le);
    let hob = HOB { q: (0..16).map(|_| OptBigR { a: None }).collect() };
    let n = hob.encode_xcdr1_le(&mut b).unwrap();
    check("OptBig 16 absent", &b[..n], false, HOB::decode_xcdr1_le);
    let mut hub = le(&[0xFFFF, 1]);
    hub.extend(le(&[7; 16384]));
    check("UBig count 65535", &hub, false, HUB::decode_xcdr1_le);
    check("Empty 3", &le(&[3]), true, HE::decode_xcdr1_le);
    check("Empty 1025", &le(&[1025]), false, HE::decode_xcdr1_le);
    check("Empty 10M", &le(&[10_000_000]), false, HE::decode_xcdr1_le);
    check("Empty nested inner 0xFFFFFFFF", &le(&[1, 0xFFFF_FFFF]), false, HEE::decode_xcdr1_le);
    let mut nested = le(&[32]); nested.extend(le(&[32; 32]));
    check("Empty nested 32 x 32 (1024 values)", &nested, true, HEE::decode_xcdr1_le);
    let mut nested = le(&[1024]); nested.extend(le(&[1024; 1024]));
    check("Empty nested 1024 x 1024 (per decode)", &nested, false, HEE::decode_xcdr1_le);
    let mut nested = le(&[8192]); nested.extend(le(&[100; 8192]));
    check("Empty nested 8192 x 100 (per decode)", &nested, false, HEE::decode_xcdr1_le);
    check("Wrap of 1M empties x1", &le(&[1]), true, HW::decode_xcdr1_le);
    check("Wrap of 1M empties x2", &le(&[2]), false, HW::decode_xcdr1_le);
    // The verifier's body: XCDR2, outer DHEADER, 8192 inner sequences of
    // DHEADER 4 and count 100 E12 (8191 values each), 65,544 bytes.
    let mut q = le(&[4 + 8192 * 8, 8192]);
    for _ in 0..8192 { q.extend(le(&[4, 100])); }
    assert_eq!(q.len(), 65_544);
    check("E12 nested 8192 x 100", &q, false, HQQE12::decode_xcdr2_le);
    // 8192 inner sequences of one absent OptBig (64 KiB each in memory).
    let n1 = HOB { q: vec![OptBigR { a: None }] }.encode_xcdr1_le(&mut b).unwrap();
    let one = b[..n1].to_vec();
    let mut q = le(&[8192]);
    for _ in 0..8192 { q.extend(&one); }
    check("OptBig nested 8192 x 1 absent (per decode)", &q, false, HOBB::decode_xcdr1_le);
    // Ordinary data that must decode.
    for k in [1usize, 100] {
        let n = HOLL { q: (0..k).map(|_| OptLL { a: None }).collect() }.encode_xcdr2_le(&mut b).unwrap();
        check(&format!("{k} absent optional long long"), &b[..n], true, HOLL::decode_xcdr2_le);
    }
    let n = HMAP { q: (0..100).map(|_| std::collections::HashMap::new()).collect() }.encode_xcdr2_le(&mut b).unwrap();
    check("100 empty maps", &b[..n], true, HMAP::decode_xcdr2_le);
    let n = HAPPOLD { q: (0..100).map(|i| AppOld { x: i }).collect() }.encode_xcdr2_le(&mut b).unwrap();
    check("100 APPENDABLE elements from an older writer", &b[..n], true, HAPP::decode_xcdr2_le);
    let mut deep = le(&[1025]); deep.extend(le(&[7; 1025]));
    check("1025 present S34", &deep, true, HS34::decode_xcdr1_le);
}
"#;

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test xtypes_datapath_rust -- --ignored"]
fn rust_decoder_bounds_hostile_counts() {
    let hdds = hdds_dir();
    let deps = format!(
        "hdds = {{ path = \"{}\" }}\n",
        hdds.join("crates/hdds").display()
    );
    let out = run_crate(
        "dp_rust_hostile",
        &deps,
        &[
            ("generated.rs", generate(HOSTILE_IDL)),
            ("main.rs", HOSTILE_MAIN.to_string()),
        ],
    );
    let ok = out.lines().filter(|l| l.starts_with("HOSTILE_OK")).count();
    assert!(ok == 19 && !out.contains("HOSTILE_FAIL"), "{out}");
}
