// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated C code against a foreign oracle.
//!
//! `fixtures/xtypes_datapath/corpus.idl` is compiled by hddsgen's C backend;
//! every sample `cyclone_probe.c` serializes is built in C and
//!
//! 1. encoded: the bytes must be Cyclone DDS 0.11's XCDR2 body
//!    (`cyclone_vectors.txt`), `_max_cdr2_size` must be that length, and a
//!    buffer one byte short must be refused;
//! 2. decoded from Cyclone's body (into a value pre-filled with 0xA5, so a
//!    member the decoder skips cannot keep a sample value): every byte must
//!    be consumed and the decoded value must encode to Cyclone's body again.
//!    The encoder being injective on these values (and equal to Cyclone on
//!    the sample), this is "the decoded value equals the sample".
//!
//! Every corpus type is expressible in C: no sample is excluded. The C
//! union keeps the discriminator it read, so `HoldU_def` (a `default`
//! discriminator of 9) is encoded too.
//!
//! XCDR1 (`_decode_cdr1_le_alloc`): every XCDR1 body of a sample in the
//! foreign and specification-built fixtures (`xcdr1_common`) is decoded,
//! and the value must encode to the sample's XCDR2 body; the evolution
//! readers, the specification's parameter-list layouts (`spec_plprobe.txt`)
//! with their verdicts, the `_at` origin, the encapsulation dispatch and
//! hostile parameter lists are checked too.
//!
//! The program runs three times: with the C99 header under `-std=c99` and with
//! the `--c-standard c89` header under `-std=c89 -pedantic-errors` (the
//! program itself is C89). Skipped, with a message, without a C compiler.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::codegen::c::CGenerator;
use hddsgen::codegen::{CStandard, CodeGenerator};
use hddsgen::Parser;

mod xcdr1_common;
use xcdr1_common::{cyclone_xcdr1, plprobe, plprobe_xcdr2, spec_xcdr1, xcdr1_bodies, PLPROBE_IDL};

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
/// `cyclone_probe.c` writes. Decoding takes the storage of pointer holders
/// from an arena (`hdds_x2c_arena_alloc`), reset before each decode.
/// Samples of `spec_mutable_union.txt` whose `alt=` body is also decoded.
const ALT_READS: usize = 5;

const SAMPLES: &[(&str, &str, &str)] = &[
    ("FinalPrims", "FinalPrims", "v.a = 0x11; v.b = I64(0x01020304, 0x05060708); v.c = -2; v.d = 1.5; v.e = true; v.f = 0xAABBCCDDu;"),
    ("NestFinal", "NestFinal", "v.a = 1; v.in8.x = I64(0x11223344, 0x55667788); v.in8.y = 7; v.z = 9;"),
    ("NestOdd", "NestOdd", "v.a = 1; v.il.x = 0x01020304; v.s = 5;"),
    ("NestApp", "NestApp", "v.a = 1; v.app.a = 2; v.app.s = (char*)\"hi\"; v.app.b = 3; v.z = 4;"),
    ("MutDefault", "MutDefault", "v.a = 1; v.s = (char*)\"abc\"; v.d = 2.0; v.o = 7;"),
    ("MutIds", "MutIds", "v.a = 1; v.b = 2; v.c = 3; v.d = 4;"),
    ("MutHash", "MutHash", "v.a = 1; v.b = 2; v.c = 3; v.d = 4;"),
    ("MutHashid", "MutHashid", "v.x = 1; v.y = 2; v.z = 3;"),
    ("MutPrims", "MutPrims", "v.x = 1; v.y = 2; v.z = 3;"),
    ("MutCompact2", "MutCompact2", "v.a = 1; v.b = 2; v.c = 3;"),
    ("NestMut", "NestMut", "v.p = 1; v.m.x = 1; v.m.y = 2; v.m.z = 3; v.q = 2;"),
    ("MutOpt_all", "MutOpt", "v.has_a = 1; v.a = 5; v.has_b = 1; v.b = (char*)\"x\"; v.c = 6;"),
    ("MutOpt_none", "MutOpt", "v.has_a = 0; v.has_b = 0; v.c = 6;"),
    ("DerivedF", "DerivedF", "v.base.id = 1; v.base.v = 2.5; v.s = 3; v.w = 4;"),
    ("DerivedA", "DerivedA", "v.base.id = 1; v.name = (char*)\"n\";"),
    ("DerivedM", "DerivedM", "v.base.id = 1; v.base.tag = (char*)\"t\"; v.extra = 2; v.s = 3; v.after = 4;"),
    ("Enums", "Enums", "v.a = E8_E8_B; v.b = E16_E16_B; v.c = E32_E32_C; v.x = 0xEE; v.d = E16_E16_A; v.e = E8_E8_C;"),
    ("MutEnums", "MutEnums", "v.a = E8_E8_C; v.b = E16_E16_B; v.c = E32_E32_B;"),
    ("AppEnums", "AppEnums", "v.p = 1; v.b = E16_E16_B;"),
    ("Seqs", "Seqs", "int64_t* u = (int64_t*)P(10); double* bd = (double*)P(11); int16_t* s = (int16_t*)P(12); int64_t* bl = (int64_t*)P(13); u[0] = 1; u[1] = 2; bd[0] = 0.5; s[0] = 3; s[1] = 4; s[2] = 5; bl[0] = 6; v.p = 1; v.u.data = u; v.u.len = 2; v.bd.data = bd; v.bd.len = 1; v.q = 2; v.s.data = s; v.s.len = 3; v.bl.data = bl; v.bl.len = 1;"),
    ("SeqStruct", "SeqStruct", "Inner8* e = (Inner8*)P(10); e[0].x = 1; e[0].y = 2; e[1].x = 3; e[1].y = 4; v.p = 1; v.v.data = e; v.v.len = 2;"),
    ("SeqApp", "SeqApp", "AppStruct* e = (AppStruct*)P(10); e[0].a = 1; e[0].s = (char*)\"a\"; e[0].b = 2; e[1].a = 3; e[1].s = (char*)\"\"; e[1].b = 4; v.p = 1; v.v.data = e; v.v.len = 2;"),
    ("SeqStr", "SeqStr", "char** s = (char**)P(10); char (*b)[5] = (char (*)[5])P(11); s[0] = (char*)\"ab\"; s[1] = (char*)\"c\"; strcpy(b[0], \"wxyz\"); v.p = 1; v.v.data = s; v.v.len = 2; v.b.data = b; v.b.len = 1;"),
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
    ("MutMix", "MutMix", "int32_t* sl = (int32_t*)P(10); char** ss = (char**)P(11); E8* se = (E8*)P(12); sl[0] = 1; sl[1] = 2; ss[0] = (char*)\"a\"; se[0] = E8_E8_B; se[1] = E8_E8_C; v.sl.data = sl; v.sl.len = 2; v.ss.data = ss; v.ss.len = 1; v.fin.x = 3; v.fin.y = 4; v.app.a = 5; v.app.s = (char*)\"b\"; v.app.b = 6; v.mut.x = 7; v.mut.y = 8; v.mut.z = 9; v.un._d = 2; strcpy(v.un._u.s, \"u\"); v.arr[0] = 10; v.arr[1] = 11; strcpy(v.bs, \"xy\"); v.en = E16_E16_B; v.se.data = se; v.se.len = 2;"),
    ("SeqEnum", "SeqEnum", "E8* a = (E8*)P(10); E32* b = (E32*)P(11); a[0] = E8_E8_C; a[1] = E8_E8_A; a[2] = E8_E8_B; b[0] = E32_E32_B; v.p = 1; v.a.data = a; v.a.len = 3; v.b.data = b; v.b.len = 1; v.arr[0] = E8_E8_B; v.arr[1] = E8_E8_C; v.arr[2] = E8_E8_A;"),
    ("WStr", "WStr", "v.p = 1; v.c = L'A'; v.s = (wchar_t*)L\"h\\x00e9\"; v.d = (wchar_t)0x263A;"),
    ("Aliases", "Aliases", "int32_t* l = (int32_t*)P(10); l[0] = 7; l[1] = 8; v.p = 1; v.m = BM32_BM32_0 | BM32_BM32_5; v.n = BM8_BM8_1; v.l.data = l; v.l.len = 2; v.ia.x = 9; v.ia.y = 10;"),
    ("ArrStr", "ArrStr", "v.p = 1; v.a[0] = (char*)\"ab\"; v.a[1] = (char*)\"\"; strcpy(v.b[0], \"xyz\"); strcpy(v.b[1], \"q\");"),
    ("MutLC", "MutLC", "uint8_t* s1 = (uint8_t*)P(10); int16_t* s2 = (int16_t*)P(11); int32_t* s4 = (int32_t*)P(12); int64_t* s8 = (int64_t*)P(13); double* sd = (double*)P(14); bool* sb = (bool*)P(15); Inner8* si = (Inner8*)P(9); s1[0] = 1; s1[1] = 2; s1[2] = 3; s2[0] = 4; s4[0] = 5; s4[1] = 6; s8[0] = 7; sd[0] = 0.5; sd[1] = 1.0; sb[0] = true; si[0].x = 1; si[0].y = 2; v.s1.data = s1; v.s1.len = 3; v.s2.data = s2; v.s2.len = 1; v.s4.data = s4; v.s4.len = 2; v.s8.data = s8; v.s8.len = 1; v.sd.data = sd; v.sd.len = 2; v.sb.data = sb; v.sb.len = 1; v.a1[0] = 1; v.a1[1] = 2; v.a1[2] = 3; v.a2[0] = 4; v.a2[1] = 5; v.a4[0] = 6; v.a4[1] = 7; v.s = (char*)\"st\"; v.si.data = si; v.si.len = 1; v.e = E32_E32_C; v.m = BM32_BM32_1; v.has_o = 0; v.a8[0] = 8; v.b = true; v.s0.data = NULL; v.s0.len = 0;"),
    ("MutLC2", "MutLC2", "BM8* sbm = (BM8*)P(10); int32_t* ls = (int32_t*)P(11); sbm[0] = BM8_BM8_1; ls[0] = 3; v.ai[0].x = 1; v.ai[0].y = 2; v.ae[0] = E8_E8_B; v.ae[1] = E8_E8_C; v.sa[0] = (char*)\"s\"; v.bm = BM8_BM8_0; v.sbm.data = sbm; v.sbm.len = 1; v.ls.data = ls; v.ls.len = 1; v.ia.x = 4; v.ia.y = 5;"),
    ("SeqChar", "SeqChar", "char* c = (char*)P(10); wchar_t* w = (wchar_t*)P(11); c[0] = 'x'; c[1] = 'y'; w[0] = L'A'; w[1] = L'B'; v.p = 1; v.c.data = c; v.c.len = 2; v.w.data = w; v.w.len = 2; strcpy(v.s, \"ab\");"),
    ("MutSeqChar", "MutSeqChar", "char* c = (char*)P(10); wchar_t* w = (wchar_t*)P(11); c[0] = 'q'; w[0] = L'Z'; v.c.data = c; v.c.len = 1; v.w.data = w; v.w.len = 1; v.z = 5;"),
    ("HoldUMh_1", "HoldUMh", "v.p = 1; v.u._d = 1; v.u._u.color = 0x11;"),
    ("HoldUMh_2", "HoldUMh", "v.p = 1; v.u._d = 2; v.u._u.shapesize = 0x22;"),
    ("Labels_1", "HoldLabels", "v.b._d = true; v.b._u.t = 0x11; v.c._d = 'c'; v.c._u.c = 0x22; v.n._d = -1; v.n._u.n = 0x33; v.s._d = LM_LE_LB; v.s._u.b = 0x44;"),
    ("Labels_2", "HoldLabels", "v.b._d = false; v.b._u.f = 0x0102; v.c._d = 'd'; v.c._u.d = 0x55; v.n._d = 2; v.n._u.p = 0x66; v.s._d = LM_LE_LA; v.s._u.a = 0x77;"),
    ("HoldUM_1", "HoldUM", "v.p = 1; v.u._d = 1; v.u._u.a = 10; v.z = 9;"),
    ("HoldUM_2", "HoldUM", "v.p = 1; v.u._d = 2; v.u._u.s = (char*)\"hello\"; v.z = 9;"),
    ("HoldUM_3", "HoldUM", "v.p = 1; v.u._d = 3; v.u._u.d = 0.25; v.z = 9;"),
];

/// Bodies written by hand from the specification (never by an hddsgen
/// encoder), decoded, then re-encoded: (label, C type, input hex, the body the decoded value must encode to, or "" when the
/// decoder must refuse the input).
const EXTRA_DECODE: &[(&str, &str, &str, &str)] = &[
    // Rule 30: an APPENDABLE value is delimited by its DHEADER; bytes a
    // newer writer appended (here 4) are skipped. Cyclone's NestApp body
    // with AppStruct's DHEADER 12 -> 16 and ee ee ee ee appended.
    (
        "NestApp_appended",
        "NestApp",
        "0100000010000000020000000300000068690003eeeeeeee04000000",
        "010000000c00000002000000030000006869000304000000",
    ),
    // Rule 22, 7.4.3.4.2: an unknown member without the M flag is skipped
    // (Cyclone's MutPrims body, DHEADER 0x18 -> 0x20, member id 9 LC 2).
    (
        "MutPrims_unknown_ok",
        "MutPrims",
        "200000000000002001000000010000200200000002000020030000000900002063000000",
        "18000000000000200100000001000020020000000200002003000000",
    ),
    // The same unknown member with the M flag must be refused.
    (
        "MutPrims_unknown_mu",
        "MutPrims",
        "20000000000000200100000001000020020000000200002003000000090000a063000000",
        "",
    ),
    // A non-optional member missing from a MUTABLE body takes its default
    // (7.4.1.2.3; Table 9: 0): MutPrims without z decodes as z = 0.
    (
        "MutPrims_missing",
        "MutPrims",
        "1000000000000020010000000100002002000000",
        "18000000000000200100000001000020020000000200002000000000",
    ),
    // A boolean byte other than 0 or 1 is invalid (7.4.3.5.3 / 7.2.2.1).
    (
        "FinalPrims_bool2",
        "FinalPrims",
        "110000000807060504030201feff0000000000000000f83f02000000ddccbbaa",
        "",
    ),
    // An enumeration value without a literal is invalid (E16 = 7).
    ("AppEnums_badlit", "AppEnums", "0400000001000700", ""),
    // A bounded string longer than its bound (string<8> holding 9 chars).
    (
        "BStr_overbound",
        "BStr",
        "010000000a00000061626364656667686900000002000000",
        "",
    ),
    // A string without its NUL (BStr.s holding "abcdefgh", length 8).
    ("BStr_nonul", "BStr", "0100000008000000616263646566676802000000", ""),
    // A sequence over its bound (Seqs.bd, sequence<double, 5>, with 6
    // elements, the body otherwise complete).
    (
        "Seqs_overbound",
        "Seqs",
        "010000000000000006000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000",
        "",
    ),
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
    ("MutW", "MutRich", "d.a == 1 && d.b == 2 && strcmp(d.s, \"w\") == 0 && d.e == EFIRST_EF_X && d.t != NULL && d.t[0] == 0 && d.in8.x == 0 && d.in8.y == 0 && d.q.len == 0 && d.q.data == NULL && d.u._d == 0 && d.u._u.o == 0 && d.bs[0] == 0 && d.arr[0] == 0 && d.arr[1] == 0 && d.has_o == 0 && d.w != NULL && d.w[0] == 0 && d.ao.t == 0 && d.ao.has_a == 0"),
    ("AppW", "AppRich", "d.a == 1 && d.b == 2 && d.e == EFIRST_EF_X && d.t != NULL && d.t[0] == 0 && d.u._d == 0 && d.u._u.o == 0 && d.has_o == 0 && d.app.a == 0 && d.app.s != NULL && d.app.s[0] == 0 && d.app.b == 0"),
];

/// XCDR1 type evolution (writer body, reader type, condition on `d`, or ""
/// when the reader must refuse the sample): the `Wider` readers of `MutW`
/// and `AppW` (an APPENDABLE member past the end of the data takes its
/// default), and `Keyed` read by a type without its key members: the
/// specification form flags them must-understand (refused), Cyclone's does
/// not (read).
fn xcdr1_evolution() -> Vec<(String, String, &'static str, &'static str)> {
    let wider = "d.a == 1 && d.b == 2 && strcmp(d.s, \"w\") == 0 && d.added == 0";
    let rich = EVOLUTION[3].2;
    vec![
        (
            "MutW/cyclone".into(),
            cyclone_xcdr1("MutW"),
            "MutWider",
            wider,
        ),
        ("MutW/spec".into(), spec_xcdr1("MutW"), "MutWider", wider),
        (
            "MutW/cyclone".into(),
            cyclone_xcdr1("MutW"),
            "MutRich",
            rich,
        ),
        (
            "AppW/cyclone".into(),
            cyclone_xcdr1("AppW"),
            "AppWider",
            "d.a == 1 && d.b == 2 && d.added == 0",
        ),
        (
            "AppW/cyclone".into(),
            cyclone_xcdr1("AppW"),
            "AppRich",
            EVOLUTION[4].2,
        ),
        ("Keyed/spec".into(), spec_xcdr1("Keyed"), "KeyedNarrow", ""),
        (
            "Keyed/cyclone".into(),
            cyclone_xcdr1("Keyed"),
            "KeyedNarrow",
            "d.v == 3",
        ),
    ]
}

/// XCDR1 bodies another writer may send, written by hand from the
/// specification (7.4.1.2.2): `PID_IGNORE` parameters (with and without
/// `FLAG_M`) are skipped. (label, C type, hex, sample whose XCDR2 body the
/// decoded value encodes to).
fn xcdr1_accepted() -> Vec<(&'static str, &'static str, String, &'static str)> {
    let mp = cyclone_xcdr1("MutPrims");
    vec![
        (
            "MutPrims_ignore",
            "MutPrims",
            format!("033f0400deadbeef{mp}"),
            "MutPrims",
        ),
        (
            "MutPrims_ignore_m",
            "MutPrims",
            format!("037f0000{mp}"),
            "MutPrims",
        ),
    ]
}

/// Hostile or malformed XCDR1 bodies, written by hand from the
/// specification (7.4.1.2.2), each refused: (label, C type, hex).
const XCDR1_REFUSED: &[(&str, &str, &str)] = &[
    // MutPrims without its PID_LIST_END.
    (
        "MutPrims_no_sentinel",
        "MutPrims",
        "017f0800000000000400000001000000",
    ),
    // A parameter longer than the buffer.
    (
        "MutPrims_overlong",
        "MutPrims",
        "017f080000000000ff00000001000000027f0000",
    ),
    // A short parameter longer than the buffer.
    (
        "MutPrims_short_overlong",
        "MutPrims",
        "00000800010000000000",
    ),
    // FinalOpt: PID_LIST_END where the optional member's parameter is.
    ("FinalOpt_sentinel", "FinalOpt", "01000000027f0000"),
    // Seqs: an element count the buffer cannot hold.
    ("Seqs_count", "Seqs", "01000000ffffff7f"),
    // MutPrims: a member value overrunning its parameter (x as a long in
    // a 2-byte parameter).
    (
        "MutPrims_overrun",
        "MutPrims",
        "017f08000000000002000000010000000000027f0000",
    ),
];

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn generate(standard: CStandard) -> String {
    let idl = format!(
        "{CORPUS}\n{UNION_IDL}\n{UNION_H_IDL}\n{LABELS_IDL}\n{EVOLUTION_IDL}\n{PLPROBE_IDL}\n\
         @mutable struct KeyedNarrow {{ @id(2) long v; }};\n"
    );
    let mut p = Parser::try_new(&idl).expect("lexer");
    let ast = p.parse().expect("parse");
    CGenerator::with_standard(standard)
        .generate(&ast)
        .expect("codegen")
}

/// The xcdr2 body Cyclone wrote for `sample`.
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

fn fn_prefix(ty: &str) -> String {
    ty.to_ascii_lowercase()
}

fn program() -> String {
    let mut body = String::new();
    for (name, ty, fill) in SAMPLES {
        let f = fn_prefix(ty);
        let exp = vector(name);
        let _ = writeln!(
            body,
            "    {{\n        {ty} v;\n        {ty} d;\n        int n;\n        size_t bl;\n        memset(&v, 0, sizeof v);\n        {{ {fill} }}\n\
             \x20       n = {f}_encode_cdr2_le(&v, out, sizeof out);\n\
             \x20       enc_check(\"{name}\", n, {f}_max_cdr2_size(&v), \"{exp}\");\n\
             \x20       bl = strlen(\"{exp}\") / 2;\n\
             \x20       if (bl > 0) {{ n = {f}_encode_cdr2_le(&v, out, bl - 1); short_check(\"{name}\", n); }}\n\
             \x20       bl = unhex(\"{exp}\", in);\n\
             \x20       memset(&d, 0xA5, sizeof d);\n\
             \x20       hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = {f}_decode_cdr2_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if (n != (int)bl) {{ printf(\"DEC_FAIL {name} used %d of %d\\n\", n, (int)bl); fails++; }}\n\
             \x20       else {{ n = {f}_encode_cdr2_le(&d, out, sizeof out); dec_check(\"{name}\", n, \"{exp}\"); }}\n    }}"
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
        let f = fn_prefix(ty);
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int n;\n        size_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = {f}_decode_cdr2_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if (strlen(\"{canon}\") == 0) {{ if (n < 0) {{ printf(\"XDEC_OK {name}\\n\"); }} else {{ printf(\"XDEC_FAIL {name} accepted\\n\"); fails++; }} }}\n\
             \x20       else if (n != (int)bl) {{ printf(\"XDEC_FAIL {name} used %d of %d\\n\", n, (int)bl); fails++; }}\n\
             \x20       else {{ n = {f}_encode_cdr2_le(&d, out, sizeof out); hexs(out, n, got); if (strcmp(got, \"{canon}\") == 0) {{ printf(\"XDEC_OK {name}\\n\"); }} else {{ printf(\"XDEC_FAIL {name}\\n  got %s\\n\", got); fails++; }} }}\n    }}"
        );
    }
    for (sample, ty, cond) in EVOLUTION {
        let f = fn_prefix(ty);
        let input = vector(sample);
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int n;\n        size_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = {f}_decode_cdr2_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if (n == (int)bl && ({cond})) {{ printf(\"EVO_OK {sample} as {ty}\\n\"); }} else {{ printf(\"EVO_FAIL {sample} as {ty} %d of %d\\n\", n, (int)bl); fails++; }}\n    }}"
        );
    }
    xcdr1_program(&mut body);
    format!(
        "#include <stdio.h>\n#include \"corpus.h\"\n\
         static union {{ int64_t i; double d; void* p; char c[256]; }} pool[16];\n\
         #define P(k) ((void*)&pool[k])\n\
         static uint8_t heap[65536];\n\
         static hdds_x2c_arena ar;\n\
         #define I64(h, l) ((int64_t)(((uint64_t)(h) << 32) | (uint64_t)(l)))\n\
         static int fails = 0;\n\
         static uint8_t out[4096];\n\
         static uint8_t in[4096];\n\
         static char got[8200];\n\
         static void hexs(const uint8_t* b, int n, char* s) {{ int i; s[0] = 0; for (i = 0; i < n; ++i) {{ sprintf(s + 2 * i, \"%02x\", b[i]); }} }}\n\
         static size_t unhex(const char* s, uint8_t* o);\n\
         static size_t unhexn(const char* const* parts, uint8_t* o) {{ size_t n = 0; for (; *parts != NULL; ++parts) {{ n += unhex(*parts, o + n); }} return n; }}\n\
         static size_t unhex(const char* s, uint8_t* o) {{ size_t n = strlen(s) / 2; size_t i; unsigned int x; for (i = 0; i < n; ++i) {{ sscanf(s + 2 * i, \"%2x\", &x); o[i] = (uint8_t)x; }} return n; }}\n\
         static void enc_check(const char* name, int n, size_t max, const char* exp) {{\n\
         \x20   if (n < 0) {{ printf(\"ENC_FAIL %s error %d\\n\", name, n); fails++; return; }}\n\
         \x20   hexs(out, n, got);\n\
         \x20   if (strcmp(got, exp) == 0 && max == (size_t)n) {{ printf(\"ENC_OK %s\\n\", name); }}\n\
         \x20   else {{ printf(\"ENC_FAIL %s max=%d\\n  got %s\\n  exp %s\\n\", name, (int)max, got, exp); fails++; }}\n\
         }}\n\
         static void short_check(const char* name, int n) {{\n\
         \x20   if (n == -CDR_BUFFER_TOO_SMALL) {{ printf(\"SHORT_OK %s\\n\", name); }} else {{ printf(\"SHORT_FAIL %s %d\\n\", name, n); fails++; }}\n\
         }}\n\
         static void dec_check(const char* name, int n, const char* exp) {{\n\
         \x20   if (n < 0) {{ printf(\"DEC_FAIL %s re-encode error %d\\n\", name, n); fails++; return; }}\n\
         \x20   hexs(out, n, got);\n\
         \x20   if (strcmp(got, exp) == 0) {{ printf(\"DEC_OK %s\\n\", name); }} else {{ printf(\"DEC_FAIL %s\\n  got %s\\n\", name, got); fails++; }}\n\
         }}\n\
         int main(void) {{\n{body}    return fails ? 1 : 0;\n}}\n"
    )
}

/// Encapsulation dispatch: (label, type, body, encapsulation id, decoded).
fn dispatch() -> [(&'static str, &'static str, String, &'static str, bool); 6] {
    [
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
        ("cdr2_le", "NestFinal", vector("NestFinal"), "0x0007u", true),
        (
            "pl_cdr2_le",
            "MutPrims",
            vector("MutPrims"),
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
            vector("MutPrims"),
            "0x000au",
            false,
        ),
    ]
}

/// `{"..", "..", NULL}`: the hex string `hex` in pieces short enough for a
/// C89 string literal (509 characters).
fn c_parts(hex: &str) -> String {
    let parts: Vec<String> = hex
        .as_bytes()
        .chunks(500)
        .map(|c| format!("\"{}\"", String::from_utf8_lossy(c)))
        .collect();
    format!("{{{}, NULL}}", parts.join(", "))
}

/// The number of XCDR1 bodies of the samples.
fn xcdr1_reads() -> usize {
    SAMPLES.iter().map(|(n, _, _)| xcdr1_bodies(n).len()).sum()
}

/// The XCDR1 checks of the program, appended to `body`.
fn xcdr1_program(body: &mut String) {
    // Every XCDR1 body of a sample decodes to a value that encodes to the
    // sample's XCDR2 body.
    for (name, ty, _) in SAMPLES {
        let f = fn_prefix(ty);
        let exp = vector(name);
        for (label, input) in xcdr1_bodies(name) {
            let parts = c_parts(&input);
            let _ = writeln!(
                body,
                "    {{\n        static const char* const hx[] = {parts};\n        {ty} d;\n        int n;\n        size_t bl;\n        bl = unhexn(hx, in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
                 \x20       n = {f}_decode_cdr1_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
                 \x20       if (n != (int)bl) {{ printf(\"X1_FAIL {label} used %d of %d\\n\", n, (int)bl); fails++; }}\n\
                 \x20       else {{ n = {f}_encode_cdr2_le(&d, out, sizeof out); hexs(out, n, got); if (strcmp(got, \"{exp}\") == 0) {{ printf(\"X1_OK {label}\\n\"); }} else {{ printf(\"X1_FAIL {label}\\n  got %s\\n\", got); fails++; }} }}\n    }}"
            );
        }
    }
    for (label, ty, input, sample) in xcdr1_accepted() {
        let f = fn_prefix(ty);
        let exp = vector(sample);
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int n;\n        size_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = {f}_decode_cdr1_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if (n != (int)bl) {{ printf(\"X1ACC_FAIL {label} used %d of %d\\n\", n, (int)bl); fails++; }}\n\
             \x20       else {{ n = {f}_encode_cdr2_le(&d, out, sizeof out); hexs(out, n, got); if (strcmp(got, \"{exp}\") == 0) {{ printf(\"X1ACC_OK {label}\\n\"); }} else {{ printf(\"X1ACC_FAIL {label}\\n  got %s\\n\", got); fails++; }} }}\n    }}"
        );
    }
    for (label, input, ty, cond) in xcdr1_evolution() {
        let f = fn_prefix(ty);
        let verdict = if cond.is_empty() {
            "n < 0".to_string()
        } else {
            format!("n == (int)bl && ({cond})")
        };
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int n;\n        size_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = {f}_decode_cdr1_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if ({verdict}) {{ printf(\"X1EVO_OK {label} as {ty}\\n\"); }} else {{ printf(\"X1EVO_FAIL {label} as {ty} %d of %d\\n\", n, (int)bl); fails++; }}\n    }}"
        );
    }
    let x2 = plprobe_xcdr2();
    for p in plprobe() {
        let ver = if p.xcdr1 { "cdr1" } else { "cdr2" };
        let (label, input) = (&p.label, &p.body);
        let verdict = if p.accept {
            format!("n == (int)bl && (n = plprobe_encode_cdr2_le(&d, out, sizeof out), hexs(out, n, got), strcmp(got, \"{x2}\") == 0)")
        } else {
            "n < 0".to_string()
        };
        let _ = writeln!(
            body,
            "    {{\n        PlProbe d;\n        int n;\n        size_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = plprobe_decode_{ver}_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if ({verdict}) {{ printf(\"PL_OK {label}\\n\"); }} else {{ printf(\"PL_FAIL {label} %d\\n\", n); fails++; }}\n    }}"
        );
    }
    for (label, ty, input) in XCDR1_REFUSED {
        let f = fn_prefix(ty);
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        int n;\n        size_t bl;\n        bl = unhex(\"{input}\", in);\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       n = {f}_decode_cdr1_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if (n < 0) {{ printf(\"X1REF_OK {label}\\n\"); }} else {{ printf(\"X1REF_FAIL {label} accepted\\n\"); fails++; }}\n    }}"
        );
    }
    // `_at`: the origin is the start of the buffer, an 8-byte member aligns
    // at 8 from it: Inner8 read at offset 1 of NestFinal's XCDR1 body.
    let nest = cyclone_xcdr1("NestFinal");
    let _ = writeln!(
        body,
        "    {{\n        Inner8 d;\n        size_t off = 1;\n        size_t bl = unhex(\"{nest}\", in);\n\
         \x20       int n = inner8_decode_cdr1_le_at(&d, in, bl, &off);\n\
         \x20       if (n == 0 && off == 20 && d.x == I64(0x11223344, 0x55667788) && d.y == 7) {{ printf(\"X1AT_OK\\n\"); }} else {{ printf(\"X1AT_FAIL %d %d\\n\", n, (int)off); fails++; }}\n    }}"
    );
    // The decoder picked from the encapsulation identifier.
    for (label, ty, input, id, ok) in dispatch() {
        let f = fn_prefix(ty);
        let verdict = if ok { "n == (int)bl" } else { "n < 0" };
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        size_t bl = unhex(\"{input}\", in);\n\
             \x20       int n = {f}_decode_by_encapsulation_alloc(&d, (uint16_t){id}, in, bl, NULL, NULL);\n\
             \x20       if ({verdict}) {{ printf(\"X1ENC_OK {label}\\n\"); }} else {{ printf(\"X1ENC_FAIL {label} %d\\n\", n); fails++; }}\n    }}"
        );
    }
}

fn compile_and_run(cc: &str, dir: &Path, name: &str, flags: &[&str], header: &str) -> String {
    fs::create_dir_all(dir).expect("dir");
    fs::write(dir.join("corpus.h"), header).expect("header");
    let src = dir.join(format!("{name}.c"));
    fs::write(&src, program()).expect("program");
    let exe = dir.join(name);
    let out = Command::new(cc)
        .args(flags)
        .args(["-Wall", "-Wextra", "-Werror", "-o"])
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
    stdout
}

fn check(out: &str) {
    let count = |p: &str| out.lines().filter(|l| l.starts_with(p)).count();
    assert!(
        !out.contains("_FAIL")
            && count("ENC_OK ") == SAMPLES.len()
            && count("SHORT_OK ") == SAMPLES.len()
            && count("DEC_OK ") == SAMPLES.len()
            && count("XDEC_OK ") == EXTRA_DECODE.len() + ALT_READS
            && count("EVO_OK ") == EVOLUTION.len()
            && count("X1_OK ") == xcdr1_reads()
            && count("X1EVO_OK ") == xcdr1_evolution().len()
            && count("PL_OK ") == plprobe().len()
            && count("X1REF_OK ") == XCDR1_REFUSED.len()
            && count("X1ACC_OK ") == xcdr1_accepted().len()
            && count("X1AT_OK") == 1
            && count("X1ENC_OK ") == dispatch().len(),
        "{out}"
    );
}

#[test]
fn c_backend_matches_cyclone_on_the_corpus() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C data path test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_c");
    let out = compile_and_run(cc, &dir, "dp_c99", &["-std=c99"], &generate(CStandard::C99));
    check(&out);
    let out = compile_and_run(
        cc,
        &dir.join("c89"),
        "dp_c89",
        &["-std=c89", "-pedantic-errors"],
        &generate(CStandard::C89),
    );
    check(&out);
    let out = compile_and_run(
        cc,
        &dir.join("c11"),
        "dp_c11",
        &["-std=c11", "-pedantic-errors"],
        &generate(CStandard::C11),
    );
    check(&out);
}
