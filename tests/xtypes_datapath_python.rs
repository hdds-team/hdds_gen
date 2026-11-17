// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated Python code against a foreign oracle.
//!
//! `fixtures/xtypes_datapath/corpus.idl` is compiled by hddsgen into a
//! Python module; one sample of each type is encoded under XCDR2 and must
//! give the bytes Cyclone DDS 0.11 serializes for the same sample
//! (`cyclone_vectors.txt`, the `xcdr2` lines, produced by
//! `cyclone_probe.c`), and Cyclone's bytes must decode to the sample, every
//! byte consumed. The Python backend has no XCDR1 encoder; its XCDR1
//! decoder (`decode_cdr1_le`) reads every XCDR1 body of a sample in the
//! foreign and specification-built fixtures (`xcdr1_common`) into the
//! sample, the evolution readers, the `spec_plprobe.txt` layouts with
//! their verdicts and the encapsulation dispatch.
//!
//! The whole corpus is expressible; `HoldU_def` (a `default` branch with a
//! discriminator no label names) is encoded too, the Python union keeping
//! its discriminator. `SCOPED_IDL` adds two modules declaring a type of
//! the same name and union labels of every kind, against bytes derived from
//! the specification's rules. Skipped, with a message, when `python3` is
//! not installed.

#![allow(clippy::expect_used, clippy::panic, clippy::needless_raw_string_hashes)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hddsgen::{Backend, Parser};

mod xcdr1_common;
use xcdr1_common::{cyclone_xcdr1, plprobe, with_pid_ignore, xcdr1_bodies, PLPROBE_IDL};

const CORPUS: &str = include_str!("fixtures/xtypes_datapath/corpus.idl");
const VECTORS: &str = include_str!("fixtures/xtypes_datapath/cyclone_vectors.txt");
/// `@mutable` unions, which Cyclone DDS 0.11 does not support, built from
/// the specification in the form hddsgen writes (`spec=`, the
/// must-understand flag on the discriminator header) and in another form a
/// reader must accept (`alt=`); the IDL is in the file's header.
const SPEC_MUTABLE_UNION: &str = include_str!("fixtures/xtypes_datapath/spec_mutable_union.txt");
/// A `@mutable @autoid(HASH)` union in the specification's form (`spec=`)
/// and another form a reader must accept (`alt=`); the IDL is in the
/// file's header.
const SPEC_HASHED_UNION: &str =
    include_str!("fixtures/xtypes_datapath/spec_mutable_union_hashed.txt");

/// Two modules declaring a type of the same name, nested modules referenced
/// from outside, and union labels of every kind the parser reads (scoped
/// enumerators, character literals, TRUE / FALSE, negative integers).
const SCOPED_IDL: &str = "
module a { @final struct T { octet x; long y; }; };
module b { @appendable struct T { string s; };
  module c { @final struct U { a::T t; T bt; }; }; };
@final struct HoldAB { a::T at; b::T bt; b::c::U u; };
module m { enum K { K0, K1, K2 }; };
@final union LU switch (m::K) { case m::K1: long a; case ::m::K2: short b; };
@final union CU switch (char) { case 'a': long a; case 'b': octet b; };
@final union BU switch (boolean) { case TRUE: long t; case FALSE: octet f; };
@final union NU switch (long) { case -1: long n; case 2: octet p; };
@final struct LabelHold { LU lu; CU cu; BU bu; NU nu; };
@final union LLU switch (long long) { case 5: octet a; case -2: short b; };
@final struct HoldLL { LLU u; };
@bit_bound(64) bitmask BM64 { BM64_0, @position(63) BM64_63 };
bitmask BM31 { @position(31) BM31_31 };
@final struct Hold64 { octet p; BM64 m; BM31 h; };
";

/// Samples of `SCOPED_IDL` and their XCDR2 bodies, derived by hand from
/// the rules of DDS-XTypes 1.3 7.4.3.5.3 (no foreign implementation was
/// run on them): alignment min(size, 4) from the start of the body (rule
/// 1), a FINAL struct its members in order, an APPENDABLE one a DHEADER
/// then its members (rule 30, spec text line 7013), a string its length
/// with the NUL, its bytes, the NUL, a union its discriminator at its
/// type's width then the member it selects (rule 26).
/// - `HoldAB`: `at` = `a::T {1, 2}`: `01` + 3 padding bytes +
///   `02000000`; `bt` = `b::T {"hi"}`: DHEADER 7, `03000000 686900`
///   (offsets 8 to 18); `u.t` = `a::T {3, 4}`: `03` at offset 19,
///   `04000000` at 20 (aligned, no padding); `u.bt` = `b::T {"z"}`:
///   DHEADER 6 at 24, `02000000 7a00`. A reader that took one `T` for the
///   other cannot produce these bytes.
/// - `LabelHold`: `LU` (an enumeration of 32 bits) `K2` = 2 selects `b`
///   (short 0x0102); `CU` 'b' (0x62) selects `b` (octet 7); `BU` TRUE
///   (01) selects `t` (long 5, aligned at 12); `NU` -1 selects `n` (6).
///   `LabelHold2` takes the other branches: `K1`, 'a' (long 10 aligned at
///   12), FALSE (octet 3), 2 (octet 4, the discriminator aligned at 20).
/// - `HoldLL`: a `long long` discriminator, 8 bytes (5 or -2), then the
///   selected octet or short (aligned at 2, offset 8).
/// - `Hold64`: a `@bit_bound(64)` bitmask with bits 0 and 63 set is the
///   uint64 0x8000000000000001, little endian, aligned at 4 under XCDR2
///   (`01000000 00000080` at offset 4); a 32-bit bitmask with bit 31 set
///   is the uint32 0x80000000 (`00000080`).
const SCOPED: &[(&str, &str, &str)] = &[
    (
        "HoldAB",
        "c.HoldAB(at=c.a.T(x=1, y=2), bt=c.b.T(s='hi'), u=c.b.c.U(t=c.a_T(x=3, y=4), bt=c.b_T(s='z')))",
        "01000000020000000700000003000000686900030400000006000000020000007a00",
    ),
    (
        "LabelHold",
        "c.LabelHold(lu=c.LU(_discriminator=c.m.K.K2, _value=0x0102), cu=c.CU(_discriminator=ord('b'), _value=7), bu=c.BU(_discriminator=True, _value=5), nu=c.NU(_discriminator=-1, _value=6))",
        "02000000020162070100000005000000ffffffff06000000",
    ),
    (
        "HoldLL",
        "c.HoldLL(u=c.LLU(_discriminator=5, _value=7))",
        "050000000000000007",
    ),
    (
        "HoldLL2",
        "c.HoldLL(u=c.LLU(_discriminator=-2, _value=0x0102))",
        "feffffffffffffff0201",
    ),
    (
        "Hold64",
        "c.Hold64(p=1, m=c.BM64(c.BM64.BM64_0 | c.BM64.BM64_63), h=c.BM31(c.BM31.BM31_31))",
        "01000000010000000000008000000080",
    ),
    (
        "LabelHold2",
        "c.LabelHold(lu=c.LU(_discriminator=c.m.K.K1, _value=9), cu=c.CU(_discriminator=ord('a'), _value=10), bu=c.BU(_discriminator=False, _value=3), nu=c.NU(_discriminator=2, _value=4))",
        "0100000009000000610000000a000000000300000200000004",
    ),
];

/// Readers with more members than the writer types `MutW`, `AppW` and
/// `NestAppW` of the corpus (type evolution), and a MUTABLE type with a
/// member of each kind for the defaults of DDS-XTypes 1.3 Table 9 (spec
/// text line 2311).
const EVOLUTION_IDL: &str = "
@mutable struct MutWider { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) long added; };
@appendable struct AppWider { long a; long b; long added; };
@final struct NestAppWider { octet p; AppWider w; long z; };
module ev { enum EF { EF_A = 5, EF_B }; };
@mutable struct MutDefaults { long l; string s; ev::EF e; Inner8 n; sequence<long> q;
  octet a[2]; UL u; BM8 m; @optional long o; boolean b; char c; double d; MutPrims mp; };
";

/// (label, Python value, Cyclone sample whose xcdr2 body is decoded, or
/// `None` and a body). A reader accepts data from a writer whose type has
/// fewer members: a MUTABLE member absent from the data, or an APPENDABLE
/// member after the DHEADER's end, takes its default (7.4.1.2.3, spec text
/// lines 6195-6203; lines 3390-3393), and the reader stops at the end of a
/// nested DHEADER (`NestAppW`: `z` = 5 is read after `w`). `MutDefaults`
/// decodes a MUTABLE body with no member (DHEADER 0): every member takes
/// its Table 9 default, the enumeration its first literal (5, not 0), the
/// union `UL` its discriminator's default 0, which selects the `default`
/// branch (octet 0), the optional member no value.
const EVOLUTION: &[(&str, &str, Option<&str>, &str)] = &[
    (
        "MutW>MutWider",
        "c.MutWider(a=1, b=2, s='w', added=0)",
        Some("MutW"),
        "",
    ),
    (
        "AppW>AppWider",
        "c.AppWider(a=1, b=2, added=0)",
        Some("AppW"),
        "",
    ),
    (
        "NestAppW>NestAppWider",
        "c.NestAppWider(p=1, w=c.AppWider(a=3, b=4, added=0), z=5)",
        Some("NestAppW"),
        "",
    ),
    (
        "MutDefaults",
        "c.MutDefaults(l=0, s='', e=c.ev.EF.EF_A, n=c.Inner8(x=0, y=0), q=[], a=[0, 0], \
         u=c.UL(_discriminator=0, _value=0), m=c.BM8(0), o=None, b=False, c=0, d=0.0, \
         mp=c.MutPrims(x=0, y=0, z=0))",
        None,
        "00000000",
    ),
];

/// The constructors give the Table 9 defaults (spec text lines 2311-2350),
/// written here from the specification.
const DEFAULTS_CHECK: &str = "
defaults_ok = (c.MutDefaults() == c.MutDefaults(l=0, s='', e=c.ev.EF.EF_A, n=c.Inner8(x=0, y=0), q=[],
                   a=[0, 0], u=c.UL(_discriminator=0, _value=0), m=c.BM8(0), o=None, b=False, c=0,
                   d=0.0, mp=c.MutPrims(x=0, y=0, z=0))
               and c.AppWider() == c.AppWider(a=0, b=0, added=0)
               and c.UE() == c.UE(_discriminator=c.E8.E8_A, _value=0)
               and c.UA() == c.UA(_discriminator=0, _value=None)
               and int(c.ev.EF.EF_A) == 5)
print('DEF_OK' if defaults_ok else 'DEF_FAIL %r' % (c.MutDefaults(),))
";

/// A char discriminator held as a one-character string encodes as its code;
/// each case property returns the value only when the discriminator
/// selects it (the `default` branch when no label matches).
const UNION_CHECKS: &str = "
try:
    cu_ok = c.CU(_discriminator='b', _value=7).encode_cdr2_le().hex() == '6207'
except Exception:
    cu_ok = False
ul_def, ul_1 = c.UL(_discriminator=9, _value=0x7F), c.UL(_discriminator=1, _value=5)
props_ok = (ul_def.o == 0x7F and ul_def.a is None and ul_1.a == 5 and ul_1.o is None
            and c.CU(_discriminator=ord('a'), _value=3).a == 3)
print('UNION_OK' if cu_ok and props_ok else 'UNION_FAIL %r %r' % (cu_ok, props_ok))
";

/// Names the module views must give: `b::c::U` is also `c.U` (its name is
/// unique in the file), `T` is not bound (two modules declare one), and
/// `type_name()` is the scoped IDL name.
const SCOPED_NAMES: &str = "
names_ok = (c.U is c.b_c_U and c.b.c.U is c.b_c_U and c.K is c.m_K and not hasattr(c, 'T')
            and c.a.T is c.a_T and c.b.T is c.b_T and c.b.c.U.type_name() == 'b::c::U'
            and c.HoldAB.type_name() == 'HoldAB')
print('NAMES_OK' if names_ok else 'NAMES_FAIL')
";

/// (sample name in `cyclone_vectors.txt`, Python value with `c` the
/// generated module). The values are those `cyclone_probe.c` writes.
const SAMPLES: &[(&str, &str)] = &[
    ("FinalPrims", "c.FinalPrims(a=0x11, b=0x0102030405060708, c=-2, d=1.5, e=True, f=0xAABBCCDD)"),
    ("NestFinal", "c.NestFinal(a=1, in8=c.Inner8(x=0x1122334455667788, y=7), z=9)"),
    ("NestOdd", "c.NestOdd(a=1, il=c.InnerL(x=0x01020304), s=5)"),
    ("NestApp", "c.NestApp(a=1, app=c.AppStruct(a=2, s='hi', b=3), z=4)"),
    ("MutDefault", "c.MutDefault(a=1, s='abc', d=2.0, o=7)"),
    ("MutIds", "c.MutIds(a=1, b=2, c=3, d=4)"),
    ("MutHash", "c.MutHash(a=1, b=2, c=3, d=4)"),
    ("MutHashid", "c.MutHashid(x=1, y=2, z=3)"),
    ("MutPrims", "c.MutPrims(x=1, y=2, z=3)"),
    ("MutCompact2", "c.MutCompact2(a=1, b=2, c=3)"),
    ("NestMut", "c.NestMut(p=1, m=c.MutPrims(x=1, y=2, z=3), q=2)"),
    ("MutOpt_all", "c.MutOpt(a=5, b='x', c=6)"),
    ("MutOpt_none", "c.MutOpt(a=None, b=None, c=6)"),
    ("DerivedF", "c.DerivedF(id=1, v=2.5, s=3, w=4)"),
    ("DerivedA", "c.DerivedA(id=1, name='n')"),
    ("DerivedM", "c.DerivedM(id=1, tag='t', extra=2, s=3, after=4)"),
    ("Enums", "c.Enums(a=c.E8.E8_B, b=c.E16.E16_B, c=c.E32.E32_C, x=0xEE, d=c.E16.E16_A, e=c.E8.E8_C)"),
    ("MutEnums", "c.MutEnums(a=c.E8.E8_C, b=c.E16.E16_B, c=c.E32.E32_B)"),
    ("AppEnums", "c.AppEnums(p=1, b=c.E16.E16_B)"),
    ("Seqs", "c.Seqs(p=1, u=[1, 2], bd=[0.5], q=2, s=[3, 4, 5], bl=[6])"),
    ("SeqStruct", "c.SeqStruct(p=1, v=[c.Inner8(x=1, y=2), c.Inner8(x=3, y=4)])"),
    ("SeqApp", "c.SeqApp(p=1, v=[c.AppStruct(a=1, s='a', b=2), c.AppStruct(a=3, s='', b=4)])"),
    ("SeqStr", "c.SeqStr(p=1, v=['ab', 'c'], b=['wxyz'])"),
    ("Arrs", "c.Arrs(p=1, a=[1, 2, 3], m=[[1, 2, 3], [4, 5, 6]], s=[[1, 2], [3, 4]])"),
    ("ArrStruct", "c.ArrStruct(p=1, a=[c.Inner8(x=1, y=2), c.Inner8(x=3, y=4)])"),
    ("HoldU_1", "c.HoldU(p=1, u=c.UL(_discriminator=1, _value=0x0A), z=9)"),
    ("HoldU_2", "c.HoldU(p=1, u=c.UL(_discriminator=2, _value='hello'), z=9)"),
    ("HoldU_3", "c.HoldU(p=1, u=c.UL(_discriminator=3, _value=0.25), z=9)"),
    ("HoldU_def", "c.HoldU(p=1, u=c.UL(_discriminator=9, _value=0x7F), z=9)"),
    ("HoldU_def0", "c.HoldU(p=1, u=c.UL(_discriminator=0, _value=0x7F), z=9)"),
    ("HoldUA_1", "c.HoldUA(p=1, u=c.UA(_discriminator=1, _value=5))"),
    ("HoldUA_2", "c.HoldUA(p=1, u=c.UA(_discriminator=2, _value='ab'))"),
    ("HoldUE_A", "c.HoldUE(p=1, u=c.UE(_discriminator=c.E8.E8_A, _value=6), z=2)"),
    ("HoldUE_B", "c.HoldUE(p=1, u=c.UE(_discriminator=c.E8.E8_B, _value=7), z=2)"),
    ("HoldUS_1", "c.HoldUS(p=1, u=c.US(_discriminator=1, _value=0x11), q=2)"),
    ("HoldUS_2", "c.HoldUS(p=1, u=c.US(_discriminator=2, _value=0x2233), q=2)"),
    ("HoldUS_3", "c.HoldUS(p=1, u=c.US(_discriminator=3, _value=0x44556677), q=2)"),
    ("BStr", "c.BStr(p=1, s='abcdefgh', z=2)"),
    ("Keyed", "c.Keyed(id=1, name='nm', v=3)"),
    ("KeyedApp", "c.KeyedApp(id=1, v=2)"),
    ("FinalOpt_all", "c.FinalOpt(t=1, a=5, b=6, u=2)"),
    ("FinalOpt_none", "c.FinalOpt(t=1, a=None, b=None, u=2)"),
    ("AppOpt_all", "c.AppOpt(t=1, a=7)"),
    ("AppOpt_none", "c.AppOpt(t=1, a=None)"),
    ("MutMix", "c.MutMix(sl=[1, 2], ss=['a'], fin=c.Inner8(x=3, y=4), app=c.AppStruct(a=5, s='b', b=6), mut=c.MutPrims(x=7, y=8, z=9), un=c.UL(_discriminator=2, _value='u'), arr=[10, 11], bs='xy', en=c.E16.E16_B, se=[c.E8.E8_B, c.E8.E8_C])"),
    ("SeqEnum", "c.SeqEnum(p=1, a=[c.E8.E8_C, c.E8.E8_A, c.E8.E8_B], b=[c.E32.E32_B], arr=[c.E8.E8_B, c.E8.E8_C, c.E8.E8_A])"),
    // wchar is an int (its UTF-16 code unit) in the generated class.
    ("WStr", "c.WStr(p=1, c=0x41, s='h\\u00e9', d=0x263A)"),
    ("Aliases", "c.Aliases(p=1, m=c.BM32(c.BM32.BM32_0 | c.BM32.BM32_5), n=c.BM8(c.BM8.BM8_1), l=[7, 8], ia=c.Inner8(x=9, y=10))"),
    ("ArrStr", "c.ArrStr(p=1, a=['ab', ''], b=['xyz', 'q'])"),
    ("MutLC", "c.MutLC(s1=[1, 2, 3], s2=[4], s4=[5, 6], s8=[7], sd=[0.5, 1.0], sb=[True], a1=[1, 2, 3], a2=[4, 5], a4=[6, 7], s='st', si=[c.Inner8(x=1, y=2)], e=c.E32.E32_C, m=c.BM32(c.BM32.BM32_1), o=None, a8=[8], b=True, s0=[])"),
    ("MutLC2", "c.MutLC2(ai=[c.Inner8(x=1, y=2)], ae=[c.E8.E8_B, c.E8.E8_C], sa=['s'], bm=c.BM8(c.BM8.BM8_0), sbm=[c.BM8(c.BM8.BM8_1)], ls=[3], ia=c.Inner8(x=4, y=5))"),
    // sequence<char> / sequence<wchar> are lists of character codes
    // (a char is an int in the generated class); string<4> is a str.
    ("SeqChar", "c.SeqChar(p=1, c=[0x78, 0x79], w=[0x41, 0x42], s='ab')"),
    ("MutSeqChar", "c.MutSeqChar(c=[0x71], w=[0x5A], z=5)"),
];

/// (fixture, sample in it, Python value): encoded to the `spec=` body;
/// both the `alt=` and the `spec=` bodies decode to it. `HoldUMh` carries
/// the MD5 name hashes of its cases as member ids (the spec's Example 2
/// values).
const MUTABLE_UNION: &[(&str, &str, &str)] = &[
    (
        SPEC_HASHED_UNION,
        "HoldUMh_1",
        "c.HoldUMh(p=1, u=c.UMh(_discriminator=1, _value=0x11))",
    ),
    (
        SPEC_HASHED_UNION,
        "HoldUMh_2",
        "c.HoldUMh(p=1, u=c.UMh(_discriminator=2, _value=0x22))",
    ),
    (
        SPEC_MUTABLE_UNION,
        "HoldUM_1",
        "c.HoldUM(p=1, u=c.UM(_discriminator=1, _value=10), z=9)",
    ),
    (
        SPEC_MUTABLE_UNION,
        "HoldUM_2",
        "c.HoldUM(p=1, u=c.UM(_discriminator=2, _value='hello'), z=9)",
    ),
    (
        SPEC_MUTABLE_UNION,
        "HoldUM_3",
        "c.HoldUM(p=1, u=c.UM(_discriminator=3, _value=0.25), z=9)",
    ),
];

/// Bodies a newer writer could send, derived from Cyclone's vectors by the
/// rules (not by an hddsgen encoder): (label, Python value, body, whether
/// decoding must fail).
/// - `NestApp+`: the APPENDABLE `AppStruct` with 4 more bytes inside its
///   DHEADER (rule 30, spec text line 7013): the reader skips them.
/// - `MutPrims+`: a member of unknown id 0x50 (LC 2) appended inside the
///   DHEADER (rules 21-22): skipped.
/// - `MutPrims!`: the same member with the must-understand flag: refused
///   (7.4.3.4.2).
/// - From the `spec=` body of `HoldUM_1` (rule 27): `HoldUM+` an unknown
///   member 5 appended (skipped); `HoldUM-disc` the discriminator member
///   left out (refused: the discriminator is required); `HoldUM-case`
///   discriminator 2 with the member of case 1 (refused: the case member
///   must be the one the discriminator selects); `HoldUM-empty` a body
///   with no member at all (refused: the discriminator is required even
///   when no case member is present); `HoldUM!` the unknown member of
///   `HoldUM+` with the must-understand flag (refused, 7.4.3.4.2).
const DERIVED: &[(&str, &str, &str, bool)] = &[
    (
        "NestApp+",
        "c.NestApp(a=1, app=c.AppStruct(a=2, s='hi', b=3), z=4)",
        "0100000010000000020000000300000068690003aabbccdd04000000",
        false,
    ),
    (
        "MutPrims+",
        "c.MutPrims(x=1, y=2, z=3)",
        "20000000000000200100000001000020020000000200002003000000500000200a000000",
        false,
    ),
    (
        "MutPrims!",
        "c.MutPrims(x=1, y=2, z=3)",
        "20000000000000200100000001000020020000000200002003000000500000a00a000000",
        true,
    ),
    (
        "HoldUM+",
        "c.HoldUM(p=1, u=c.UM(_discriminator=1, _value=10), z=9)",
        "0100000018000000000000a001000000010000200a00000005000020aabbccdd09000000",
        false,
    ),
    (
        "HoldUM-disc",
        "c.HoldUM(p=1, u=c.UM(_discriminator=1, _value=10), z=9)",
        "0100000008000000010000200a00000009000000",
        true,
    ),
    (
        "HoldUM-case",
        "c.HoldUM(p=1, u=c.UM(_discriminator=2, _value=10), z=9)",
        "0100000010000000000000a002000000010000200a00000009000000",
        true,
    ),
    (
        "HoldUM-empty",
        "c.HoldUM(p=1, u=c.UM(_discriminator=1, _value=10), z=9)",
        "010000000000000009000000",
        true,
    ),
    (
        "HoldUM!",
        "c.HoldUM(p=1, u=c.UM(_discriminator=1, _value=10), z=9)",
        "0100000018000000000000a001000000010000200a000000050000a0aabbccdd09000000",
        true,
    ),
];

const RUNNER_HEAD: &str = r#"import importlib.util, os, sys
d = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('corpus', os.path.join(d, 'corpus.py'))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
def chk(name, v, exp):
    try:
        got = v.encode_cdr2_le().hex()
        print(('ENC_OK %s' % name) if got == exp else ('ENC_FAIL %s\n  got %s\n  exp %s' % (name, got, exp)))
    except Exception as e:
        print('ENC_FAIL %s error %r' % (name, e))
    try:
        b = bytes.fromhex(exp)
        dv, n = type(v).decode_cdr2_le(b)
        print(('DEC_OK %s' % name) if dv == v and n == len(b) else ('DEC_FAIL %s used %d of %d\n  got %r' % (name, n, len(b), dv)))
    except Exception as e:
        print('DEC_FAIL %s error %r' % (name, e))
def decode_only(name, v, exp):
    try:
        b = bytes.fromhex(exp)
        dv, n = type(v).decode_cdr2_le(b)
        print(('DEC_OK %s' % name) if dv == v and n == len(b) else ('DEC_FAIL %s used %d of %d\n  got %r' % (name, n, len(b), dv)))
    except Exception as e:
        print('DEC_FAIL %s error %r' % (name, e))
def x1(name, v, body, refused=False):
    try:
        b = bytes.fromhex(body)
        dv, n = type(v).decode_cdr1_le(b)
        ok = (not refused) and dv == v and n == len(b)
        print(('X1_OK %s' % name) if ok else ('X1_FAIL %s used %d of %d\n  got %r' % (name, n, len(b), dv)))
    except ValueError as e:
        print(('X1_OK %s' % name) if refused else ('X1_FAIL %s error %r' % (name, e)))
def x1enc(name, v, enc, body, refused):
    try:
        b = bytes.fromhex(body)
        dv, n = type(v).decode_by_encapsulation(enc, b)
        ok = (not refused) and dv == v and n == len(b)
        print(('X1_OK %s' % name) if ok else ('X1_FAIL %s decoded %r' % (name, dv)))
    except ValueError as e:
        print(('X1_OK %s' % name) if refused else ('X1_FAIL %s error %r' % (name, e)))
def derived(name, v, body, refused):
    try:
        b = bytes.fromhex(body)
        dv, n = type(v).decode_cdr2_le(b)
        ok = (not refused) and dv == v and n == len(b)
        print(('DER_OK %s' % name) if ok else ('DER_FAIL %s decoded %r' % (name, dv)))
    except ValueError as e:
        print(('DER_OK %s' % name) if refused else ('DER_FAIL %s error %r' % (name, e)))
"#;

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::Python.generator().generate(&ast).expect("codegen")
}

/// The IDL of a union fixture (its `#   ` header lines).
fn union_idl(fixture: &str) -> String {
    let mut idl = String::new();
    for l in fixture.lines().filter_map(|l| l.strip_prefix("#   ")) {
        idl.push_str(l);
        idl.push('\n');
    }
    idl
}

/// (`alt=`, `spec=`) XCDR2 bodies of `sample` in a union fixture, without
/// the encapsulation header.
fn alt_spec_bodies(fixture: &str, sample: &str) -> (String, String) {
    fixture
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == sample && it.next()? == "xcdr2").then_some(())?;
            let alt = it.next()?.strip_prefix("alt=")?;
            let spec = it.next()?.strip_prefix("spec=")?;
            Some((alt[8..].to_string(), spec[8..].to_string()))
        })
        .unwrap_or_else(|| panic!("no xcdr2 line for {sample}"))
}

/// Cyclone's XCDR2 body of `sample`.
fn xcdr2_body(sample: &str) -> String {
    VECTORS
        .lines()
        .find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == sample && it.next()? == "xcdr2").then_some(())?;
            let _hdr = it.next()?;
            it.next()?.strip_prefix("body=").map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no Cyclone xcdr2 vector for {sample}"))
}

#[test]
fn python_backend_matches_cyclone_on_the_corpus() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found; Python XCDR2 data path test not run");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_python");
    fs::create_dir_all(&dir).expect("dir");
    fs::write(
        dir.join("corpus.py"),
        generate(&format!(
            "{CORPUS}{}{}{SCOPED_IDL}{EVOLUTION_IDL}{PLPROBE_IDL}",
            union_idl(SPEC_MUTABLE_UNION),
            union_idl(SPEC_HASHED_UNION)
        )),
    )
    .expect("corpus");
    let mut runner = String::from(RUNNER_HEAD);
    for (name, value) in SAMPLES {
        let _ = writeln!(runner, "chk('{name}', {value}, '{}')", xcdr2_body(name));
    }
    for (fixture, name, value) in MUTABLE_UNION {
        let (alt, spec) = alt_spec_bodies(fixture, name);
        let _ = writeln!(runner, "chk('{name}', {value}, '{spec}')");
        let _ = writeln!(runner, "decode_only('{name}/alt', {value}, '{alt}')");
    }
    for (name, value, body, refused) in DERIVED {
        let refused = if *refused { "True" } else { "False" };
        let _ = writeln!(runner, "derived('{name}', {value}, '{body}', {refused})");
    }
    for (name, value, body) in SCOPED {
        let _ = writeln!(runner, "chk('{name}', {value}, '{body}')");
    }
    for (name, value, sample, body) in EVOLUTION {
        let body = sample.map_or_else(|| (*body).to_string(), xcdr2_body);
        let _ = writeln!(runner, "decode_only('{name}', {value}, '{body}')");
    }
    let x1_checks = xcdr1_runner(&mut runner);
    runner.push_str(SCOPED_NAMES);
    runner.push_str(DEFAULTS_CHECK);
    runner.push_str(UNION_CHECKS);
    fs::write(dir.join("run.py"), runner).expect("runner");
    let out = Command::new("python3")
        .arg(dir.join("run.py"))
        .output()
        .expect("python3");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let count = |p: &str| stdout.lines().filter(|l| l.starts_with(p)).count();
    assert!(
        count("ENC_OK") == SAMPLES.len() + MUTABLE_UNION.len() + SCOPED.len()
            && count("DEC_OK")
                == SAMPLES.len() + 2 * MUTABLE_UNION.len() + SCOPED.len() + EVOLUTION.len()
            && count("DER_OK") == DERIVED.len()
            && count("NAMES_OK") == 1
            && count("DEF_OK") == 1
            && count("UNION_OK") == 1
            && count("X1_OK") == x1_checks
            && !stdout.contains("_FAIL"),
        "{stdout}"
    );
}

/// The `spec_plprobe.txt` sample.
const PLPROBE_VALUE: &str =
    "c.PlProbe(id=0x11223344, s=-2, name='hdds', big=0x0102030405060708, tag=0xab)";

/// The XCDR1 checks, appended to the runner; their number.
fn xcdr1_runner(runner: &mut String) -> usize {
    let mut n = 0;
    let samples = SAMPLES
        .iter()
        .copied()
        .chain(MUTABLE_UNION.iter().map(|(_, s, v)| (*s, *v)))
        .chain(
            EVOLUTION
                .iter()
                .filter_map(|(_, v, s, _)| s.filter(|s| *s != "NestAppW").map(|s| (s, *v))),
        );
    for (name, value) in samples {
        for (label, hex) in xcdr1_bodies(name) {
            let _ = writeln!(runner, "x1('{label}', {value}, '{hex}')");
            n += 1;
        }
    }
    let _ = writeln!(
        runner,
        "x1('MutPrims/pid_ignore', c.MutPrims(x=1, y=2, z=3), '{}')",
        with_pid_ignore(&cyclone_xcdr1("MutPrims"))
    );
    n += 1;
    for p in plprobe().into_iter().filter(|p| p.xcdr1) {
        let refused = if p.accept { "False" } else { "True" };
        let _ = writeln!(
            runner,
            "x1('plprobe {}', {PLPROBE_VALUE}, '{}', {refused})",
            p.label, p.body
        );
        n += 1;
    }
    let mut nf = xcdr1_bodies("NestFinal").into_iter();
    let (_, x1) = nf.next().expect("NestFinal");
    let x2 = xcdr2_body("NestFinal");
    let v = "c.NestFinal(a=1, in8=c.Inner8(x=0x1122334455667788, y=7), z=9)";
    for (label, enc, body, refused) in [
        ("cdr_le", 0x0001, &x1, "False"),
        ("cdr2_le", 0x0007, &x2, "False"),
        ("cdr_be", 0x0000, &x1, "True"),
        ("cdr2_be", 0x0006, &x2, "True"),
    ] {
        let _ = writeln!(runner, "x1enc('{label}', {v}, {enc}, '{body}', {refused})");
        n += 1;
    }
    n
}

/// Types of the storage bounds: `App64` builds 64 values from its 4-byte
/// DHEADER when its member is absent (the default array), `OptBig` and
/// `UBig` up to 2^18 from one byte; `Empty` takes no byte on the wire,
/// `E1` to `E12` neither (`En` holds two `En-1`: 2^n values built); `S34`
/// nests 34 FINAL structures around a `long` (4 bytes however deep).
fn zero_wire_idl() -> String {
    let mut s = String::from(
        "@appendable struct App64 { long a[64]; };\n\
         @final struct HQA { sequence<App64> q; };\n\
         @final struct HMA { map<long, App64> m; };\n\
         @final struct OptBig { @optional long a[262144]; };\n\
         @final struct HQOB { sequence<OptBig> q; };\n\
         @final union UBig switch (long) { case 1: long a[262144]; };\n\
         @final struct HQUB { sequence<UBig> q; };\n\
         @final struct Empty {};\n\
         @final struct HQE { sequence<Empty> q; };\n\
         @final struct HQQE { sequence<sequence<Empty>> q; };\n\
         @final struct E1 { Empty a; Empty b; };\n\
         @final struct OptLL { @optional long long a; };\n\
         @final struct HQOLL { sequence<OptLL> q; };\n\
         @final struct HQML { sequence<map<long, long>> q; };\n\
         @appendable struct V2 { long a[9]; };\n\
         @final struct HQV2 { sequence<V2> q; };\n\
         @final struct Opt4K { @optional long a[4096]; };\n\
         @final struct HQQO4K { sequence<sequence<Opt4K>> q; };\n\
         @final struct HQQOLL { sequence<sequence<OptLL>> q; };\n",
    );
    for i in 2..=12 {
        let _ = writeln!(s, "@final struct E{i} {{ E{} a; E{} b; }};", i - 1, i - 1);
    }
    s.push_str("@final struct S0 { long a; };\n");
    for i in 1..=34 {
        let _ = writeln!(s, "@final struct S{i} {{ S{} x; }};", i - 1);
    }
    s.push_str(
        "@final struct HQE4 { sequence<E4> q; };\n\
         @final struct HQE10 { sequence<E10> q; };\n\
         @final struct HQQE12 { sequence<sequence<E12>> q; };\n\
         @final struct HQS34 { sequence<S34> q; };\n",
    );
    s
}

/// (label, type, body in hex, `None` when the body decodes (every byte
/// consumed, or the 8 bytes of the sequence with trailing bytes left), else
/// the exact `ValueError` message).
const ZERO_WIRE_CASES: &[(&str, &str, &str, Option<&str>)] = &[
    ("seqempty_fits", "HQE", "0400000003000000", None),
    (
        "seqempty_count_1025",
        "HQE",
        "0400000001040000",
        Some("1025 elements of no wire size (1 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode"),
    ),
    (
        "seqempty_count_10m",
        "HQE",
        "0400000080969800",
        Some("10000000 elements of no wire size (1 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode"),
    ),
    (
        "seqseqempty_inner_max",
        "HQQE",
        "100000000200000004000000ffffffff04000000ffffffff",
        Some("4294967295 elements of no wire size (1 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode"),
    ),
    ("seqe10_one", "HQE10", "0400000001000000", None),
    (
        "seqe10_count_2",
        "HQE10",
        "0400000002000000",
        Some("2 elements of no wire size (1024 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode"),
    ),
];

const ZERO_WIRE_RUNNER: &str = r#"import importlib.util, os, sys
d = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('zw', os.path.join(d, 'zw.py'))
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
def case(name, t, body, want, used=None):
    b = bytes.fromhex(body) if isinstance(body, str) else bytes(body)
    try:
        v, n = getattr(c, t).decode_cdr2_le(b)
        exp = len(b) if used is None else used
        ok = want is None and n == exp
        print(('ZW_OK %s' % name) if ok else ('ZW_FAIL %s decoded, %d bytes used' % (name, n)))
    except ValueError as e:
        print(('ZW_OK %s' % name) if str(e) == want else ('ZW_FAIL %s error %r' % (name, e)))
CAP = 'elements of no wire size (1 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode'
# 1025 elements with 200 bytes left: the values budget would allow them,
# the zero-wire budget refuses.
case('seqempty_count_1025_left_200', 'HQE', '0400000001040000' + '00' * 200, '1025 ' + CAP)
# sequence<E4> of 128 (2048 values) with 1024 bytes left after the count:
# within 64 times the body, beyond the 1024 zero-wire values of the decode
# (the body does not lift them); 64 (1024 values) decode.
case('seqe4_128_over_cap', 'HQE4', '0400000080000000' + '00' * 1024,
     '128 elements of no wire size (16 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode')
case('seqe4_64_at_cap', 'HQE4', '0400000040000000' + '00' * 1024, None, 8)
def seqseq(counts):
    b = bytearray(8 + 8 * len(counts))
    b[0:4] = (len(b) - 4).to_bytes(4, 'little')
    b[4:8] = len(counts).to_bytes(4, 'little')
    for i, k in enumerate(counts):
        b[8 + 8 * i:12 + 8 * i] = (4).to_bytes(4, 'little')
        b[12 + 8 * i:16 + 8 * i] = k.to_bytes(4, 'little')
    return b
# The budgets hold for the whole decode, not per collection: 8191 inner
# sequences of 1024 elements in 64 KiB (each within the cap) are refused at
# the second; with every inner count 0xFFFFFFFF at the first; 4 of 256
# decode, 5 of 205 do not.
case('seqseqempty_64k_per_decode', 'HQQE', seqseq([1024] * 8191),
     '1024 elements of no wire size (1 values each) exceed the 0 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode')
case('seqseqempty_64k_inner_max', 'HQQE', seqseq([0xFFFFFFFF] * 8191), '4294967295 ' + CAP)
case('seqseqempty_4x256', 'HQQE', seqseq([256] * 4), None)
case('seqseqempty_5x205', 'HQQE', seqseq([205] * 5),
     '205 elements of no wire size (1 values each) exceed the 204 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode')
# sequence<sequence<E12>> (E12: 4096 values, no byte on the wire), 8192
# inner sequences of 100 in a 65544-byte body: each passed the
# per-collection bounds of earlier releases (killed at 4 GB); refused at
# the first inner sequence.
case('seqseqe12_64k', 'HQQE12', seqseq([100] * 8192),
     '100 elements of no wire size (4096 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode')
# sequence<App64>: one present element decodes; so do 16 absent elements
# (4 bytes each, 64 values built each: an older writer's elements), and 3
# map entries of 8 bytes, 65 values each.
case('seqapp64_one_present', 'HQA', '08010000' + '01000000' + '00010000' + '00' * 256, None)
case('seqapp64_absent_16', 'HQA', '44000000' + '10000000' + '00000000' * 16, None)
case('mapapp64_count_3', 'HMA', '1c000000' + '03000000' + '00' * 24, None)
# An absent optional array, a union whose only branch is the array: counted
# at their largest value, like the C and C++ holders.
case('seqoptbig_count_16', 'HQOB', '14000000' + '10000000' + '00' * 16,
     '16 elements of 262144 values each exceed the 1536 values left of the decode budget')
case('sequbig_count_16', 'HQUB', '14000000' + '10000000' + '00' * 16,
     '16 elements of 262145 values each exceed the 1536 values left of the decode budget')
def zseq(count, each):
    b = bytearray(8 + count * each)
    b[0:4] = (len(b) - 4).to_bytes(4, 'little')
    b[4:8] = count.to_bytes(4, 'little')
    return b
# Valid data of many values per wire byte: all-absent optional members,
# empty maps, APPENDABLE elements of an older writer (a DHEADER of 0).
case('seqoptll_absent_1', 'HQOLL', zseq(1, 1), None)
case('seqoptll_absent_1000', 'HQOLL', zseq(1000, 1), None)
case('seqmap_empty_100', 'HQML', zseq(100, 4), None)
case('seqapp_older_writer_1000', 'HQV2', zseq(1000, 4), None)
def nested(outer, k):
    b = bytearray(8)
    for _ in range(outer):
        b += bytes((4 - len(b) % 4) % 4)
        b += (4 + k).to_bytes(4, 'little') + k.to_bytes(4, 'little') + bytes(k)
    b[0:4] = (len(b) - 4).to_bytes(4, 'little')
    b[4:8] = outer.to_bytes(4, 'little')
    return b
# Nested collections of elements that take wire bytes share the budget too:
# 100 inner sequences of one absent Opt4K (1 byte, 4096 values) in 1205
# bytes, each within the budget alone (64 * 1205 = 77120 values), are
# refused at the nineteenth (18 * 4096 charged); 3 of 10 absent OptLL
# decode.
case('seqseqopt4k_100', 'HQQO4K', nested(100, 1),
     '1 elements of 4096 values each exceed the 3392 values left of the decode budget')
case('seqseqoptll_3x10', 'HQQOLL', nested(3, 10), None)
# 34 FINAL structures around a long take its 4 bytes however deep: 1025
# present elements decode (not counted as elements of no wire size).
s34 = bytearray(8 + 4 * 1025)
s34[0:4] = (len(s34) - 4).to_bytes(4, 'little')
s34[4:8] = (1025).to_bytes(4, 'little')
case('seqs34_count_1025', 'HQS34', s34, None)
# The bounds are module globals: raised, 1025 elements decode.
c.CDR_MAX_ZERO_WIRE_ELEMENTS = 2048
case('seqempty_count_1025_raised', 'HQE', '0400000001040000', None)
c.CDR_MAX_ZERO_WIRE_ELEMENTS = 1024
"#;

/// The values a decode can build are bounded before any is, for the whole
/// decode however its collections nest: an absent member takes its whole
/// default (an array) from a few bytes, so the decoder refuses more values
/// than the larger of `CDR_MAX_ZERO_WIRE_ELEMENTS` (1024) and
/// `CDR_MAX_ALLOC_RATIO` (64) times the body. Elements that take no byte on
/// the wire (an empty FINAL structure, DDS-XTypes 1.3 7.4.3.5.3 rule 17,
/// spec text lines 6887-6891) are not bounded by the bytes: the decode is
/// refused beyond `CDR_MAX_ZERO_WIRE_ELEMENTS` values of them (one for an
/// empty structure). Every refusal is checked
/// by its exact message. The run is killed and failed after 60 s.
#[test]
fn python_decoder_bounds_sequence_storage() {
    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not found; Python storage bounds test not run");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_python_zw");
    fs::create_dir_all(&dir).expect("dir");
    fs::write(dir.join("zw.py"), generate(&zero_wire_idl())).expect("module");
    let mut runner = String::from(ZERO_WIRE_RUNNER);
    for (name, ty, body, want) in ZERO_WIRE_CASES {
        let want = want.map_or_else(|| "None".to_string(), |w| format!("'{w}'"));
        let _ = writeln!(runner, "case('{name}', '{ty}', '{body}', {want})");
    }
    fs::write(dir.join("run.py"), runner).expect("runner");
    let mut child = Command::new("python3")
        .arg(dir.join("run.py"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("python3");
    let start = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("Python zero-wire run still running after 60 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("output");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success()
            && stdout.lines().filter(|l| l.starts_with("ZW_OK ")).count()
                == ZERO_WIRE_CASES.len() + 21
            && !stdout.contains("_FAIL"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
