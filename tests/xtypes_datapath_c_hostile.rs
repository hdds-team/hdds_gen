// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Hostile bodies against the generated C decoder, under the address
//! sanitizer (`-fsanitize=address`).
//!
//! The C holders of an unbounded string or wstring, and of the elements of
//! every sequence and map, are pointers. The decoder obtains their storage
//! from the reader's allocator (`_decode_cdr2_le_alloc`), the size computed
//! from bytes it has already bounded against the buffer; without an
//! allocator such a member is refused. Each body below is decoded into a
//! value pre-filled with 0xA5 (no pointer the caller set is ever written
//! through) with an arena whose buffer is `malloc`ed at the exact size the
//! case allows, so that the sanitizer reports a write one byte past it.
//! The return code is checked exactly: the byte count for a body that must
//! decode, the error otherwise. The allocator also sums the requests of
//! the decode: beyond the budget the decoder promises (the larger of
//! `CDR_MAX_ZERO_WIRE_ELEMENTS` (1024) bytes and `CDR_MAX_ALLOC_RATIO` (64)
//! times the body) the case fails, whatever the arena then answers (a
//! small arena alone would refuse an oversized request and mask it). The
//! decoder bounds a sequence or map count by the bytes left divided by the
//! minimum wire size of an element, then charges its storage to the budget
//! of the whole decode (`-CDR_OUT_OF_MEMORY` beyond it): the C holder of an
//! element can be far larger than its least wire form (an absent optional
//! array, a union), and collections nest. The storage of elements that take
//! no byte on the wire is also charged to the `CDR_MAX_ZERO_WIRE_ELEMENTS`
//! bytes of the whole decode (`-CDR_INVALID_DATA` beyond them).
//!
//! The delimiters (DDS-XTypes 1.3 7.4.3.5.3 rules 22 and 30, spec text
//! lines 6929 and 7013): a DHEADER claiming more than the buffer holds is
//! refused where it is read (`-CDR_UNEXPECTED_EOF`), a member whose
//! EMHEADER1 / NEXTINT claims more than its DHEADER is refused
//! (`-CDR_INVALID_DATA`), a value overrunning its DHEADER is refused
//! (`-CDR_INVALID_DATA`).
//!
//! Built with `-fsanitize=address,undefined -fno-sanitize-recover=all`
//! when the compiler supports it (otherwise without, with a message), and
//! twice: with the default `CDR_MAX_STRING` and with
//! `-DCDR_MAX_STRING=0xFFFFFFFFu` (a length near `UINT32_MAX` is then
//! refused by the buffer bound alone). Skipped, with a message, without a C
//! compiler.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hddsgen::codegen::c::CGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

const IDL: &str = "
@final struct HS { string s; };
@final struct HW { wstring w; };
@final struct HQ { sequence<long> q; };
@final struct HQS { sequence<string> q; };
@final struct Inner2 { long x; };
@final struct HQI { sequence<Inner2> q; };
@final struct HM { map<long, long> m; };
@appendable struct HA { long a; };
@final struct HNA { HA a; long z; };
@mutable struct HMu { @id(1) long a; @id(2) string s; };
@final struct Mid { long v[16]; };
@final struct HQM { sequence<Mid> q; };
@final struct Big { long v[262144]; };
@final struct HQB { sequence<Big> q; };
@final struct HMM { map<long, Mid> m; };
@final struct OptMid { @optional long v[16]; };
@final struct HQOM { sequence<OptMid> q; };
@final union USmall switch (long) { case 1: long a; case 2: long long b; };
@final struct HQU { sequence<USmall> q; };
@final struct OptBig { @optional long a[262144]; };
@final struct HQOB { sequence<OptBig> q; };
@final union UBig switch (long) { case 1: long a[262144]; };
@final struct HQUB { sequence<UBig> q; };
@final struct HMO { map<long, OptMid> m; };
@final struct Empty {};
@final struct HQE { sequence<Empty> q; };
@final struct HQQE { sequence<sequence<Empty>> q; };
@final struct Wrap { Empty e[1048576]; };
@final struct HWR { sequence<Wrap> s; };
@final struct OptLL { @optional long long a; };
@final struct HQOLL { sequence<OptLL> q; };
@final struct HQML { sequence<map<long, long>> q; };
@appendable struct V2 { long a[9]; };
@final struct HQV2 { sequence<V2> q; };
@final struct Opt4K { @optional long a[4096]; };
@final struct HQQO4K { sequence<sequence<Opt4K>> q; };
@final struct HQQOLL { sequence<sequence<OptLL>> q; };
";

/// The IDL of the cases: [`IDL`], then `E1` to `E20` (`En` holds two `En-1`,
/// `E0` being `Empty`: no byte on the wire, 2^n bytes in C), `D0` to `D33`
/// (`Dn` holds one `Dn-1`, `D0` a 1 MiB array: 1 MiB on the wire however
/// deep), `S0` to `S34` (`Sn` holds one `Sn-1`, `S0` a `long`: 4 bytes on
/// the wire however deep) and their sequences (C needs a type declared
/// before its use).
fn idl() -> String {
    let mut s = IDL.to_string();
    for i in 1..=20 {
        let inner = if i == 1 {
            "Empty".to_string()
        } else {
            format!("E{}", i - 1)
        };
        let _ = writeln!(s, "@final struct E{i} {{ {inner} a; {inner} b; }};");
    }
    s.push_str("@final struct D0 { long a[262144]; };\n");
    for i in 1..=33 {
        let _ = writeln!(s, "@final struct D{i} {{ D{} x; }};", i - 1);
    }
    s.push_str("@final struct S0 { long a; };\n");
    for i in 1..=34 {
        let _ = writeln!(s, "@final struct S{i} {{ S{} x; }};", i - 1);
    }
    s.push_str(
        "@final struct HQE4 { sequence<E4> q; };\n\
         @final struct HQE20 { sequence<E20> q; };\n\
         @final struct HQQE12 { sequence<sequence<E12>> q; };\n\
         @final struct HQD { sequence<D33> q; };\n\
         @final struct HQS34 { sequence<S34> q; };\n",
    );
    s
}

/// Besides these, the program calls the runtime's `hdds_x2c_ralloc`
/// directly with a count whose product by the element size overflows
/// `size_t` (unreachable through a 64-bit decode, whose counts are bounded
/// by the buffer first): it must be refused, not wrapped.
///
/// (label, C type, body hex, arena bytes as a C expression or `NULL` for
/// no allocator, expected return: `OK` for the whole body consumed, else
/// the error expression).
const CASES: &[(&str, &str, &str, &str, &str)] = &[
    // string "abcdefgh": 9 bytes of storage, not 8; none without allocator.
    ("str_fits", "HS", "09000000616263646566676800", "9", "OK"),
    // string "abc" whose last byte is not the NUL its length counts
    // (7.4.3.5.3 rule 3).
    (
        "str_no_nul",
        "HS",
        "03000000616263",
        "64",
        "-CDR_INVALID_DATA",
    ),
    (
        "str_arena_short",
        "HS",
        "09000000616263646566676800",
        "8",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "str_no_alloc",
        "HS",
        "09000000616263646566676800",
        "NULL",
        "-CDR_OUT_OF_MEMORY",
    ),
    // string length near UINT32_MAX / beyond the bytes left.
    ("str_len_max", "HS", "ffffffff61626364", "64", "HUGE_RC"),
    ("str_len_max_m1", "HS", "feffffff61626364", "64", "HUGE_RC"),
    (
        "str_len_past_end",
        "HS",
        "64000000616263",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // wstring "ab" (UTF-16): 3 wchar_t of storage with the NUL.
    (
        "wstr_fits",
        "HW",
        "0400000061006200",
        "3 * sizeof(wchar_t)",
        "OK",
    ),
    (
        "wstr_arena_short",
        "HW",
        "0400000061006200",
        "3 * sizeof(wchar_t) - 1",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "wstr_no_alloc",
        "HW",
        "0400000061006200",
        "NULL",
        "-CDR_OUT_OF_MEMORY",
    ),
    ("wstr_len_max", "HW", "feffffff61006200", "64", "HUGE_RC"),
    (
        "wstr_len_odd",
        "HW",
        "03000000610062",
        "64",
        "-CDR_INVALID_DATA",
    ),
    // sequence<long> {1, 2}.
    (
        "seq_fits",
        "HQ",
        "020000000100000002000000",
        "2 * sizeof(int32_t)",
        "OK",
    ),
    (
        "seq_arena_short",
        "HQ",
        "020000000100000002000000",
        "2 * sizeof(int32_t) - 1",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "seq_no_alloc",
        "HQ",
        "020000000100000002000000",
        "NULL",
        "-CDR_OUT_OF_MEMORY",
    ),
    ("seq_empty_no_alloc", "HQ", "00000000", "NULL", "OK"),
    (
        "seq_count_max",
        "HQ",
        "ffffffff01000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    (
        "seq_count_2p30",
        "HQ",
        "0100004001000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // sequence<string> {"a"} (DHEADER 10): the array, then the string at
    // the next 16-byte boundary of the arena.
    (
        "seqstr_fits",
        "HQS",
        "0a00000001000000020000006100",
        "16 + 2",
        "OK",
    ),
    (
        "seqstr_arena_short",
        "HQS",
        "0a00000001000000020000006100",
        "16 + 1",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "seqstr_count_max",
        "HQS",
        "08000000ffffffff02000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    (
        "seqstr_dheader_max",
        "HQS",
        "ffffff7f01000000020000006100",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // sequence<Inner2> {{7}}: an element takes at least 4 bytes.
    (
        "seqstruct_fits",
        "HQI",
        "080000000100000007000000",
        "sizeof(Inner2)",
        "OK",
    ),
    (
        "seqstruct_arena_short",
        "HQI",
        "080000000100000007000000",
        "sizeof(Inner2) - 1",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "seqstruct_count_max",
        "HQI",
        "08000000ffffffff07000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // sequence<Mid> (64 bytes on the wire per element, DHEADER first):
    // one element decodes; a count of 2 or of 64 with 64 bytes left cannot
    // be held and is refused before any storage is requested.
    (
        "seqmid_fits",
        "HQM",
        concat!(
            "4400000001000000",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000002"
        ),
        "sizeof(Mid)",
        "OK",
    ),
    (
        "seqmid_count_2",
        "HQM",
        concat!(
            "4400000002000000",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000002"
        ),
        "2 * sizeof(Mid)",
        "-CDR_UNEXPECTED_EOF",
    ),
    (
        "seqmid_count_64",
        "HQM",
        concat!(
            "4400000040000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // sequence<Big> (1 MiB per element): 16 elements announced with 16
    // bytes left; the old one-byte bound requested 16 MiB.
    (
        "seqbig_count_16",
        "HQB",
        "140000001000000000000000000000000000000000000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // map<long, Mid>: an entry takes at least 68 bytes; 16 entries with 64
    // bytes left.
    (
        "mapmid_count_16",
        "HMM",
        concat!(
            "4400000010000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000000"
        ),
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // sequence<OptMid>: an element takes one byte on the wire when its
    // optional array is absent, 68 bytes in C. One present element
    // (DHEADER 72: count, flag, padding, 64 bytes) decodes; so do 16
    // absent elements in 16 bytes (1088 bytes of storage, within 64 times
    // the 24-byte body).
    (
        "seqoptmid_one_present",
        "HQOM",
        concat!(
            "480000000100000001000000",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000002"
        ),
        "sizeof(OptMid)",
        "OK",
    ),
    (
        "seqoptmid_absent_16",
        "HQOM",
        "140000001000000000000000000000000000000000000000",
        "16 * sizeof(OptMid)",
        "OK",
    ),
    // sequence<USmall> (a FINAL union: one byte of least wire size, 16
    // bytes in C): {a = 7} decodes; 8 elements in 8 bytes get their 128
    // bytes (within the budget) and run out of bytes at the second.
    (
        "sequnion_fits",
        "HQU",
        "0c000000010000000100000007000000",
        "sizeof(USmall)",
        "OK",
    ),
    (
        "sequnion_count_8",
        "HQU",
        "0c000000080000000100000007000000",
        "2048",
        "-CDR_UNEXPECTED_EOF",
    ),
    // 16 elements of about 1 MiB each (an absent optional array, a union
    // whose only branch is the array) with 16 bytes left: the count bound
    // (one byte each) passes, the request (about 16 MiB) is over the
    // budget (64 times the 24-byte body) and refused.
    (
        "seqoptbig_count_16",
        "HQOB",
        "140000001000000000000000000000000000000000000000",
        "2048",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "sequbig_count_16",
        "HQUB",
        "140000001000000000000000000000000000000000000000",
        "2048",
        "-CDR_OUT_OF_MEMORY",
    ),
    // map<long, OptMid>: an entry takes at least 5 bytes; 3 entries in 15
    // bytes pass the count bound, the values get their 204 bytes, and the
    // third key (aligned at 4) is past the end.
    (
        "mapoptmid_count_3",
        "HMO",
        "1300000003000000000000000000000000000000000000",
        "2048",
        "-CDR_UNEXPECTED_EOF",
    ),
    // sequence<Empty>: an element takes no byte on the wire, so the bytes
    // left bound nothing; the decode is refused beyond
    // CDR_MAX_ZERO_WIRE_ELEMENTS (1024) bytes of such elements (one each).
    // 3 elements decode; 1025 and 10 million are refused.
    ("seqempty_fits", "HQE", "0400000003000000", "3", "OK"),
    // 1024 Empty in an 8-byte body: beyond 64 times the body, within the
    // CDR_MAX_ZERO_WIRE_ELEMENTS bytes the budget always grants.
    (
        "seqempty_count_1024",
        "HQE",
        "0400000000040000",
        "1024",
        "OK",
    ),
    (
        "seqempty_count_1025",
        "HQE",
        "0400000001040000",
        "2048",
        "-CDR_INVALID_DATA",
    ),
    (
        "seqempty_count_10m",
        "HQE",
        "0400000080969800",
        "2048",
        "-CDR_INVALID_DATA",
    ),
    // sequence<sequence<Empty>>: two inner sequences, the first of count
    // 0xFFFFFFFF.
    (
        "seqseqempty_inner_max",
        "HQQE",
        "100000000200000004000000ffffffff04000000ffffffff",
        "2048",
        "-CDR_INVALID_DATA",
    ),
    // sequence<Wrap> (Wrap: an array of 2^20 Empty, 1 MiB in C): the array
    // carries a DHEADER (rule 9), so an element takes at least 4 bytes and
    // the storage request (1 MiB for one element in 12 bytes) is over the
    // budget and refused.
    (
        "seqwrap_one",
        "HWR",
        "080000000100000000000000",
        "2048",
        "-CDR_OUT_OF_MEMORY",
    ),
    // Elements of no byte on the wire but 2^n bytes in C (E4: 16, E20:
    // 1 MiB): their storage is refused beyond the CDR_MAX_ZERO_WIRE_ELEMENTS
    // bytes of the decode, before any is requested (otherwise 1 GiB asked
    // from 8 bytes, and a decode walking 2^30 empty structures).
    ("seqe4_one", "HQE4", "0400000001000000", "16", "OK"),
    (
        "seqe20_one",
        "HQE20",
        "0400000001000000",
        "2048",
        "-CDR_INVALID_DATA",
    ),
    (
        "seqe20_count_1024",
        "HQE20",
        "0400000000040000",
        "2048",
        "-CDR_INVALID_DATA",
    ),
    // D33 takes its 1 MiB on the wire however deep it nests: one element
    // announced with no byte left is refused before any storage is
    // requested (a complete one decodes, DEEP_BLOCK).
    (
        "seqdeep_one_truncated",
        "HQD",
        "0400000001000000",
        "2048",
        "-CDR_UNEXPECTED_EOF",
    ),
    // map<long, long> {1: 2}: keys, then values at the next boundary.
    (
        "map_fits",
        "HM",
        "010000000100000002000000",
        "16 + sizeof(int32_t)",
        "OK",
    ),
    (
        "map_arena_short",
        "HM",
        "010000000100000002000000",
        "16 + sizeof(int32_t) - 1",
        "-CDR_OUT_OF_MEMORY",
    ),
    (
        "map_count_max",
        "HM",
        "ffffffff0100000002000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
    // HNA { HA { a = 1 }, z = 2 }: DHEADER 4; 0x100 (past the buffer); 2
    // (the member overruns it).
    ("app_ok", "HNA", "040000000100000002000000", "NULL", "OK"),
    (
        "app_dheader_past_end",
        "HNA",
        "000100000100000002000000",
        "NULL",
        "-CDR_UNEXPECTED_EOF",
    ),
    (
        "app_dheader_overrun",
        "HNA",
        "020000000100000002000000",
        "NULL",
        "-CDR_INVALID_DATA",
    ),
    // HMu { a = 1, s = "x" }: DHEADER 18, a (LC 2), s (LC 5).
    (
        "mut_ok",
        "HMu",
        "12000000010000200100000002000050020000007800",
        "2",
        "OK",
    ),
    // a with LC 4 and a NEXTINT of 0x100 inside a DHEADER of 12.
    (
        "mut_nextint_past_dheader",
        "HMu",
        "0c000000010000400001000001000000",
        "64",
        "-CDR_INVALID_DATA",
    ),
    // s with LC 5 and a string length of 0x100 inside a DHEADER of 12.
    (
        "mut_lc5_past_dheader",
        "HMu",
        "0c000000020000500001000078000000",
        "64",
        "-CDR_INVALID_DATA",
    ),
    (
        "mut_dheader_past_end",
        "HMu",
        "000100000100002001000000",
        "64",
        "-CDR_UNEXPECTED_EOF",
    ),
];

/// The budgets hold for the whole decode, not per collection. A 64 KiB
/// body of `sequence<sequence<Empty>>` with 8191 inner sequences of 1024
/// elements each (each within `CDR_MAX_ZERO_WIRE_ELEMENTS`) is refused at
/// the second; with every inner count 0xFFFFFFFF at the first; 4 inner
/// sequences of 256 (1024 in all) decode, 5 of 205 (1025) do not.
/// `sequence<sequence<E12>>` (E12: 4 KiB in C, no byte on the wire) of
/// 8192 inner sequences of 100 elements, a 65544-byte body: every inner
/// sequence passed the bytes-left bound and the per-collection caps of
/// earlier releases (1024 elements, 8 times the bytes left: 400 KiB each),
/// 734 MB asked in all; the 1024 zero-wire bytes of the decode refuse the
/// first inner sequence (400 KiB), only the outer holder (128 KiB) is
/// requested.
const ZERO_WIRE_BLOCK: &str = "        static uint8_t big[65544];
        size_t n = (sizeof big - 8u) / 8u;
        size_t bl = 8u + 8u * n;
        size_t i;
        size_t hs = budget(sizeof big);
        uint8_t* hb = (uint8_t*)malloc(hs);
        HQQE d;
        HQQE12 d12;
        hdds_x2c_arena ar;
        int rc;
        n = 8191u;
        bl = 8u + 8u * n;
        memset(big, 0, sizeof big);
        put32(big, (uint32_t)(bl - 4u)); put32(big + 4, (uint32_t)n);
        for (i = 0; i < n; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 1024u); }
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqe_decode_cdr2_le_alloc(&d, big, bl, count_alloc, &ar);
        check(\"seqseqempty_64k_per_decode\", rc, -CDR_INVALID_DATA, bl);
        for (i = 0; i < n; ++i) { put32(big + 12u + 8u * i, 0xFFFFFFFFu); }
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqe_decode_cdr2_le_alloc(&d, big, bl, count_alloc, &ar);
        check(\"seqseqempty_64k_inner_max\", rc, -CDR_INVALID_DATA, bl);
        bl = 8u + 8u * 4u;
        put32(big, (uint32_t)(bl - 4u)); put32(big + 4, 4u);
        for (i = 0; i < 4u; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 256u); }
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqe_decode_cdr2_le_alloc(&d, big, bl, count_alloc, &ar);
        check(\"seqseqempty_4x256\", rc, (int)bl, bl);
        bl = 8u + 8u * 5u;
        put32(big, (uint32_t)(bl - 4u)); put32(big + 4, 5u);
        for (i = 0; i < 5u; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 205u); }
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqe_decode_cdr2_le_alloc(&d, big, bl, count_alloc, &ar);
        check(\"seqseqempty_5x205\", rc, -CDR_INVALID_DATA, bl);
        n = 8192u;
        bl = 8u + 8u * n;
        put32(big, (uint32_t)(bl - 4u)); put32(big + 4, (uint32_t)n);
        for (i = 0; i < n; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 100u); }
        memset(&d12, 0xA5, sizeof d12);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqe12_decode_cdr2_le_alloc(&d12, big, bl, count_alloc, &ar);
        printf(\"seqseqe12_64k requested %lu bytes\\n\", (unsigned long)max_req);
        check(\"seqseqe12_64k\", rc, -CDR_INVALID_DATA, bl);
        free(hb);
";

/// Valid data whose C storage is many times its wire size decodes within
/// the default budget: all-absent `@optional long long` elements (1 byte
/// on the wire, 16 in C) one and 1000 of them; 100 empty `map<long, long>`
/// (4 bytes on the wire each, 24 in C); 1000 APPENDABLE elements of an
/// older writer whose type had no member yet (a DHEADER of 0, 36 bytes of
/// defaulted `long a[9]` in C).
const STORAGE_RATIO_BLOCK: &str = "        static uint8_t body[8u + 4u * 1000u];
        size_t bl;
        size_t hs = budget(sizeof body);
        uint8_t* hb = (uint8_t*)malloc(hs);
        HQOLL d1;
        HQML d2;
        HQV2 d3;
        hdds_x2c_arena ar;
        int rc;
        bl = zseq(body, 1u, 1u);
        memset(&d1, 0xA5, sizeof d1);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqoll_decode_cdr2_le_alloc(&d1, body, bl, count_alloc, &ar);
        check(\"seqoptll_absent_1\", rc, (int)bl, bl);
        bl = zseq(body, 1000u, 1u);
        memset(&d1, 0xA5, sizeof d1);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqoll_decode_cdr2_le_alloc(&d1, body, bl, count_alloc, &ar);
        check(\"seqoptll_absent_1000\", rc, (int)bl, bl);
        bl = zseq(body, 100u, 4u);
        memset(&d2, 0xA5, sizeof d2);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqml_decode_cdr2_le_alloc(&d2, body, bl, count_alloc, &ar);
        check(\"seqmap_empty_100\", rc, (int)bl, bl);
        bl = zseq(body, 1000u, 4u);
        memset(&d3, 0xA5, sizeof d3);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqv2_decode_cdr2_le_alloc(&d3, body, bl, count_alloc, &ar);
        check(\"seqapp_older_writer_1000\", rc, (int)bl, bl);
        free(hb);
";

/// `sequence<E4>` (16 bytes in C, none on the wire) of 128 elements with
/// 1024 bytes left after the count: 2048 bytes, within
/// `CDR_MAX_ALLOC_RATIO` times the body but beyond the
/// `CDR_MAX_ZERO_WIRE_ELEMENTS` bytes of the decode, are refused (the bytes
/// of the body do not lift the zero-wire cap: they bound nothing of such
/// elements); 64 of them (1024 bytes) decode (the sequence ends after its
/// count; the trailing bytes only count as body).
const ZERO_WIRE_RATIO_BLOCK: &str = "        static uint8_t body[1032];
        size_t bl = sizeof body;
        size_t hs = 128u * sizeof(E4);
        uint8_t* hb = (uint8_t*)malloc(hs);
        HQE4 d;
        hdds_x2c_arena ar;
        int rc;
        body[0] = 4; body[4] = 128;
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqe4_decode_cdr2_le_alloc(&d, body, bl, count_alloc, &ar);
        check(\"seqe4_128_over_cap\", rc, -CDR_INVALID_DATA, bl);
        body[4] = 64;
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqe4_decode_cdr2_le_alloc(&d, body, bl, count_alloc, &ar);
        check(\"seqe4_64_at_cap\", rc, 8, bl);
        free(hb);
";

/// Types nested deeper than 32 levels keep their least wire size:
/// `sequence<S34>` of 1025 present elements (4 bytes each, over
/// `CDR_MAX_ZERO_WIRE_ELEMENTS`) and `sequence<D33>` of one complete
/// element (1 MiB) decode.
const DEEP_BLOCK: &str = "        size_t bl = 8u + 4u * 1025u;
        uint8_t* body = (uint8_t*)calloc(1, bl);
        size_t hs = 1025u * sizeof(S34);
        uint8_t* hb = (uint8_t*)malloc(hs);
        HQS34 d;
        HQD dd;
        hdds_x2c_arena ar;
        int rc;
        body[0] = (uint8_t)((bl - 4u) & 0xFFu); body[1] = (uint8_t)((bl - 4u) >> 8);
        body[4] = 0x01; body[5] = 0x04;
        body[bl - 4u] = 7;
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqs34_decode_cdr2_le_alloc(&d, body, bl, count_alloc, &ar);
        if (rc == (int)bl && d.q.len != 1025u) { rc = -1000; }
        if (rc == (int)bl && d.q.data[1024].x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.x.a != 7) { rc = -1001; }
        check(\"seqs34_count_1025\", rc, (int)bl, bl);
        free(hb);
        free(body);
        bl = 8u + 4u * 262144u;
        body = (uint8_t*)calloc(1, bl);
        hs = sizeof(D33);
        hb = (uint8_t*)malloc(hs);
        body[0] = (uint8_t)((bl - 4u) & 0xFFu); body[1] = (uint8_t)(((bl - 4u) >> 8) & 0xFFu); body[2] = (uint8_t)((bl - 4u) >> 16);
        body[4] = 1;
        memset(&dd, 0xA5, sizeof dd);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqd_decode_cdr2_le_alloc(&dd, body, bl, count_alloc, &ar);
        check(\"seqdeep_one_complete\", rc, (int)bl, bl);
        free(hb);
        free(body);
";

/// Nested collections of elements that take wire bytes share the budget
/// too: 100 inner sequences of one absent `Opt4K` (1 byte on the wire,
/// 16 KiB in C) in a 1205-byte body, each inner request within the budget
/// alone (64 times the body: 77120 bytes), are refused once their sum
/// exceeds it; 3 inner sequences of 10 absent `OptLL` decode.
const NESTED_BLOCK: &str = "        static uint8_t body[1205];
        size_t bl;
        size_t hs = budget(sizeof body);
        uint8_t* hb = (uint8_t*)malloc(hs);
        HQQO4K d;
        HQQOLL d2;
        hdds_x2c_arena ar;
        int rc;
        bl = nested(body, 100u, 1u);
        memset(&d, 0xA5, sizeof d);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqo4k_decode_cdr2_le_alloc(&d, body, bl, count_alloc, &ar);
        check(\"seqseqopt4k_100\", rc, -CDR_OUT_OF_MEMORY, bl);
        bl = nested(body, 3u, 10u);
        memset(&d2, 0xA5, sizeof d2);
        hdds_x2c_arena_init(&ar, hb, hs);
        max_req = 0;
        rc = hqqoll_decode_cdr2_le_alloc(&d2, body, bl, count_alloc, &ar);
        check(\"seqseqoptll_3x10\", rc, (int)bl, bl);
        free(hb);
";

/// Wall-clock limit of one hostile run (sanitized): the 64 KiB
/// zero-wire body must not turn into a CPU sink.
const RUN_LIMIT: Duration = Duration::from_secs(60);

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn program() -> String {
    let mut body = String::new();
    for (label, ty, hex, arena, want) in CASES {
        let f = ty.to_ascii_lowercase();
        let (alloc, actx, size) = if *arena == "NULL" {
            ("NULL", "NULL", "0".to_string())
        } else {
            ("count_alloc", "&ar", format!("(size_t)({arena})"))
        };
        let want = if *want == "OK" { "(int)bl" } else { want };
        let _ = writeln!(
            body,
            "    {{\n        {ty} d;\n        size_t bl = unhex(\"{hex}\", in);\n        size_t hs = {size};\n        uint8_t* hb = hs ? (uint8_t*)malloc(hs) : NULL;\n        hdds_x2c_arena ar;\n        int rc;\n        memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, hb, hs);\n        max_req = 0;\n        rc = {f}_decode_cdr2_le_alloc(&d, in, bl, {alloc}, {actx});\n        check(\"{label}\", rc, {want}, bl);\n        (void)ar;\n        free(hb);\n    }}"
        );
    }
    format!(
        "#include <stdio.h>\n#include <stdlib.h>\n#include \"hostile.h\"\n\
         #if CDR_MAX_STRING < 0xFFFFFFFFu\n#define HUGE_RC (-CDR_INVALID_DATA)\n#else\n#define HUGE_RC (-CDR_UNEXPECTED_EOF)\n#endif\n\
         static uint8_t in[512];\n\
         static int fails = 0;\n\
         static size_t unhex(const char* s, uint8_t* o) {{ size_t n = strlen(s) / 2; size_t i; unsigned int x; for (i = 0; i < n; ++i) {{ sscanf(s + 2 * i, \"%2x\", &x); o[i] = (uint8_t)x; }} return n; }}\n\
         static size_t max_req = 0;\n\
         static void* count_alloc(void* ctx, size_t n) {{ max_req += n; return hdds_x2c_arena_alloc(ctx, n); }}\n\
         static size_t budget(size_t bl) {{ return 64u * bl > 1024u ? 64u * bl : 1024u; }}\n\
         static void put32(uint8_t* b, uint32_t v) {{ b[0] = (uint8_t)(v & 0xFFu); b[1] = (uint8_t)((v >> 8) & 0xFFu); b[2] = (uint8_t)((v >> 16) & 0xFFu); b[3] = (uint8_t)(v >> 24); }}\n\
         static size_t nested(uint8_t* b, uint32_t outer, uint32_t k) {{ size_t at = 8u; uint32_t i; memset(b, 0, 8u); for (i = 0; i < outer; ++i) {{ while (at % 4u != 0) {{ b[at++] = 0; }} put32(b + at, 4u + k); put32(b + at + 4u, k); memset(b + at + 8u, 0, k); at += 8u + k; }} put32(b, (uint32_t)(at - 4u)); put32(b + 4, outer); return at; }}\n\
         static size_t zseq(uint8_t* b, uint32_t count, size_t each) {{ size_t bl = 8u + count * each; memset(b, 0, bl); put32(b, (uint32_t)(bl - 4u)); put32(b + 4, count); return bl; }}\n\
         static void check(const char* label, int rc, int want, size_t bl) {{\n\
         \x20   if (max_req > budget(bl)) {{ printf(\"FAIL %s requested %lu bytes for a %lu-byte body\\n\", label, (unsigned long)max_req, (unsigned long)bl); fails++; }}\n\
         \x20   else if (rc != want) {{ printf(\"FAIL %s rc %d want %d\\n\", label, rc, want); fails++; }}\n\
         \x20   else {{ printf(\"OK %s\\n\", label); }}\n\
         }}\n\
         static uint8_t spare[64];\n\
         static void* spare_alloc(void* ctx, size_t n) {{ (void)ctx; (void)n; return spare; }}\n\
         int main(void) {{\n{body}\
         \x20   {{\n{ZERO_WIRE_BLOCK}    }}\n\
         \x20   {{\n{ZERO_WIRE_RATIO_BLOCK}    }}\n\
         \x20   {{\n{DEEP_BLOCK}    }}\n\
         \x20   {{\n{STORAGE_RATIO_BLOCK}    }}\n\
         \x20   {{\n{NESTED_BLOCK}    }}\n\
         \x20   {{\n        hdds_x2c_r c;\n        void* p;\n        memset(&c, 0, sizeof c);\n        c.alloc = spare_alloc;\n\
         \x20       max_req = 0;\n\
         \x20       check(\"ralloc_size_overflow\", hdds_x2c_ralloc(&c, ((size_t)-1) / 4u + 2u, 4u, &p), -CDR_OUT_OF_MEMORY, 0u);\n    }}\n\
         \x20   return fails ? 1 : 0;\n}}\n"
    )
}

fn supports_asan(cc: &str, dir: &Path) -> bool {
    let src = dir.join("probe.c");
    fs::write(&src, "int main(void) { return 0; }\n").expect("probe");
    Command::new(cc)
        .args(["-fsanitize=address,undefined", "-o"])
        .arg(dir.join("probe"))
        .arg(&src)
        .output()
        .is_ok_and(|o| o.status.success())
        && Command::new(dir.join("probe"))
            .output()
            .is_ok_and(|o| o.status.success())
}

#[test]
fn c_decoder_refuses_hostile_bodies_without_overflow() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C hostile-body test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_c_hostile");
    fs::create_dir_all(&dir).expect("dir");
    let mut p = Parser::try_new(&idl()).expect("lexer");
    let ast = p.parse().expect("parse");
    let header = CGenerator::new().generate(&ast).expect("codegen");
    fs::write(dir.join("hostile.h"), header).expect("header");
    let src = dir.join("hostile.c");
    fs::write(&src, program()).expect("program");
    let asan = supports_asan(cc, &dir);
    if !asan {
        eprintln!("{cc} has no AddressSanitizer; hostile bodies checked without it");
    }
    for (name, extra) in [
        ("default", None),
        ("maxstr", Some("-DCDR_MAX_STRING=0xFFFFFFFFu")),
    ] {
        let exe = dir.join(format!("hostile_{name}"));
        let mut cmd = Command::new(cc);
        cmd.args(["-std=c99", "-g", "-Wall", "-Wextra", "-Werror"]);
        if asan {
            cmd.args(["-fsanitize=address,undefined", "-fno-sanitize-recover=all"]);
        }
        if let Some(d) = extra {
            cmd.arg(d);
        }
        let out = cmd.arg("-o").arg(&exe).arg(&src).output().expect("cc");
        assert!(
            out.status.success(),
            "{name}: compile failed\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut child = Command::new(&exe)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run");
        let start = Instant::now();
        while child.try_wait().expect("wait").is_none() {
            if start.elapsed() > RUN_LIMIT {
                let _ = child.kill();
                panic!("{name}: still running after {RUN_LIMIT:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let run = child.wait_with_output().expect("output");
        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
        assert!(
            run.status.success()
                && stdout.lines().filter(|l| l.starts_with("OK ")).count() == CASES.len() + 16
                && !stderr.contains("Sanitizer"),
            "{name}:\n{stdout}\n{stderr}"
        );
    }
}
