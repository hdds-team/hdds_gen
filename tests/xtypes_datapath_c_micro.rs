// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated C-micro code against a foreign oracle.
//!
//! `fixtures/xtypes_datapath/corpus.idl` is compiled by hddsgen's C-micro
//! backend and built against `sdk/c-micro/include`; every sample
//! `cyclone_probe.c` serializes is built in C and
//!
//! 1. encoded: the bytes must be Cyclone DDS 0.11's XCDR2 body
//!    (`cyclone_vectors.txt`), at most `<TYPE>_ENCODED_SIZE_MAX`, and a
//!    cursor one byte short must be refused;
//! 2. decoded from Cyclone's body (into a value pre-filled with 0xA5): every
//!    byte must be consumed and the decoded value must encode to Cyclone's
//!    body again (the encoder being injective on these values and equal to
//!    Cyclone on the sample, the decoded value is the sample).
//!
//! Every corpus type is expressible: no sample is excluded. The program
//! is compiled with `-std=c99 -pedantic-errors` (the SDK header uses
//! `inline`). A type this backend cannot hold (a map, a bitset, `fixed`, a
//! `@mutable` union) is refused by the generator, checked below. Skipped,
//! with a message, without a C compiler.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::{Backend, Parser};

const CORPUS: &str = include_str!("fixtures/xtypes_datapath/corpus.idl");
const VECTORS: &str = include_str!("fixtures/xtypes_datapath/cyclone_vectors.txt");
/// The @mutable union of `spec_mutable_union.txt`, added to the corpus.
const UNION_IDL: &str = "@mutable union UM switch (long) { case 1: long a; case 2: string s; case 3: double d; };\n@final struct HoldUM { octet p; UM u; long z; };\n";
const UNION: &str = include_str!("fixtures/xtypes_datapath/spec_mutable_union.txt");
/// The @mutable @autoid(HASH) union of `spec_mutable_union_hashed.txt`
/// (case ids by MD5 name hash), added to the corpus.
const UNION_H_IDL: &str = "@mutable @autoid(HASH) union UMh switch (long) { case 1: long color; case 2: short shapesize; };\n@final struct HoldUMh { octet p; UMh u; };\n";
const UNION_H: &str = include_str!("fixtures/xtypes_datapath/spec_mutable_union_hashed.txt");
/// Union labels of every kind the parser accepts (TRUE / FALSE, a
/// character, a negative integer, scoped enumerators `m::A`, `::m::B`),
/// read through `data_model::union_label_value`.
const LABELS_IDL: &str = "module lm { enum LE { LA, LB, LC }; };\n\
@final union UBool switch (boolean) { case TRUE: long t; case FALSE: short f; };\n\
@final union UChar switch (char) { case 'c': long c; case 'd': octet d; };\n\
@final union UNeg switch (long) { case -1: long n; case 2: octet p; };\n\
@final union UScoped switch (lm::LE) { case lm::LA: long a; case ::lm::LB: octet b; };\n\
@final struct HoldLabels { UBool b; UChar c; UNeg n; UScoped s; };\n";
/// Bodies of `HoldLabels`, written by hand from the specification (rule
/// 26, spec text line 6974: the discriminator at its type's width, then
/// the selected member; alignment min(size, 4) from the start of the
/// body): `Labels_1` b = TRUE / t = 0x11, c = 'c' / 0x22, n = -1 / 0x33,
/// s = LB / 0x44; `Labels_2` b = FALSE / f = 0x0102, c = 'd' / 0x55,
/// n = 2 / 0x66, s = LA / 0x77.
const SPEC_VECTORS: &[(&str, &str)] = &[
    (
        "Labels_1",
        "01000000110000006300000022000000ffffffff330000000100000044",
    ),
    (
        "Labels_2",
        "000002016455000002000000660000000000000077000000",
    ),
];

/// (`alt=` body, `spec=` body) of an xcdr2 line of `spec_mutable_union.txt`,
/// without the encapsulation header.
fn alt_spec(sample: &str) -> Option<(String, String)> {
    UNION
        .lines()
        .chain(UNION_H.lines())
        .filter(|l| !l.starts_with('#'))
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == sample && it.next()? == "xcdr2").then_some(())?;
            let r = it.next()?.strip_prefix("alt=")?.get(8..)?.to_string();
            let s = it.next()?.strip_prefix("spec=")?.get(8..)?.to_string();
            Some((r, s))
        })
}

/// (sample, C type, statements filling `v`). The values are those
/// `cyclone_probe.c` writes.
/// Samples of `spec_mutable_union.txt` whose `alt=` body is also decoded.
const ALT_READS: usize = 5;

const SAMPLES: &[(&str, &str, &str)] = &[
    ("FinalPrims", "FinalPrims", "v.a = 0x11; v.b = I64(0x01020304, 0x05060708); v.c = -2; v.d = 1.5; v.e = 1; v.f = 0xAABBCCDDu;"),
    ("NestFinal", "NestFinal", "v.a = 1; v.in8.x = I64(0x11223344, 0x55667788); v.in8.y = 7; v.z = 9;"),
    ("NestOdd", "NestOdd", "v.a = 1; v.il.x = 0x01020304; v.s = 5;"),
    ("NestApp", "NestApp", "v.a = 1; v.app.a = 2; strcpy(v.app.s, \"hi\"); v.app.b = 3; v.z = 4;"),
    ("MutDefault", "MutDefault", "v.a = 1; strcpy(v.s, \"abc\"); v.d = 2.0; v.o = 7;"),
    ("MutIds", "MutIds", "v.a = 1; v.b = 2; v.c = 3; v.d = 4;"),
    ("MutHash", "MutHash", "v.a = 1; v.b = 2; v.c = 3; v.d = 4;"),
    ("MutHashid", "MutHashid", "v.x = 1; v.y = 2; v.z = 3;"),
    ("MutPrims", "MutPrims", "v.x = 1; v.y = 2; v.z = 3;"),
    ("MutCompact2", "MutCompact2", "v.a = 1; v.b = 2; v.c = 3;"),
    ("NestMut", "NestMut", "v.p = 1; v.m.x = 1; v.m.y = 2; v.m.z = 3; v.q = 2;"),
    ("MutOpt_all", "MutOpt", "v.has_a = 1; v.a = 5; v.has_b = 1; strcpy(v.b, \"x\"); v.c = 6;"),
    ("MutOpt_none", "MutOpt", "v.has_a = 0; v.has_b = 0; v.c = 6;"),
    ("DerivedF", "DerivedF", "v.base.id = 1; v.base.v = 2.5; v.s = 3; v.w = 4;"),
    ("DerivedA", "DerivedA", "v.base.id = 1; strcpy(v.name, \"n\");"),
    ("DerivedM", "DerivedM", "v.base.id = 1; strcpy(v.base.tag, \"t\"); v.extra = 2; v.s = 3; v.after = 4;"),
    ("Enums", "Enums", "v.a = E8_E8_B; v.b = E16_E16_B; v.c = E32_E32_C; v.x = 0xEE; v.d = E16_E16_A; v.e = E8_E8_C;"),
    ("MutEnums", "MutEnums", "v.a = E8_E8_C; v.b = E16_E16_B; v.c = E32_E32_B;"),
    ("AppEnums", "AppEnums", "v.p = 1; v.b = E16_E16_B;"),
    ("Seqs", "Seqs", "v.p = 1; v.u.data[0] = 1; v.u.data[1] = 2; v.u.count = 2; v.bd.data[0] = 0.5; v.bd.count = 1; v.q = 2; v.s.data[0] = 3; v.s.data[1] = 4; v.s.data[2] = 5; v.s.count = 3; v.bl.data[0] = 6; v.bl.count = 1;"),
    ("SeqStruct", "SeqStruct", "v.p = 1; v.v.data[0].x = 1; v.v.data[0].y = 2; v.v.data[1].x = 3; v.v.data[1].y = 4; v.v.count = 2;"),
    ("SeqApp", "SeqApp", "v.p = 1; v.v.data[0].a = 1; strcpy(v.v.data[0].s, \"a\"); v.v.data[0].b = 2; v.v.data[1].a = 3; strcpy(v.v.data[1].s, \"\"); v.v.data[1].b = 4; v.v.count = 2;"),
    ("SeqStr", "SeqStr", "v.p = 1; strcpy(v.v.data[0], \"ab\"); strcpy(v.v.data[1], \"c\"); v.v.count = 2; strcpy(v.b.data[0], \"wxyz\"); v.b.count = 1;"),
    ("Arrs", "Arrs", "v.p = 1; v.a[0] = 1; v.a[1] = 2; v.a[2] = 3; v.m[0][0] = 1; v.m[0][1] = 2; v.m[0][2] = 3; v.m[1][0] = 4; v.m[1][1] = 5; v.m[1][2] = 6; v.s[0][0] = 1; v.s[0][1] = 2; v.s[1][0] = 3; v.s[1][1] = 4;"),
    ("ArrStruct", "ArrStruct", "v.p = 1; v.a[0].x = 1; v.a[0].y = 2; v.a[1].x = 3; v.a[1].y = 4;"),
    ("HoldU_1", "HoldU", "v.p = 1; v.u._d = 1; v.u._u.a = 0x0A; v.z = 9;"),
    ("HoldU_2", "HoldU", "v.p = 1; v.u._d = 2; strcpy(v.u._u.s, \"hello\"); v.z = 9;"),
    ("HoldU_3", "HoldU", "v.p = 1; v.u._d = 3; v.u._u.d = 0.25; v.z = 9;"),
    ("HoldU_def", "HoldU", "v.p = 1; v.u._d = 9; v.u._u.o = 0x7F; v.z = 9;"),
    ("HoldU_def0", "HoldU", "v.p = 1; v.u._d = 0; v.u._u.o = 0x7F; v.z = 9;"),
    ("HoldUA_1", "HoldUA", "v.p = 1; v.u._d = 1; v.u._u.a = 5;"),
    ("HoldUA_2", "HoldUA", "v.p = 1; v.u._d = 2; strcpy(v.u._u.s, \"ab\");"),
    ("HoldUE_A", "HoldUE", "v.p = 1; v.u._d = E8_E8_A; v.u._u.a = 6; v.z = 2;"),
    ("HoldUE_B", "HoldUE", "v.p = 1; v.u._d = E8_E8_B; v.u._u.b = 7; v.z = 2;"),
    ("HoldUS_1", "HoldUS", "v.p = 1; v.u._d = 1; v.u._u.a = 0x11; v.q = 2;"),
    ("HoldUS_2", "HoldUS", "v.p = 1; v.u._d = 2; v.u._u.b = 0x2233; v.q = 2;"),
    ("HoldUS_3", "HoldUS", "v.p = 1; v.u._d = 3; v.u._u.c = 0x44556677; v.q = 2;"),
    ("BStr", "BStr", "v.p = 1; strcpy(v.s, \"abcdefgh\"); v.z = 2;"),
    ("Keyed", "Keyed", "v.id = 1; strcpy(v.name, \"nm\"); v.v = 3;"),
    ("KeyedApp", "KeyedApp", "v.id = 1; v.v = 2;"),
    ("FinalOpt_all", "FinalOpt", "v.t = 1; v.has_a = 1; v.a = 5; v.has_b = 1; v.b = 6; v.u = 2;"),
    ("FinalOpt_none", "FinalOpt", "v.t = 1; v.has_a = 0; v.has_b = 0; v.u = 2;"),
    ("AppOpt_all", "AppOpt", "v.t = 1; v.has_a = 1; v.a = 7;"),
    ("AppOpt_none", "AppOpt", "v.t = 1; v.has_a = 0;"),
    ("MutMix", "MutMix", "v.sl.data[0] = 1; v.sl.data[1] = 2; v.sl.count = 2; strcpy(v.ss.data[0], \"a\"); v.ss.count = 1; v.fin.x = 3; v.fin.y = 4; v.app.a = 5; strcpy(v.app.s, \"b\"); v.app.b = 6; v.mut.x = 7; v.mut.y = 8; v.mut.z = 9; v.un._d = 2; strcpy(v.un._u.s, \"u\"); v.arr[0] = 10; v.arr[1] = 11; strcpy(v.bs, \"xy\"); v.en = E16_E16_B; v.se.data[0] = E8_E8_B; v.se.data[1] = E8_E8_C; v.se.count = 2;"),
    ("SeqEnum", "SeqEnum", "v.p = 1; v.a.data[0] = E8_E8_C; v.a.data[1] = E8_E8_A; v.a.data[2] = E8_E8_B; v.a.count = 3; v.b.data[0] = E32_E32_B; v.b.count = 1; v.arr[0] = E8_E8_B; v.arr[1] = E8_E8_C; v.arr[2] = E8_E8_A;"),
    ("WStr", "WStr", "v.p = 1; v.c = 0x41; v.s[0] = 0x68; v.s[1] = 0xE9; v.s[2] = 0; v.d = 0x263A;"),
    ("Aliases", "Aliases", "v.p = 1; v.m = BM32_BM32_0 | BM32_BM32_5; v.n = BM8_BM8_1; v.l.data[0] = 7; v.l.data[1] = 8; v.l.count = 2; v.ia.x = 9; v.ia.y = 10;"),
    ("ArrStr", "ArrStr", "v.p = 1; strcpy(v.a[0], \"ab\"); strcpy(v.a[1], \"\"); strcpy(v.b[0], \"xyz\"); strcpy(v.b[1], \"q\");"),
    ("MutLC", "MutLC", "v.s1.data[0] = 1; v.s1.data[1] = 2; v.s1.data[2] = 3; v.s1.count = 3; v.s2.data[0] = 4; v.s2.count = 1; v.s4.data[0] = 5; v.s4.data[1] = 6; v.s4.count = 2; v.s8.data[0] = 7; v.s8.count = 1; v.sd.data[0] = 0.5; v.sd.data[1] = 1.0; v.sd.count = 2; v.sb.data[0] = 1; v.sb.count = 1; v.a1[0] = 1; v.a1[1] = 2; v.a1[2] = 3; v.a2[0] = 4; v.a2[1] = 5; v.a4[0] = 6; v.a4[1] = 7; strcpy(v.s, \"st\"); v.si.data[0].x = 1; v.si.data[0].y = 2; v.si.count = 1; v.e = E32_E32_C; v.m = BM32_BM32_1; v.has_o = 0; v.a8[0] = 8; v.b = 1; v.s0.count = 0;"),
    ("MutLC2", "MutLC2", "v.ai[0].x = 1; v.ai[0].y = 2; v.ae[0] = E8_E8_B; v.ae[1] = E8_E8_C; strcpy(v.sa[0], \"s\"); v.bm = BM8_BM8_0; v.sbm.data[0] = BM8_BM8_1; v.sbm.count = 1; v.ls.data[0] = 3; v.ls.count = 1; v.ia.x = 4; v.ia.y = 5;"),
    ("SeqChar", "SeqChar", "v.p = 1; v.c.data[0] = 'x'; v.c.data[1] = 'y'; v.c.count = 2; v.w.data[0] = 0x41; v.w.data[1] = 0x42; v.w.count = 2; strcpy(v.s, \"ab\");"),
    ("MutSeqChar", "MutSeqChar", "v.c.data[0] = 'q'; v.c.count = 1; v.w.data[0] = 0x5A; v.w.count = 1; v.z = 5;"),
    ("HoldUMh_1", "HoldUMh", "v.p = 1; v.u._d = 1; v.u._u.color = 0x11;"),
    ("HoldUMh_2", "HoldUMh", "v.p = 1; v.u._d = 2; v.u._u.shapesize = 0x22;"),
    ("Labels_1", "HoldLabels", "v.b._d = 1; v.b._u.t = 0x11; v.c._d = 'c'; v.c._u.c = 0x22; v.n._d = -1; v.n._u.n = 0x33; v.s._d = LM_LE_LB; v.s._u.b = 0x44;"),
    ("Labels_2", "HoldLabels", "v.b._d = 0; v.b._u.f = 0x0102; v.c._d = 'd'; v.c._u.d = 0x55; v.n._d = 2; v.n._u.p = 0x66; v.s._d = LM_LE_LA; v.s._u.a = 0x77;"),
    ("HoldUM_1", "HoldUM", "v.p = 1; v.u._d = 1; v.u._u.a = 10; v.z = 9;"),
    ("HoldUM_2", "HoldUM", "v.p = 1; v.u._d = 2; strcpy(v.u._u.s, \"hello\"); v.z = 9;"),
    ("HoldUM_3", "HoldUM", "v.p = 1; v.u._d = 3; v.u._u.d = 0.25; v.z = 9;"),
];

/// Bodies written by hand from the specification (never by an hddsgen
/// encoder): (label, C type, input hex, the body the decoded value must
/// encode to, or "" when the decoder must refuse the input). See
/// `tests/xtypes_datapath_c.rs` for the derivation of each.
const EXTRA_DECODE: &[(&str, &str, &str, &str)] = &[
    (
        "NestApp_appended",
        "NestApp",
        "0100000010000000020000000300000068690003eeeeeeee04000000",
        "010000000c00000002000000030000006869000304000000",
    ),
    (
        "MutPrims_unknown_ok",
        "MutPrims",
        "200000000000002001000000010000200200000002000020030000000900002063000000",
        "18000000000000200100000001000020020000000200002003000000",
    ),
    (
        "MutPrims_unknown_mu",
        "MutPrims",
        "20000000000000200100000001000020020000000200002003000000090000a063000000",
        "",
    ),
    (
        "MutPrims_missing",
        "MutPrims",
        "1000000000000020010000000100002002000000",
        "18000000000000200100000001000020020000000200002000000000",
    ),
    (
        "FinalPrims_bool2",
        "FinalPrims",
        "110000000807060504030201feff0000000000000000f83f02000000ddccbbaa",
        "",
    ),
    ("AppEnums_badlit", "AppEnums", "0400000001000700", ""),
    (
        "BStr_overbound",
        "BStr",
        "010000000a00000061626364656667686900000002000000",
        "",
    ),
    // A sequence over its bound (Seqs.bd, sequence<double, 5>, with 6
    // elements, the body otherwise complete) is refused.
    ("Seqs_overbound", "Seqs",
     "010000000000000006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000",
     ""),
    // A string without its NUL (BStr.s holding "abcdefgh", length 8).
    ("BStr_nonul", "BStr", "0100000008000000616263646566676802000000", ""),
    // HoldUM: the case member before the discriminator.
    ("HoldUM_case_first", "HoldUM", "0100000010000000010000200a000000000000a00100000009000000", "0100000010000000000000a001000000010000200a00000009000000"),
    // HoldUM: an unknown member without the M flag is skipped.
    ("HoldUM_unknown_ok", "HoldUM", "0100000018000000000000a001000000010000200a000000090000206300000009000000", "0100000010000000000000a001000000010000200a00000009000000"),
    // HoldUM: the same member with the M flag is refused.
    ("HoldUM_unknown_mu", "HoldUM", "0100000018000000000000a001000000010000200a000000090000a06300000009000000", ""),
    // HoldUM: no member at all (no discriminator, no case member).
    ("HoldUM_empty", "HoldUM", "010000000000000009000000", ""),
    // HoldUM: no discriminator member.
    ("HoldUM_no_disc", "HoldUM", "0100000008000000010000200a00000009000000", ""),
    // HoldUM: discriminator 2 with the member of case 1.
    ("HoldUM_wrong_case", "HoldUM", "0100000010000000000000a002000000010000200a00000009000000", ""),
    // HoldUM: the case member twice.
    ("HoldUM_two_cases", "HoldUM", "0100000018000000000000a001000000010000200a000000010000200b00000009000000", ""),
    // HoldUM: a discriminator no label selects, no case member.
    ("HoldUM_none", "HoldUM", "0100000008000000000000a00700000009000000", "0100000008000000000000a00700000009000000"),
    // HoldUM: a discriminator no label selects, with a case member.
    ("HoldUM_none_case", "HoldUM", "0100000010000000000000a007000000010000200a00000009000000", ""),
];

/// Reader types with more members than the writer types of the corpus
/// (`MutW`, `AppW`, `NestAppW`, Cyclone DDS 0.11 bodies): the members the
/// writer's type lacks take their default (DDS-XTypes 1.3 7.4.1.2.3, spec
/// text lines 6195-6203; 3390-3393; Table 9, lines 2311-2349: zero, "",
/// an empty sequence, the first enumeration literal (`EF_X` = 5, not 0), a
/// union at its default discriminator and the member it selects, nested
/// structures at their default; an optional member absent).
const EVOLUTION_IDL: &str = "enum EFirst { EF_X = 5, EF_Y = 2 };\n\
@mutable struct MutWider { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) long added; };\n\
@appendable struct AppWider { long a; long b; long added; };\n\
@final struct NestAppWider { octet p; AppWider w; long z; };\n\
@mutable struct MutRich { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) EFirst e; @id(5) string t; @id(6) Inner8 in8; @id(7) sequence<long> q; @id(8) UL u; @id(9) string<4> bs; @id(10) long arr[2]; @id(11) @optional long o; @id(12) wstring w; @id(13) AppOpt ao; };\n\
@appendable struct AppRich { long a; long b; EFirst e; string t; UL u; @optional long o; AppStruct app; };\n";

/// (writer sample, reader type, condition on the decoded value `d`).
const EVOLUTION: &[(&str, &str, &str)] = &[
    ("MutW", "MutWider", "d.a == 1 && d.b == 2 && strcmp(d.s, \"w\") == 0 && d.added == 0"),
    ("AppW", "AppWider", "d.a == 1 && d.b == 2 && d.added == 0"),
    ("NestAppW", "NestAppWider", "d.p == 1 && d.w.a == 3 && d.w.b == 4 && d.w.added == 0 && d.z == 5"),
    ("MutW", "MutRich", "d.a == 1 && d.b == 2 && strcmp(d.s, \"w\") == 0 && d.e == EFIRST_EF_X && d.t[0] == 0 && d.in8.x == 0 && d.in8.y == 0 && d.q.count == 0 && d.u._d == 0 && d.u._u.o == 0 && d.bs[0] == 0 && d.arr[0] == 0 && d.arr[1] == 0 && d.has_o == 0 && d.w[0] == 0 && d.ao.t == 0 && d.ao.has_a == 0"),
    ("AppW", "AppRich", "d.a == 1 && d.b == 2 && d.e == EFIRST_EF_X && d.t[0] == 0 && d.u._d == 0 && d.u._u.o == 0 && d.has_o == 0 && d.app.a == 0 && d.app.s[0] == 0 && d.app.b == 0"),
];

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn generate(idl: &str) -> Result<String, String> {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::CMicro
        .generator()
        .generate(&ast)
        .map_err(|e| e.to_string())
}

fn vector(sample: &str) -> String {
    VECTORS
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == sample && it.next()? == "xcdr2").then_some(())?;
            let _hdr = it.next()?;
            Some(it.next()?.strip_prefix("body=")?.to_string())
        })
        .or_else(|| alt_spec(sample).map(|(_, spec)| spec))
        .or_else(|| {
            SPEC_VECTORS
                .iter()
                .find(|(n, _)| *n == sample)
                .map(|(_, b)| (*b).to_string())
        })
        .unwrap_or_else(|| panic!("no xcdr2 vector for {sample}"))
}

/// The C-micro function prefix of a type name (`CamelCase` to `snake_case`).
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn program() -> String {
    let mut body = String::new();
    for (name, ty, fill) in SAMPLES {
        let f = snake(ty);
        let exp = vector(name);
        let max = ty.to_uppercase();
        let _ = writeln!(
            body,
            "    {{\n        {ty} v;\n        {ty} d;\n        int rc;\n        uint32_t bl;\n        memset(&v, 0, sizeof v);\n        {{ {fill} }}\n\
             \x20       hdds_cdr_init(&cdr, out, sizeof out);\n\
             \x20       rc = {f}_encode(&v, &cdr);\n\
             \x20       enc_check(\"{name}\", rc, cdr.pos, {max}_ENCODED_SIZE_MAX, \"{exp}\");\n\
             \x20       bl = (uint32_t)(strlen(\"{exp}\") / 2);\n\
             \x20       if (bl > 0) {{ hdds_cdr_init(&cdr, out, bl - 1); rc = {f}_encode(&v, &cdr); short_check(\"{name}\", rc); }}\n\
             \x20       bl = unhex(\"{exp}\", in);\n\
             \x20       memset(&d, 0xA5, sizeof d);\n\
             \x20       hdds_cdr_init(&cdr, in, bl);\n\
             \x20       rc = {f}_decode(&d, &cdr);\n\
             \x20       if (rc != 0 || cdr.pos != bl) {{ printf(\"DEC_FAIL {name} rc %d used %u of %u\\n\", rc, (unsigned)cdr.pos, (unsigned)bl); fails++; }}\n\
             \x20       else {{ hdds_cdr_init(&cdr, out, sizeof out); rc = {f}_encode(&d, &cdr); dec_check(\"{name}\", rc, cdr.pos, \"{exp}\"); }}\n    }}"
        );
    }
    let alt_reads: Vec<(String, &str, String, String)> = SAMPLES
        .iter()
        .filter_map(|(n, ty, _)| alt_spec(n).map(|(r, s)| (format!("{n}/alt"), *ty, r, s)))
        .collect();
    let extra = EXTRA_DECODE
        .iter()
        .map(|(n, ty, i, c)| ((*n).to_string(), *ty, (*i).to_string(), (*c).to_string()))
        .chain(alt_reads);
    for (name, ty, input, canon) in extra {
        let f = snake(ty);
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int rc;\n        uint32_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n\
             \x20       hdds_cdr_init(&cdr, in, bl);\n\
             \x20       rc = {f}_decode(&d, &cdr);\n\
             \x20       if (strlen(\"{canon}\") == 0) {{ if (rc != 0) {{ printf(\"XDEC_OK {name}\\n\"); }} else {{ printf(\"XDEC_FAIL {name} accepted\\n\"); fails++; }} }}\n\
             \x20       else if (rc != 0 || cdr.pos != bl) {{ printf(\"XDEC_FAIL {name} rc %d\\n\", rc); fails++; }}\n\
             \x20       else {{ hdds_cdr_init(&cdr, out, sizeof out); rc = {f}_encode(&d, &cdr); hexs(out, cdr.pos, got); if (rc == 0 && strcmp(got, \"{canon}\") == 0) {{ printf(\"XDEC_OK {name}\\n\"); }} else {{ printf(\"XDEC_FAIL {name}\\n  got %s\\n\", got); fails++; }} }}\n    }}"
        );
    }
    for (sample, ty, cond) in EVOLUTION {
        let f = snake(ty);
        let input = vector(sample);
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int rc;\n        uint32_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n\
             \x20       hdds_cdr_init(&cdr, in, bl);\n\
             \x20       rc = {f}_decode(&d, &cdr);\n\
             \x20       if (rc == 0 && cdr.pos == bl && ({cond})) {{ printf(\"EVO_OK {sample} as {ty}\\n\"); }} else {{ printf(\"EVO_FAIL {sample} as {ty} rc %d used %u of %u\\n\", rc, (unsigned)cdr.pos, (unsigned)bl); fails++; }}\n    }}"
        );
    }
    format!(
        "#include <stdio.h>\n#include \"corpus.h\"\n\
         #define I64(h, l) ((int64_t)(((uint64_t)(h) << 32) | (uint64_t)(l)))\n\
         static int fails = 0;\n\
         static uint8_t out[8192];\n\
         static uint8_t in[8192];\n\
         static char got[16400];\n\
         static hdds_cdr_t cdr;\n\
         static void hexs(const uint8_t* b, uint32_t n, char* s) {{ uint32_t i; s[0] = 0; for (i = 0; i < n; ++i) {{ sprintf(s + 2 * i, \"%02x\", b[i]); }} }}\n\
         static uint32_t unhex(const char* s, uint8_t* o) {{ uint32_t n = (uint32_t)(strlen(s) / 2); uint32_t i; unsigned int x; for (i = 0; i < n; ++i) {{ sscanf(s + 2 * i, \"%2x\", &x); o[i] = (uint8_t)x; }} return n; }}\n\
         static void enc_check(const char* name, int rc, uint32_t n, unsigned long max, const char* exp) {{\n\
         \x20   if (rc != 0) {{ printf(\"ENC_FAIL %s error %d\\n\", name, rc); fails++; return; }}\n\
         \x20   hexs(out, n, got);\n\
         \x20   if (strcmp(got, exp) == 0 && (unsigned long)n <= max) {{ printf(\"ENC_OK %s\\n\", name); }}\n\
         \x20   else {{ printf(\"ENC_FAIL %s max=%lu\\n  got %s\\n  exp %s\\n\", name, max, got, exp); fails++; }}\n\
         }}\n\
         static void short_check(const char* name, int rc) {{\n\
         \x20   if (rc == HDDS_CDR_ERR_OVERFLOW) {{ printf(\"SHORT_OK %s\\n\", name); }} else {{ printf(\"SHORT_FAIL %s %d\\n\", name, rc); fails++; }}\n\
         }}\n\
         static void dec_check(const char* name, int rc, uint32_t n, const char* exp) {{\n\
         \x20   if (rc != 0) {{ printf(\"DEC_FAIL %s re-encode error %d\\n\", name, rc); fails++; return; }}\n\
         \x20   hexs(out, n, got);\n\
         \x20   if (strcmp(got, exp) == 0) {{ printf(\"DEC_OK %s\\n\", name); }} else {{ printf(\"DEC_FAIL %s\\n  got %s\\n\", name, got); fails++; }}\n\
         }}\n\
         int main(void) {{\n{body}    return fails ? 1 : 0;\n}}\n"
    )
}

#[test]
fn c_micro_backend_matches_cyclone_on_the_corpus() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C-micro data path test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_c_micro");
    fs::create_dir_all(&dir).expect("dir");
    fs::write(
        dir.join("corpus.h"),
        generate(&format!(
            "{CORPUS}\n{UNION_IDL}\n{UNION_H_IDL}\n{LABELS_IDL}\n{EVOLUTION_IDL}"
        ))
        .expect("codegen"),
    )
    .expect("header");
    let src = dir.join("dp.c");
    fs::write(&src, program()).expect("program");
    let exe = dir.join("dp");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let out = Command::new(cc)
        .args([
            "-std=c99",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
        ])
        .arg(&include)
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success(), "{stdout}");
    let count = |p: &str| stdout.lines().filter(|l| l.starts_with(p)).count();
    assert!(
        !stdout.contains("_FAIL")
            && count("ENC_OK ") == SAMPLES.len()
            && count("SHORT_OK ") == SAMPLES.len()
            && count("DEC_OK ") == SAMPLES.len()
            && count("XDEC_OK ") == EXTRA_DECODE.len() + ALT_READS
            && count("EVO_OK ") == EVOLUTION.len(),
        "{stdout}"
    );
}

/// A type the C-micro holders cannot represent is refused by name, never
/// generated with a different serialization.
#[test]
fn c_micro_refuses_what_it_cannot_hold() {
    for (idl, needle) in [
        (
            "struct M { map<long, long> m; };",
            "struct `M` refused: a map",
        ),
        (
            "bitset B { bitfield<3> a; }; struct S { B b; };",
            "struct `S` refused: a bitset",
        ),
        (
            "struct F { fixed<5, 2> f; };",
            "struct `F` refused: `fixed`",
        ),
        (
            "@mutable union U switch (long) { case 1: @id(0) long a; };",
            "union `U` refused: case `a` has member id 0",
        ),
    ] {
        let err = generate(idl).expect_err(idl);
        assert!(err.contains(needle), "{idl}: {err}");
    }
}

/// The helpers of `sdk/c-micro/include/hdds_micro_cdr.h` follow XCDR2: an
/// 8-byte value aligned at 4 (DDS-XTypes 1.3 7.4.3.2.2, spec text lines
/// 6364-6367: MAXALIGN(VERSION2) = 4; rule 2, line 6762), zero padding.
/// Expected bytes written from the specification: octet 1 at 0, padding
/// to 4, u64 0x0102030405060708 at 4, f64 1.5 at 12, u16 7 at 20, padding
/// to 24, u64 9 at 24 (32 bytes), into a buffer pre-filled with 0xEE.
#[test]
fn c_micro_sdk_helpers_are_xcdr2() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C-micro SDK helper test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("c_micro_sdk_helpers");
    fs::create_dir_all(&dir).expect("dir");
    let src = dir.join("sdk.c");
    fs::write(
        &src,
        "#include <stdio.h>\n#include \"hdds_micro_cdr.h\"\n\
         static const char* EXP = \"010000000807060504030201000000000000f83f070000000900000000000000\";\n\
         int main(void) {\n\
         \x20   uint8_t b[64]; char got[129]; hdds_cdr_t c; uint32_t i; uint8_t u8; uint64_t u64; double f; uint16_t u16;\n\
         \x20   memset(b, 0xEE, sizeof b);\n\
         \x20   hdds_cdr_init(&c, b, sizeof b);\n\
         \x20   if (hdds_cdr_write_u8(&c, 1) || hdds_cdr_write_u64(&c, 0x0102030405060708ull) || hdds_cdr_write_f64(&c, 1.5)\n\
         \x20       || hdds_cdr_write_u16(&c, 7) || hdds_cdr_write_u64(&c, 9)) { printf(\"write error\\n\"); return 1; }\n\
         \x20   for (i = 0; i < c.pos; ++i) { sprintf(got + 2 * i, \"%02x\", b[i]); }\n\
         \x20   got[2 * c.pos] = 0;\n\
         \x20   if (strcmp(got, EXP) != 0) { printf(\"got %s\\nexp %s\\n\", got, EXP); return 2; }\n\
         \x20   hdds_cdr_init(&c, b, 32);\n\
         \x20   if (hdds_cdr_read_u8(&c, &u8) || u8 != 1 || hdds_cdr_read_u64(&c, &u64) || u64 != 0x0102030405060708ull\n\
         \x20       || hdds_cdr_read_f64(&c, &f) || f != 1.5 || hdds_cdr_read_u16(&c, &u16) || u16 != 7\n\
         \x20       || hdds_cdr_read_u64(&c, &u64) || u64 != 9 || c.pos != 32) { printf(\"read error\\n\"); return 3; }\n\
         \x20   printf(\"SDK_OK\\n\");\n\
         \x20   return 0;\n\
         }\n",
    )
    .expect("program");
    let exe = dir.join("sdk");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let out = Command::new(cc)
        .args([
            "-std=c99",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
        ])
        .arg(&include)
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success() && stdout.contains("SDK_OK"),
        "{stdout}"
    );
}

/// The read and write helpers of `sdk/c-micro/include/hdds_micro_cdr.h`
/// refuse what does not fit, with the exact error code:
/// - `hdds_cdr_read_string`: the XCDR2 length counts the NUL (DDS-XTypes 1.3
///   7.4.3.5.3 rule 3): a length of 0 or a last byte that is not NUL is
///   `HDDS_CDR_ERR_INVALID`, the destination left untouched;
/// - `hdds_cdr_align` (read side): padding past the end of the buffer is
///   `HDDS_CDR_ERR_UNDERFLOW`, the position unchanged (`remaining()` never
///   wraps);
/// - a length whose sum with the position wraps `uint32_t` is refused by
///   read / write bytes and read / write string (the subtraction form),
///   never copied (write string: at the end of a 4 GiB reservation, Linux
///   64-bit only);
/// - `hdds_cdr_remaining` is 0 for a position past the size.
#[test]
fn c_micro_sdk_helpers_refuse_hostile_input() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C-micro SDK hostile-input test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("c_micro_sdk_hostile");
    fs::create_dir_all(&dir).expect("dir");
    let src = dir.join("sdk_hostile.c");
    fs::write(
        &src,
        r#"#define _DEFAULT_SOURCE
#include <stdio.h>
#include "hdds_micro_cdr.h"
#if defined(__linux__) && UINTPTR_MAX > 0xFFFFFFFFu
#include <sys/mman.h>
#define HAVE_4G 1
#endif
static int fails = 0;
static void check(const char* label, long got, long want) {
    if (got == want) { printf("OK %s\n", label); } else { printf("FAIL %s got %ld want %ld\n", label, got, want); fails++; }
}
static uint32_t unhex(const char* s, uint8_t* o) { uint32_t n = (uint32_t)(strlen(s) / 2); uint32_t i; unsigned int x; for (i = 0; i < n; ++i) { sscanf(s + 2 * i, "%2x", &x); o[i] = (uint8_t)x; } return n; }
int main(void) {
    uint8_t b[64];
    char s[16];
    hdds_cdr_t c;
    uint8_t u8;
    uint32_t u32;
    uint32_t got_len;
    uint8_t data[8];
    /* read_string: "ab" with its NUL */
    hdds_cdr_init(&c, b, unhex("0300000061620000", b));
    memset(s, 'Z', sizeof s);
    check("str_ok", hdds_cdr_read_string(&c, s, sizeof s), HDDS_CDR_OK);
    check("str_ok_value", strcmp(s, "ab"), 0);
    check("str_ok_pos", (long)c.pos, 7);
    /* length 0: no room for the NUL the length must count */
    hdds_cdr_init(&c, b, unhex("00000000", b));
    memset(s, 'Z', sizeof s);
    check("str_len0", hdds_cdr_read_string(&c, s, sizeof s), HDDS_CDR_ERR_INVALID);
    check("str_len0_untouched", s[0], 'Z');
    /* "abc" whose last byte is not NUL */
    hdds_cdr_init(&c, b, unhex("03000000616263", b));
    memset(s, 'Z', sizeof s);
    check("str_no_nul", hdds_cdr_read_string(&c, s, sizeof s), HDDS_CDR_ERR_INVALID);
    check("str_no_nul_untouched", s[0], 'Z');
    /* length past the end */
    hdds_cdr_init(&c, b, unhex("08000000616200", b));
    check("str_past_end", hdds_cdr_read_string(&c, s, sizeof s), HDDS_CDR_ERR_UNDERFLOW);
    /* length 0xFFFFFFFE at position 4: pos + len wraps to 2 */
    hdds_cdr_init(&c, b, unhex("feffffff61620000", b));
    check("str_wrap", hdds_cdr_read_string(&c, s, 0xFFFFFFFFu), HDDS_CDR_ERR_UNDERFLOW);
    /* read_bytes: the same wrapping length; on every error the length out
     * is 0, never the length the input claimed */
    hdds_cdr_init(&c, b, unhex("feffffff61620000", b));
    got_len = 0xA5A5A5A5u;
    check("rbytes_wrap", hdds_cdr_read_bytes(&c, data, 0xFFFFFFFFu, &got_len), HDDS_CDR_ERR_UNDERFLOW);
    check("rbytes_wrap_len", (long)got_len, 0);
    hdds_cdr_init(&c, b, unhex("09000000616263646566676869", b));
    got_len = 0xA5A5A5A5u;
    check("rbytes_over_max", hdds_cdr_read_bytes(&c, data, sizeof data, &got_len), HDDS_CDR_ERR_OVERFLOW);
    check("rbytes_over_max_len", (long)got_len, 0);
    hdds_cdr_init(&c, b, unhex("0200", b));
    got_len = 0xA5A5A5A5u;
    check("rbytes_short_len", hdds_cdr_read_bytes(&c, data, sizeof data, &got_len), HDDS_CDR_ERR_UNDERFLOW);
    check("rbytes_short_len_len", (long)got_len, 0);
    hdds_cdr_init(&c, b, unhex("020000006162", b));
    check("rbytes_ok", hdds_cdr_read_bytes(&c, data, sizeof data, &got_len), HDDS_CDR_OK);
    check("rbytes_ok_len", (long)got_len, 2);
    /* write_bytes: a cursor claiming UINT32_MAX bytes; the length word
     * fits at 0, the 0xFFFFFFFE bytes after it wrap pos + len */
    hdds_cdr_init(&c, b, 0xFFFFFFFFu);
    check("wbytes_wrap", hdds_cdr_write_bytes(&c, data, 0xFFFFFFFEu), HDDS_CDR_ERR_OVERFLOW);
    check("wbytes_wrap_pos", (long)c.pos, 4);
    /* align (read side) past the end: 5 bytes, position 5 */
    hdds_cdr_init(&c, b, unhex("0100000002", b));
    check("align_u32", hdds_cdr_read_u32(&c, &u32), HDDS_CDR_OK);
    check("align_u8", hdds_cdr_read_u8(&c, &u8), HDDS_CDR_OK);
    check("align_past_end", hdds_cdr_align(&c, 4u), HDDS_CDR_ERR_UNDERFLOW);
    check("align_past_end_pos", (long)c.pos, 5);
    check("align_past_end_remaining", (long)hdds_cdr_remaining(&c), 0);
    /* a u32 whose padding passes the end: 6 bytes, position 5 */
    hdds_cdr_init(&c, b, unhex("010000000203", b));
    c.pos = 5u;
    check("u32_pad_past_end", hdds_cdr_read_u32(&c, &u32), HDDS_CDR_ERR_UNDERFLOW);
    check("u32_pad_past_end_pos", (long)c.pos, 5);
    /* a position past the size (set by the caller): nothing remains */
    c.pos = 9u;
    check("remaining_pos_past_size", (long)hdds_cdr_remaining(&c), 0);
#ifdef HAVE_4G
    {
        /* write_string near the 4 GiB end of a reserved (lazily mapped)
         * buffer: the length word at 0xFFFFFFF0, then 27 bytes that wrap
         * pos + len; refused, never copied past the reservation. */
        uint8_t* big = (uint8_t*)mmap(NULL, (size_t)1 << 32, PROT_READ | PROT_WRITE,
                                      MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
        if (big == MAP_FAILED) {
            printf("FAIL wstring_wrap: no 4 GiB reservation\n");
            fails++;
        } else {
            hdds_cdr_init(&c, big, 0xFFFFFFFFu);
            c.pos = 0xFFFFFFF0u;
            check("wstring_wrap", hdds_cdr_write_string(&c, "abcdefghijklmnopqrstuvwxyz"), HDDS_CDR_ERR_OVERFLOW);
            check("wstring_wrap_pos", (long)c.pos, 0xFFFFFFF4L);
            munmap(big, (size_t)1 << 32);
        }
    }
#else
    printf("OK wstring_wrap (no 4 GiB address space: not checked)\n");
    printf("OK wstring_wrap_pos (not checked)\n");
#endif
    return fails ? 1 : 0;
}
"#,
    )
    .expect("program");
    let exe = dir.join("sdk_hostile");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let out = Command::new(cc)
        .args([
            "-std=c99",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
        ])
        .arg(&include)
        .arg("-o")
        .arg(&exe)
        .arg(&src)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success()
            && !stdout.contains("FAIL")
            && stdout.lines().filter(|l| l.starts_with("OK ")).count() == 29,
        "status {:?}\n{stdout}",
        run.status
    );
}
