// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Data path of the generated TypeScript code against a foreign oracle.
//!
//! `fixtures/xtypes_datapath/corpus.idl` is compiled by hddsgen into one
//! TypeScript file, run by `node --experimental-transform-types` (node
//! strips the types without checking them: that test proves the bytes;
//! `generated_typescript_passes_tsc_strict` checks the typing with
//! `tsc --strict --noEmit`; it needs a `tsc`, so it is `#[ignore]`d and
//! fails when requested without one: `npm install --prefix <target
//! dir>/ts typescript`, or `HDDS_TSC`, then `cargo test --test
//! xtypes_datapath_typescript -- --ignored`). One sample of each type is encoded under XCDR2 into a
//! buffer pre-filled with 0xAA (padding must be written as zero) and must
//! give the bytes Cyclone DDS 0.11 serializes for the same sample
//! (`cyclone_vectors.txt`, the `xcdr2` lines, produced by
//! `cyclone_probe.c`); Cyclone's bytes must decode to the sample, every
//! byte consumed. The TypeScript backend has no XCDR1 encoder; its XCDR1
//! decoder (`decode<Name>Cdr1`) reads every XCDR1 body of a sample in the
//! foreign and specification-built fixtures (`xcdr1_common`) into the
//! sample, the evolution readers, the `spec_plprobe.txt` layouts with
//! their verdicts, `PID_IGNORE` and the encapsulation dispatch.
//!
//! The whole corpus is expressible; `HoldU_def` (a `default` branch with a
//! discriminator no label names) is encoded too, the TypeScript union
//! keeping its discriminator. `SCOPED_IDL` adds two modules declaring a
//! type of the same name, nested modules and union labels of every kind,
//! against bytes derived from the specification's rules. Skipped, with a
//! message, without a `node` that runs TypeScript.

#![allow(clippy::expect_used, clippy::panic, clippy::needless_raw_string_hashes)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
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
const NODE_ARGS: [&str; 2] = ["--experimental-transform-types", "--no-warnings"];

/// Two modules declaring a type of the same name, nested modules referenced
/// from outside, and union labels of every kind the parser reads (scoped
/// enumerators, character literals, TRUE / FALSE, negative integers). The
/// same IDL as `tests/xtypes_datapath_python.rs`.
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

/// (label, encoder, decoder, value, XCDR2 body) of `SCOPED_IDL`, the bodies
/// derived by hand from the rules of DDS-XTypes 1.3 7.4.3.5.3 (see
/// `tests/xtypes_datapath_python.rs` for the derivation of `HoldAB` and
/// `LabelHold`). `b::c::U` alone, through the module view
/// (`b.c.encodeU`): `t.x` 03 and 3 padding bytes, `t.y` at 4, the DHEADER
/// of `bt` (6) at 8, `02000000 7a00`. `HoldLL`: a `long long`
/// discriminator (a bigint), 8 bytes, then the selected member. `Hold64`: a `@bit_bound(64)`
/// bitmask with bits 0 and 63 set is the uint64 0x8000000000000001, little
/// endian, aligned at 4 under XCDR2 (`01000000 00000080` at offset 4); a
/// 32-bit bitmask with bit 31 set is the uint32 0x80000000 (`00000080`).
const SCOPED: &[(&str, &str, &str, &str, &str)] = &[
    (
        "HoldAB",
        "encodeHoldAB",
        "decodeHoldAB",
        "{ at: { x: 1, y: 2 }, bt: { s: \"hi\" }, u: { t: { x: 3, y: 4 }, bt: { s: \"z\" } } }",
        "01000000020000000700000003000000686900030400000006000000020000007a00",
    ),
    (
        "U",
        "b.c.encodeU",
        "b.c.decodeU",
        "{ t: { x: 3, y: 4 }, bt: { s: \"z\" } }",
        "030000000400000006000000020000007a00",
    ),
    (
        "LabelHold",
        "encodeLabelHold",
        "decodeLabelHold",
        "{ lu: { discriminator: m.K.K2, b: 0x0102 }, cu: { discriminator: \"b\", b: 7 }, bu: { discriminator: true, t: 5 }, nu: { discriminator: -1, n: 6 } }",
        "02000000020162070100000005000000ffffffff06000000",
    ),
    (
        "HoldLL",
        "encodeHoldLL",
        "decodeHoldLL",
        "{ u: { discriminator: 5n, a: 7 } }",
        "050000000000000007",
    ),
    (
        "HoldLL2",
        "encodeHoldLL",
        "decodeHoldLL",
        "{ u: { discriminator: -2n, b: 0x0102 } }",
        "feffffffffffffff0201",
    ),
    (
        "Hold64",
        "encodeHold64",
        "decodeHold64",
        "{ p: 1, m: BM64.BM64_0 | BM64.BM64_63, h: BM31.BM31_31 }",
        "01000000010000000000008000000080",
    ),
    (
        "LabelHold2",
        "encodeLabelHold",
        "decodeLabelHold",
        "{ lu: { discriminator: m.K.K1, a: 9 }, cu: { discriminator: \"a\", a: 10 }, bu: { discriminator: false, f: 3 }, nu: { discriminator: 2, p: 4 } }",
        "0100000009000000610000000a000000000300000200000004",
    ),
];

/// Readers with more members than the writer types `MutW`, `AppW` and
/// `NestAppW` of the corpus (type evolution), and a MUTABLE type with a
/// member of each kind for the defaults of DDS-XTypes 1.3 Table 9 (spec
/// text line 2311). The same IDL as `tests/xtypes_datapath_python.rs`.
const EVOLUTION_IDL: &str = "
@mutable struct MutWider { @id(1) long a; @id(2) long b; @id(3) string s; @id(4) long added; };
@appendable struct AppWider { long a; long b; long added; };
@final struct NestAppWider { octet p; AppWider w; long z; };
module ev { enum EF { EF_A = 5, EF_B }; };
@mutable struct MutDefaults { long l; string s; ev::EF e; Inner8 n; sequence<long> q;
  octet a[2]; UL u; BM8 m; @optional long o; boolean b; char c; double d; MutPrims mp; };
";

/// (label, decoder, value, Cyclone sample whose xcdr2 body is decoded, or
/// `None` and a body): a MUTABLE member absent from the data, or an
/// APPENDABLE member after the DHEADER's end, takes its default
/// (7.4.1.2.3, spec text lines 6195-6203; lines 3390-3393); the reader
/// stops at the end of a nested DHEADER (`NestAppW`: `z` = 5). See
/// `tests/xtypes_datapath_python.rs` for `MutDefaults`; a char defaults to
/// NUL, a 64-bit integer to `0n`, the absent optional member is left out.
const EVOLUTION: &[(&str, &str, &str, Option<&str>, &str)] = &[
    (
        "MutW>MutWider",
        "decodeMutWider",
        "{ a: 1, b: 2, s: \"w\", added: 0 }",
        Some("MutW"),
        "",
    ),
    (
        "AppW>AppWider",
        "decodeAppWider",
        "{ a: 1, b: 2, added: 0 }",
        Some("AppW"),
        "",
    ),
    (
        "NestAppW>NestAppWider",
        "decodeNestAppWider",
        "{ p: 1, w: { a: 3, b: 4, added: 0 }, z: 5 }",
        Some("NestAppW"),
        "",
    ),
    (
        "MutDefaults",
        "decodeMutDefaults",
        MUT_DEFAULTS,
        None,
        "00000000",
    ),
];

/// Table 9 default of `MutDefaults`, written from the specification.
const MUT_DEFAULTS: &str = "{ l: 0, s: \"\", e: ev.EF.EF_A, n: { x: 0n, y: 0 }, q: [], a: [0, 0], \
    u: { discriminator: 0, o: 0 }, m: 0, b: false, c: \"\\u0000\", d: 0, mp: { x: 0, y: 0, z: 0 } }";

/// `default<Name>()` gives the Table 9 default.
const DEFAULTS_CHECK: &str = "
console.log(deq(defaultMutDefaults(), MUT_DEFAULTS) && deq(ev.EF.EF_A, 5)
  && deq(defaultAppWider(), { a: 0, b: 0, added: 0 }) && deq(defaultUL(), { discriminator: 0, o: 0 })
  && deq(defaultUE(), { discriminator: E8.E8_A, a: 0 }) && deq(defaultUA(), { discriminator: 0 })
  ? \"DEF_OK\" : \"DEF_FAIL\");
";

/// The module views export each declaration under its own name.
const SCOPED_NAMES: &str = "
console.log(b.c.encodeU === encodeb_c_U && a.decodeT === decodea_T && b.encodeT === encodeb_T
  && m.K === m_K && m.encodeK === encodem_K && typeof a.computeKeyT === \"function\"
  ? \"NAMES_OK\" : \"NAMES_FAIL\");
";

/// (sample name in `cyclone_vectors.txt`, TypeScript type, value). The
/// values are those `cyclone_probe.c` writes; 64-bit integers are bigints,
/// an absent optional member is left out.
const SAMPLES: &[(&str, &str, &str)] = &[
    ("FinalPrims", "FinalPrims", "{ a: 0x11, b: 0x0102030405060708n, c: -2, d: 1.5, e: true, f: 0xAABBCCDD }"),
    ("NestFinal", "NestFinal", "{ a: 1, in8: { x: 0x1122334455667788n, y: 7 }, z: 9 }"),
    ("NestOdd", "NestOdd", "{ a: 1, il: { x: 0x01020304 }, s: 5 }"),
    ("NestApp", "NestApp", "{ a: 1, app: { a: 2, s: \"hi\", b: 3 }, z: 4 }"),
    ("MutDefault", "MutDefault", "{ a: 1, s: \"abc\", d: 2.0, o: 7 }"),
    ("MutIds", "MutIds", "{ a: 1, b: 2, c: 3, d: 4 }"),
    ("MutHash", "MutHash", "{ a: 1, b: 2, c: 3, d: 4 }"),
    ("MutHashid", "MutHashid", "{ x: 1, y: 2, z: 3 }"),
    ("MutPrims", "MutPrims", "{ x: 1, y: 2, z: 3 }"),
    ("MutCompact2", "MutCompact2", "{ a: 1, b: 2n, c: 3 }"),
    ("NestMut", "NestMut", "{ p: 1, m: { x: 1, y: 2, z: 3 }, q: 2 }"),
    ("MutOpt_all", "MutOpt", "{ a: 5, b: \"x\", c: 6 }"),
    ("MutOpt_none", "MutOpt", "{ c: 6 }"),
    ("DerivedF", "DerivedF", "{ id: 1, v: 2.5, s: 3, w: 4n }"),
    ("DerivedA", "DerivedA", "{ id: 1, name: \"n\" }"),
    ("DerivedM", "DerivedM", "{ id: 1, tag: \"t\", extra: 2, s: 3, after: 4 }"),
    ("Enums", "Enums", "{ a: E8.E8_B, b: E16.E16_B, c: E32.E32_C, x: 0xEE, d: E16.E16_A, e: E8.E8_C }"),
    ("MutEnums", "MutEnums", "{ a: E8.E8_C, b: E16.E16_B, c: E32.E32_B }"),
    ("AppEnums", "AppEnums", "{ p: 1, b: E16.E16_B }"),
    ("Seqs", "Seqs", "{ p: 1, u: [1n, 2n], bd: [0.5], q: 2, s: [3, 4, 5], bl: [6n] }"),
    ("SeqStruct", "SeqStruct", "{ p: 1, v: [{ x: 1n, y: 2 }, { x: 3n, y: 4 }] }"),
    ("SeqApp", "SeqApp", "{ p: 1, v: [{ a: 1, s: \"a\", b: 2 }, { a: 3, s: \"\", b: 4 }] }"),
    ("SeqStr", "SeqStr", "{ p: 1, v: [\"ab\", \"c\"], b: [\"wxyz\"] }"),
    ("Arrs", "Arrs", "{ p: 1, a: [1n, 2n, 3n], m: [[1, 2, 3], [4, 5, 6]], s: [[1, 2], [3, 4]] }"),
    ("ArrStruct", "ArrStruct", "{ p: 1, a: [{ x: 1n, y: 2 }, { x: 3n, y: 4 }] }"),
    ("HoldU_1", "HoldU", "{ p: 1, u: { discriminator: 1, a: 0x0A }, z: 9 }"),
    ("HoldU_2", "HoldU", "{ p: 1, u: { discriminator: 2, s: \"hello\" }, z: 9 }"),
    ("HoldU_3", "HoldU", "{ p: 1, u: { discriminator: 3, d: 0.25 }, z: 9 }"),
    ("HoldU_def", "HoldU", "{ p: 1, u: { discriminator: 9, o: 0x7F }, z: 9 }"),
    ("HoldU_def0", "HoldU", "{ p: 1, u: { discriminator: 0, o: 0x7F }, z: 9 }"),
    ("HoldUA_1", "HoldUA", "{ p: 1, u: { discriminator: 1, a: 5n } }"),
    ("HoldUA_2", "HoldUA", "{ p: 1, u: { discriminator: 2, s: \"ab\" } }"),
    ("HoldUE_A", "HoldUE", "{ p: 1, u: { discriminator: E8.E8_A, a: 6 }, z: 2 }"),
    ("HoldUE_B", "HoldUE", "{ p: 1, u: { discriminator: E8.E8_B, b: 7 }, z: 2 }"),
    ("HoldUS_1", "HoldUS", "{ p: 1, u: { discriminator: 1, a: 0x11 }, q: 2 }"),
    ("HoldUS_2", "HoldUS", "{ p: 1, u: { discriminator: 2, b: 0x2233 }, q: 2 }"),
    ("HoldUS_3", "HoldUS", "{ p: 1, u: { discriminator: 3, c: 0x44556677 }, q: 2 }"),
    ("BStr", "BStr", "{ p: 1, s: \"abcdefgh\", z: 2 }"),
    ("Keyed", "Keyed", "{ id: 1, name: \"nm\", v: 3 }"),
    ("KeyedApp", "KeyedApp", "{ id: 1, v: 2 }"),
    ("FinalOpt_all", "FinalOpt", "{ t: 1, a: 5, b: 6n, u: 2 }"),
    ("FinalOpt_none", "FinalOpt", "{ t: 1, u: 2 }"),
    ("AppOpt_all", "AppOpt", "{ t: 1, a: 7 }"),
    ("AppOpt_none", "AppOpt", "{ t: 1 }"),
    ("MutMix", "MutMix", "{ sl: [1, 2], ss: [\"a\"], fin: { x: 3n, y: 4 }, app: { a: 5, s: \"b\", b: 6 }, mut: { x: 7, y: 8, z: 9 }, un: { discriminator: 2, s: \"u\" }, arr: [10n, 11n], bs: \"xy\", en: E16.E16_B, se: [E8.E8_B, E8.E8_C] }"),
    ("SeqEnum", "SeqEnum", "{ p: 1, a: [E8.E8_C, E8.E8_A, E8.E8_B], b: [E32.E32_B], arr: [E8.E8_B, E8.E8_C, E8.E8_A] }"),
    ("WStr", "WStr", "{ p: 1, c: \"A\", s: \"h\\u00e9\", d: \"\\u263A\" }"),
    ("Aliases", "Aliases", "{ p: 1, m: BM32.BM32_0 | BM32.BM32_5, n: BM8.BM8_1, l: [7, 8], ia: { x: 9n, y: 10 } }"),
    ("ArrStr", "ArrStr", "{ p: 1, a: [\"ab\", \"\"], b: [\"xyz\", \"q\"] }"),
    ("MutLC", "MutLC", "{ s1: [1, 2, 3], s2: [4], s4: [5, 6], s8: [7n], sd: [0.5, 1.0], sb: [true], a1: [1, 2, 3], a2: [4, 5], a4: [6, 7], s: \"st\", si: [{ x: 1n, y: 2 }], e: E32.E32_C, m: BM32.BM32_1, a8: [8n], b: true, s0: [] }"),
    ("MutLC2", "MutLC2", "{ ai: [{ x: 1n, y: 2 }], ae: [E8.E8_B, E8.E8_C], sa: [\"s\"], bm: BM8.BM8_0, sbm: [BM8.BM8_1], ls: [3], ia: { x: 4n, y: 5 } }"),
    // sequence<char> / sequence<wchar> are arrays of one-character
    // strings; string<4> is a string.
    ("SeqChar", "SeqChar", "{ p: 1, c: [\"x\", \"y\"], w: [\"A\", \"B\"], s: \"ab\" }"),
    ("MutSeqChar", "MutSeqChar", "{ c: [\"q\"], w: [\"Z\"], z: 5 }"),
];

/// (fixture, sample in it, type, value): encoded to the `spec=` body;
/// both the `alt=` and the `spec=` bodies decode to it. `HoldUMh` carries
/// the MD5 name hashes of its cases as member ids (the spec's Example 2
/// values).
const MUTABLE_UNION: &[(&str, &str, &str, &str)] = &[
    (
        SPEC_MUTABLE_UNION,
        "HoldUM_1",
        "HoldUM",
        "{ p: 1, u: { discriminator: 1, a: 10 }, z: 9 }",
    ),
    (
        SPEC_MUTABLE_UNION,
        "HoldUM_2",
        "HoldUM",
        "{ p: 1, u: { discriminator: 2, s: \"hello\" }, z: 9 }",
    ),
    (
        SPEC_MUTABLE_UNION,
        "HoldUM_3",
        "HoldUM",
        "{ p: 1, u: { discriminator: 3, d: 0.25 }, z: 9 }",
    ),
    (
        SPEC_HASHED_UNION,
        "HoldUMh_1",
        "HoldUMh",
        "{ p: 1, u: { discriminator: 1, color: 0x11 } }",
    ),
    (
        SPEC_HASHED_UNION,
        "HoldUMh_2",
        "HoldUMh",
        "{ p: 1, u: { discriminator: 2, shapesize: 0x22 } }",
    ),
];

/// Bodies a newer writer could send, derived from Cyclone's vectors by the
/// rules (not by an hddsgen encoder): (label, type, value, body, whether
/// decoding must fail). See `tests/xtypes_datapath_python.rs`, which also
/// documents the `HoldUM` ones.
const DERIVED: &[(&str, &str, &str, &str, bool)] = &[
    (
        "NestApp+",
        "NestApp",
        "{ a: 1, app: { a: 2, s: \"hi\", b: 3 }, z: 4 }",
        "0100000010000000020000000300000068690003aabbccdd04000000",
        false,
    ),
    (
        "MutPrims+",
        "MutPrims",
        "{ x: 1, y: 2, z: 3 }",
        "20000000000000200100000001000020020000000200002003000000500000200a000000",
        false,
    ),
    (
        "MutPrims!",
        "MutPrims",
        "{ x: 1, y: 2, z: 3 }",
        "20000000000000200100000001000020020000000200002003000000500000a00a000000",
        true,
    ),
    (
        "HoldUM+",
        "HoldUM",
        "{ p: 1, u: { discriminator: 1, a: 10 }, z: 9 }",
        "0100000018000000000000a001000000010000200a00000005000020aabbccdd09000000",
        false,
    ),
    (
        "HoldUM-disc",
        "HoldUM",
        "{ p: 1, u: { discriminator: 1, a: 10 }, z: 9 }",
        "0100000008000000010000200a00000009000000",
        true,
    ),
    (
        "HoldUM-case",
        "HoldUM",
        "{ p: 1, u: { discriminator: 2, a: 10 }, z: 9 }",
        "0100000010000000000000a002000000010000200a00000009000000",
        true,
    ),
    (
        "HoldUM-empty",
        "HoldUM",
        "{ p: 1, u: { discriminator: 1, a: 10 }, z: 9 }",
        "010000000000000009000000",
        true,
    ),
    (
        "HoldUM!",
        "HoldUM",
        "{ p: 1, u: { discriminator: 1, a: 10 }, z: 9 }",
        "0100000018000000000000a001000000010000200a000000050000a0aabbccdd09000000",
        true,
    ),
];

const CHECKS_TS: &str = r#"
function hex(b: Uint8Array): string { return Array.from(b).map((x) => x.toString(16).padStart(2, "0")).join(""); }
function unhex(s: string): Uint8Array { const b = new Uint8Array(s.length / 2); for (let i = 0; i < b.length; i++) { b[i] = parseInt(s.slice(2 * i, 2 * i + 2), 16); } return b; }
function deq(a: any, b: any): boolean {
  if (a === b) { return true; }
  if (typeof a !== typeof b) { return false; }
  if (a instanceof Map && b instanceof Map) { return a.size === b.size && [...a].every(([k, v]) => deq(v, b.get(k))); }
  if (Array.isArray(a) && Array.isArray(b)) { return a.length === b.length && a.every((x, i) => deq(x, b[i])); }
  if (a !== null && b !== null && typeof a === "object") {
    const keys = new Set([...Object.keys(a), ...Object.keys(b)]);
    for (const k of keys) { if (!deq(a[k], b[k])) { return false; } }
    return true;
  }
  return false;
}
function show(v: any): string { return JSON.stringify(v, (_k, x) => (typeof x === "bigint" ? `${x}n` : x)); }
function chk(name: string, v: any, enc: (o: any, b: Cdr2Buffer) => void, dec: (b: Cdr2Buffer) => any, exp: string): void {
  try {
    const buf = new Cdr2Buffer(new Uint8Array(4096).fill(0xaa));
    enc(v, buf);
    const got = hex(buf.toBytes());
    console.log(got === exp ? `ENC_OK ${name}` : `ENC_FAIL ${name}\n  got ${got}\n  exp ${exp}`);
  } catch (e) { console.log(`ENC_FAIL ${name} error ${e}`); }
  try {
    const bytes = unhex(exp);
    const buf = new Cdr2Buffer(bytes);
    const d = dec(buf);
    console.log(deq(d, v) && buf.position === bytes.length ? `DEC_OK ${name}` : `DEC_FAIL ${name} used ${buf.position} of ${bytes.length}\n  got ${show(d)}`);
  } catch (e) { console.log(`DEC_FAIL ${name} error ${e}`); }
}
function decodeOnly(name: string, v: any, dec: (b: Cdr2Buffer) => any, exp: string): void {
  try {
    const bytes = unhex(exp);
    const buf = new Cdr2Buffer(bytes);
    const d = dec(buf);
    console.log(deq(d, v) && buf.position === bytes.length ? `DEC_OK ${name}` : `DEC_FAIL ${name} used ${buf.position} of ${bytes.length}\n  got ${show(d)}`);
  } catch (e) { console.log(`DEC_FAIL ${name} error ${e}`); }
}
function x1(name: string, v: any, dec: (b: Cdr1Buffer) => any, body: string, refused: boolean): void {
  try {
    const bytes = unhex(body);
    const buf = new Cdr1Buffer(bytes);
    const d = dec(buf);
    console.log(!refused && deq(d, v) && buf.position === bytes.length ? `X1_OK ${name}` : `X1_FAIL ${name} used ${buf.position} of ${bytes.length}\n  got ${show(d)}`);
  } catch (e) { console.log(refused ? `X1_OK ${name}` : `X1_FAIL ${name} error ${e}`); }
}
function x1enc(name: string, v: any, dec: (id: number, b: Uint8Array) => any, id: number, body: string, refused: boolean): void {
  try {
    const d = dec(id, unhex(body));
    console.log(!refused && deq(d, v) ? `X1_OK ${name}` : `X1_FAIL ${name} decoded ${show(d)}`);
  } catch (e) { console.log(refused ? `X1_OK ${name}` : `X1_FAIL ${name} error ${e}`); }
}
function derived(name: string, v: any, dec: (b: Cdr2Buffer) => any, body: string, refused: boolean): void {
  try {
    const bytes = unhex(body);
    const buf = new Cdr2Buffer(bytes);
    const d = dec(buf);
    console.log(!refused && deq(d, v) && buf.position === bytes.length ? `DER_OK ${name}` : `DER_FAIL ${name} decoded ${show(d)}`);
  } catch (e) { console.log(refused ? `DER_OK ${name}` : `DER_FAIL ${name} error ${e}`); }
}
"#;

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::TypeScript
        .generator()
        .generate(&ast)
        .expect("codegen")
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

/// Every IDL of this test.
fn full_idl() -> String {
    format!(
        "{CORPUS}{}{}{SCOPED_IDL}{EVOLUTION_IDL}{PLPROBE_IDL}",
        union_idl(SPEC_MUTABLE_UNION),
        union_idl(SPEC_HASHED_UNION)
    )
}

/// Types only the strict type check covers: a member the codec does not
/// serialize (`fixed`) still declares the local its decoder returns.
const TSC_ONLY_IDL: &str = "
@final struct FixedHold { fixed<5,2> f; long x; };
@mutable struct MutFixed { fixed<5,2> f; long x; };
";

/// Uses of the generated names in type positions, for `tsc`.
const TYPED_USES: &str = "
const _u: b.c.U = b.c.defaultU();
const _t: a.T = { x: 1, y: 2 };
const _k: m.K = m.K.K2;
const _m64: BM64 = BM64.BM64_0 | BM64.BM64_63;
const _m32: BM32 = BM32.BM32_0 | BM32.BM32_5;
const _h: Hold64 = { p: 1, m: _m64, h: BM31.BM31_31 };
const _d: MutDefaults = defaultMutDefaults();
const _l: LabelHold = { lu: { discriminator: m.K.K2, b: 1 }, cu: { discriminator: \"a\", a: 2 },
  bu: { discriminator: true, t: 3 }, nu: { discriminator: -1, n: 4 } };
export const typedUses = [_u, _t, _k, _m64, _m32, _h, _d, _l];
";

/// The `tsc` to type-check with: `$HDDS_TSC`, else a local install in the
/// target directory (`npm install --prefix <target>/ts typescript`), else
/// `tsc` on the PATH.
fn find_tsc() -> Option<PathBuf> {
    let runs = |p: &Path| {
        Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let local = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .map(|t| t.join("ts/node_modules/.bin/tsc"));
    std::env::var_os("HDDS_TSC")
        .map(PathBuf::from)
        .into_iter()
        .chain(local)
        .chain(std::iter::once(PathBuf::from("tsc")))
        .find(|p| runs(p))
}

/// The generated TypeScript of the whole test IDL type-checks under
/// `tsc --strict` (node strips the types without checking them). Ignored
/// by default (it needs a `tsc`); requested without one, it fails rather
/// than pass without checking anything.
#[test]
#[ignore = "needs tsc: npm install --prefix <target dir>/ts typescript (or HDDS_TSC), then cargo test --test xtypes_datapath_typescript -- --ignored"]
fn generated_typescript_passes_tsc_strict() {
    let Some(tsc) = find_tsc() else {
        panic!(
            "tsc not found: the strict type check cannot run (install it with \
             `npm install --prefix <target dir>/ts typescript`, or set HDDS_TSC)"
        );
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tsc_strict");
    fs::create_dir_all(&dir).expect("dir");
    let mut src = generate(&format!("{}{TSC_ONLY_IDL}", full_idl()));
    src.push_str(TYPED_USES);
    let file = dir.join("generated.ts");
    fs::write(&file, src).expect("source");
    let out = Command::new(&tsc)
        .args(["--strict", "--noEmit", "--target", "es2020"])
        .arg(&file)
        .output()
        .expect("tsc");
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn node_runs_typescript(dir: &Path) -> bool {
    let probe = dir.join("probe.ts");
    if fs::write(
        &probe,
        "namespace N { export enum E { A } }\nconsole.log(N.E.A);\n",
    )
    .is_err()
    {
        return false;
    }
    Command::new("node")
        .args(NODE_ARGS)
        .arg(&probe)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn typescript_backend_matches_cyclone_on_the_corpus() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_typescript");
    fs::create_dir_all(&dir).expect("dir");
    if !node_runs_typescript(&dir) {
        eprintln!(
            "node with --experimental-transform-types not found; TypeScript XCDR2 data path test not run"
        );
        return;
    }
    let mut src = generate(&full_idl());
    src.push_str(CHECKS_TS);
    for (name, ty, value) in SAMPLES {
        let _ = writeln!(
            src,
            "chk(\"{name}\", {value}, encode{ty}, decode{ty}, \"{}\");",
            xcdr2_body(name)
        );
    }
    for (fixture, name, ty, value) in MUTABLE_UNION {
        let (alt, spec) = alt_spec_bodies(fixture, name);
        let _ = writeln!(
            src,
            "chk(\"{name}\", {value}, encode{ty}, decode{ty}, \"{spec}\");"
        );
        let _ = writeln!(
            src,
            "decodeOnly(\"{name}/alt\", {value}, decode{ty}, \"{alt}\");"
        );
    }
    for (name, enc, dec, value, body) in SCOPED {
        let _ = writeln!(src, "chk(\"{name}\", {value}, {enc}, {dec}, \"{body}\");");
    }
    src.push_str(SCOPED_NAMES);
    for (name, dec, value, sample, body) in EVOLUTION {
        let body = sample.map_or_else(|| (*body).to_string(), xcdr2_body);
        let _ = writeln!(src, "decodeOnly(\"{name}\", {value}, {dec}, \"{body}\");");
    }
    let _ = writeln!(src, "const MUT_DEFAULTS = {MUT_DEFAULTS};");
    src.push_str(DEFAULTS_CHECK);
    for (name, ty, value, body, refused) in DERIVED {
        let _ = writeln!(
            src,
            "derived(\"{name}\", {value}, decode{ty}, \"{body}\", {refused});"
        );
    }
    let x1_checks = xcdr1_checks(&mut src);
    let file = dir.join("corpus.ts");
    fs::write(&file, src).expect("source");
    let out = Command::new("node")
        .args(NODE_ARGS)
        .arg(&file)
        .output()
        .expect("node");
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
            && count("DEF_OK") == 1
            && count("DER_OK") == DERIVED.len()
            && count("NAMES_OK") == 1
            && count("X1_OK") == x1_checks
            && !stdout.contains("_FAIL"),
        "{stdout}"
    );
}

/// The `spec_plprobe.txt` sample.
const PLPROBE_VALUE: &str =
    "{ id: 0x11223344, s: -2, name: \"hdds\", big: 0x0102030405060708n, tag: 0xab }";

/// The XCDR1 checks, appended to `src`; their number.
fn xcdr1_checks(src: &mut String) -> usize {
    let mut n = 0;
    let samples = SAMPLES
        .iter()
        .map(|(s, t, v)| (*s, format!("decode{t}"), *v))
        .chain(
            MUTABLE_UNION
                .iter()
                .map(|(_, s, t, v)| (*s, format!("decode{t}"), *v)),
        )
        .chain(EVOLUTION.iter().filter_map(|(_, dec, v, s, _)| {
            s.filter(|s| *s != "NestAppW")
                .map(|s| (s, (*dec).to_string(), *v))
        }));
    for (name, dec, value) in samples {
        for (label, hex) in xcdr1_bodies(name) {
            let _ = writeln!(
                src,
                "x1(\"{label}\", {value}, {dec}Cdr1, \"{hex}\", false);"
            );
            n += 1;
        }
    }
    let _ = writeln!(
        src,
        "x1(\"MutPrims/pid_ignore\", {{ x: 1, y: 2, z: 3 }}, decodeMutPrimsCdr1, \"{}\", false);",
        with_pid_ignore(&cyclone_xcdr1("MutPrims"))
    );
    n += 1;
    for p in plprobe().into_iter().filter(|p| p.xcdr1) {
        let _ = writeln!(
            src,
            "x1(\"plprobe {}\", {PLPROBE_VALUE}, decodePlProbeCdr1, \"{}\", {});",
            p.label, p.body, !p.accept
        );
        n += 1;
    }
    let v = "{ a: 1, in8: { x: 0x1122334455667788n, y: 7 }, z: 9 }";
    let (x1, x2) = (cyclone_xcdr1("NestFinal"), xcdr2_body("NestFinal"));
    for (label, id, body, refused) in [
        ("cdr_le", 0x0001, &x1, false),
        ("cdr2_le", 0x0007, &x2, false),
        ("cdr_be", 0x0000, &x1, true),
        ("cdr2_be", 0x0006, &x2, true),
    ] {
        let _ = writeln!(
            src,
            "x1enc(\"{label}\", {v}, decodeNestFinalByEncapsulation, {id}, \"{body}\", {refused});"
        );
        n += 1;
    }
    n
}

/// Types of the storage bounds: `App64` builds 64 values from its 4-byte
/// DHEADER when its member is absent (the default array), `OptBig` and
/// `UBig` up to 2^18 from one byte; `Empty` takes no byte on the wire,
/// `E1` to `E12` neither (`En` holds two `En-1`: 2^n values built); `S34`
/// nests 34 FINAL structures around a `long` (4 bytes however deep).
fn storage_idl() -> String {
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

const STORAGE_RUNNER: &str = r#"
function unhex(s: string): Uint8Array { const b = new Uint8Array(s.length / 2); for (let i = 0; i < b.length; i++) { b[i] = parseInt(s.substr(2 * i, 2), 16); } return b; }
function z(n: number): string { return "00".repeat(n); }
function kase(name: string, dec: (b: Cdr2Buffer) => any, body: string | Uint8Array, want: string | null, used?: number): void {
  const bytes = typeof body === "string" ? unhex(body) : body;
  try {
    const buf = new Cdr2Buffer(bytes);
    dec(buf);
    const exp = used === undefined ? bytes.length : used;
    console.log(want === null && buf.position === exp ? `ZW_OK ${name}` : `ZW_FAIL ${name} decoded, ${buf.position} bytes used`);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    console.log(msg === want ? `ZW_OK ${name}` : `ZW_FAIL ${name} error ${msg}`);
  }
}
const CAP = "elements of no wire size (1 values each) exceed the 1024 values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode";
function zwMsg(n: number, w: number, left: number): string {
  return `${n} elements of no wire size (${w} values each) exceed the ${left} values left of CDR_MAX_ZERO_WIRE_ELEMENTS (1024) for the decode`;
}
function seqseq(counts: number[]): Uint8Array {
  const b = new Uint8Array(8 + 8 * counts.length);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, b.length - 4, true);
  dv.setUint32(4, counts.length, true);
  counts.forEach((k, i) => { dv.setUint32(8 + 8 * i, 4, true); dv.setUint32(12 + 8 * i, k, true); });
  return b;
}
function zseq(count: number, each: number): Uint8Array {
  const b = new Uint8Array(8 + count * each);
  const dv = new DataView(b.buffer);
  dv.setUint32(0, b.length - 4, true);
  dv.setUint32(4, count, true);
  return b;
}
kase("seqempty_fits", decodeHQE, "0400000003000000", null);
kase("seqempty_count_1025", decodeHQE, "0400000001040000", `1025 ${CAP}`);
kase("seqempty_count_10m", decodeHQE, "0400000080969800", `10000000 ${CAP}`);
// 1025 elements with 200 bytes left: the values budget would allow them,
// the zero-wire budget refuses.
kase("seqempty_count_1025_left_200", decodeHQE, "0400000001040000" + z(200), `1025 ${CAP}`);
kase("seqseqempty_inner_max", decodeHQQE, "100000000200000004000000ffffffff04000000ffffffff", `4294967295 ${CAP}`);
kase("seqe10_one", decodeHQE10, "0400000001000000", null);
kase("seqe10_count_2", decodeHQE10, "0400000002000000", zwMsg(2, 1024, 1024));
// sequence<E4> of 128 (2048 values) with 1024 bytes left after the count:
// within 64 times the body, beyond the 1024 zero-wire values of the decode
// (the body does not lift them); 64 (1024 values) decode.
kase("seqe4_128_over_cap", decodeHQE4, "0400000080000000" + z(1024), zwMsg(128, 16, 1024));
kase("seqe4_64_at_cap", decodeHQE4, "0400000040000000" + z(1024), null, 8);
// sequence<App64>: one present element decodes; so do 16 absent elements
// (4 bytes each, 64 values built each: an older writer's elements), and 3
// map entries of 8 bytes, 65 values each.
kase("seqapp64_one_present", decodeHQA, "08010000" + "01000000" + "00010000" + z(256), null);
kase("seqapp64_absent_16", decodeHQA, "44000000" + "10000000" + "00000000".repeat(16), null);
kase("mapapp64_count_3", decodeHMA, "1c000000" + "03000000" + z(24), null);
// An absent optional array, a union whose only branch is the array:
// counted at their largest value, like the C and C++ holders.
kase("seqoptbig_count_16", decodeHQOB, "14000000" + "10000000" + z(16),
  "16 elements of 262144 values each exceed the 1536 values left of the decode budget");
kase("sequbig_count_16", decodeHQUB, "14000000" + "10000000" + z(16),
  "16 elements of 262145 values each exceed the 1536 values left of the decode budget");
// The budgets hold for the whole decode, not per collection: 8191 inner
// sequences of 1024 elements in 64 KiB (each within the cap) are refused
// at the second; with every inner count 0xFFFFFFFF at the first; 4 of 256
// decode, 5 of 205 do not.
kase("seqseqempty_64k_per_decode", decodeHQQE, seqseq(new Array(8191).fill(1024)), zwMsg(1024, 1, 0));
kase("seqseqempty_64k_inner_max", decodeHQQE, seqseq(new Array(8191).fill(0xffffffff)), `4294967295 ${CAP}`);
kase("seqseqempty_4x256", decodeHQQE, seqseq([256, 256, 256, 256]), null);
kase("seqseqempty_5x205", decodeHQQE, seqseq([205, 205, 205, 205, 205]), zwMsg(205, 1, 204));
// sequence<sequence<E12>> (E12: 4096 values, no byte on the wire), 8192
// inner sequences of 100 in a 65544-byte body: each passed the
// per-collection bounds of earlier releases (a heap of 1 GiB ran out);
// refused at the first inner sequence.
kase("seqseqe12_64k", decodeHQQE12, seqseq(new Array(8192).fill(100)), zwMsg(100, 4096, 1024));
// Valid data of many values per wire byte: all-absent optional members,
// empty maps, APPENDABLE elements of an older writer (a DHEADER of 0).
kase("seqoptll_absent_1", decodeHQOLL, zseq(1, 1), null);
kase("seqoptll_absent_1000", decodeHQOLL, zseq(1000, 1), null);
kase("seqmap_empty_100", decodeHQML, zseq(100, 4), null);
kase("seqapp_older_writer_1000", decodeHQV2, zseq(1000, 4), null);
function nested(outer: number, k: number): Uint8Array {
  const b = new Uint8Array(8 + outer * (12 + k));
  const dv = new DataView(b.buffer);
  let at = 8;
  for (let i = 0; i < outer; i++) {
    at += (4 - (at % 4)) % 4;
    dv.setUint32(at, 4 + k, true);
    dv.setUint32(at + 4, k, true);
    at += 8 + k;
  }
  dv.setUint32(0, at - 4, true);
  dv.setUint32(4, outer, true);
  return b.slice(0, at);
}
// Nested collections of elements that take wire bytes share the budget
// too: 100 inner sequences of one absent Opt4K (1 byte, 4096 values) in
// 1205 bytes, each within the budget alone (64 * 1205 = 77120 values), are
// refused at the nineteenth (18 * 4096 charged); 3 of 10 absent OptLL
// decode.
kase("seqseqopt4k_100", decodeHQQO4K, nested(100, 1),
  "1 elements of 4096 values each exceed the 3392 values left of the decode budget");
kase("seqseqoptll_3x10", decodeHQQOLL, nested(3, 10), null);
// 34 FINAL structures around a long take its 4 bytes however deep: 1025
// present elements decode (not counted as elements of no wire size).
{
  const s34 = new Uint8Array(8 + 4 * 1025);
  const dv = new DataView(s34.buffer);
  dv.setUint32(0, s34.length - 4, true);
  dv.setUint32(4, 1025, true);
  kase("seqs34_count_1025", decodeHQS34, s34, null);
}
"#;

/// Number of `kase` lines of [`STORAGE_RUNNER`].
const STORAGE_CASES: usize = 26;

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
fn typescript_decoder_bounds_sequence_storage() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_typescript_storage");
    fs::create_dir_all(&dir).expect("dir");
    if !node_runs_typescript(&dir) {
        eprintln!(
            "node with --experimental-transform-types not found; TypeScript storage bounds test not run"
        );
        return;
    }
    let mut src = generate(&storage_idl());
    src.push_str(STORAGE_RUNNER);
    let file = dir.join("storage.ts");
    fs::write(&file, src).expect("source");
    let mut child = Command::new("node")
        .args(NODE_ARGS)
        .arg(&file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("node");
    let start = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("TypeScript storage run still running after 60 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("output");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success()
            && stdout.lines().filter(|l| l.starts_with("ZW_OK ")).count() == STORAGE_CASES
            && !stdout.contains("_FAIL"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
