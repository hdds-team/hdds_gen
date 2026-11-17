// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Hostile bodies against the generated C++ decoder, under the address and
//! undefined-behaviour sanitizers.
//!
//! The decoder fills `std::vector` and `std::map` holders, whose storage
//! comes from `operator new`. A C++ element can be far larger than its
//! least wire form: an `@optional` array is one presence byte on the wire
//! when absent and the whole `std::optional<std::array>` in memory, a FINAL
//! union one discriminator byte and its largest branch in memory; elements
//! of an empty FINAL structure take no byte at all; and collections nest.
//! The decoder therefore charges the storage of every sequence and map of a
//! decode to one budget, the larger of `CDR_MAX_ZERO_WIRE_ELEMENTS` (1024)
//! bytes and `CDR_MAX_ALLOC_RATIO` (64) times the body, and the storage of
//! elements of least wire size 0 also to the `CDR_MAX_ZERO_WIRE_ELEMENTS`
//! bytes of the decode, refusing (`-1`, before anything is reserved or decoded) beyond
//! either (DDS-XTypes 1.3 7.4.3.5.3, spec text lines 6750-7015, for the
//! least wire sizes).
//!
//! The program replaces the global `operator new` (over `malloc`, the
//! matching `operator delete` over `free`) and sums the requests of a
//! decode: beyond that budget plus twice the body (strings, which the bytes
//! they consume bound) it prints the case as failed and exits at once (the
//! decoder is `noexcept`: a `std::bad_alloc` would terminate it, and a
//! sanitizer would abort on a 64 GiB request). The return value is checked
//! exactly: the bytes read for a body that must decode, `-1` otherwise.
//!
//! Built with `-fsanitize=address,undefined -fno-sanitize-recover=all`
//! when the compiler supports it (otherwise without, with a message), and
//! twice: with the default bounds and with
//! `-DCDR_MAX_ZERO_WIRE_ELEMENTS=2048u` (a count of 1025, or 2048 bytes
//! of elements, then decodes).
//! Each run is killed and failed after 60 s. Skipped, with a message,
//! without a C++ compiler.

#![allow(clippy::expect_used, clippy::panic)]

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hddsgen::codegen::cpp::CppGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::Parser;

const IDL: &str = "
@final struct OptMid { @optional long v[16]; };
@final struct HQOM { sequence<OptMid> q; };
@final union USmall switch (long) { case 1: long a; case 2: long long b; };
@final struct HQU { sequence<USmall> q; };
@final struct OptBig { @optional long a[262144]; };
@final struct HQOB { sequence<OptBig> q; };
@final union UBig switch (long) { case 1: long a[262144]; };
@final struct HQUB { sequence<UBig> q; };
@final struct HMOB { map<long, OptBig> m; };
@final struct Empty {};
@final struct HQE { sequence<Empty> q; };
@final struct HQQE { sequence<sequence<Empty>> q; };
@final struct Wrap { Empty e[1048576]; };
@final struct HWR { sequence<Wrap> s; };
@final struct HQ { sequence<long> q; };
@final struct OptLL { @optional long long a; };
@final struct HQOLL { sequence<OptLL> q; };
@final struct HQML { sequence<map<long, long>> q; };
@appendable struct V2 { long a[9]; };
@final struct HQV2 { sequence<V2> q; };
@final struct Opt4K { @optional long a[4096]; };
@final struct HQQO4K { sequence<sequence<Opt4K>> q; };
@final struct HQQOLL { sequence<sequence<OptLL>> q; };
@final struct WE { sequence<Empty> e; };
@final struct HQW { sequence<WE> q; };
";

/// The IDL of the cases: [`IDL`], then `E1` to `E12` (`En` holds two
/// `En-1`, `E0` being `Empty`: no byte on the wire, 2^n bytes in C++; the
/// C test goes to `E20`),
/// `D0` to `D33` (`Dn` holds one `Dn-1`, `D0` a 1 MiB array: 1 MiB on the
/// wire however deep), `S0` to `S34` (`Sn` holds one `Sn-1`, `S0` a
/// `long`: 4 bytes on the wire however deep) and their sequences.
fn idl() -> String {
    let mut s = IDL.to_string();
    for i in 1..=12 {
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
         @final struct HQE10 { sequence<E10> q; };\n\
         @final struct HQQE12 { sequence<sequence<E12>> q; };\n\
         @final struct HQD { sequence<D33> q; };\n\
         @final struct HQS34 { sequence<S34> q; };\n",
    );
    s
}

/// (label, type, body in hex, expected: `OK` for the body length, else
/// the C++ expression of the return value).
fn cases() -> Vec<(&'static str, &'static str, String, &'static str)> {
    let mut v: Vec<(&'static str, &'static str, String, &'static str)> = vec![
        // sequence<long> {5}: the bounds leave a plain body alone.
        ("seqlong_fits", "HQ", "0100000005000000".into(), "OK"),
        // sequence<OptMid>: one present element (DHEADER 72: count, flag,
        // padding, 64 bytes) decodes; so do 16 absent elements in 16 bytes
        // (16 * 68 bytes, within 64 times the 24-byte body).
        (
            "seqoptmid_one_present",
            "HQOM",
            concat!(
                "480000000100000001000000",
                "0100000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000002"
            )
            .into(),
            "OK",
        ),
        (
            "seqoptmid_absent_16",
            "HQOM",
            "140000001000000000000000000000000000000000000000".into(),
            "OK",
        ),
        // sequence<USmall> (16 bytes in C++): {a = 7} decodes; 8 elements
        // in 8 bytes get their 128 bytes and run out of bytes.
        (
            "sequnion_fits",
            "HQU",
            "0c000000010000000100000007000000".into(),
            "OK",
        ),
        (
            "sequnion_count_8",
            "HQU",
            "0c000000080000000100000007000000".into(),
            "-1",
        ),
        // 16 elements of about 1 MiB each (an absent optional array, a
        // union whose only branch is the array) with 16 bytes left: about
        // 16 MiB reserved without the bound.
        (
            "seqoptbig_count_16",
            "HQOB",
            "140000001000000000000000000000000000000000000000".into(),
            "-1",
        ),
        (
            "sequbig_count_16",
            "HQUB",
            "140000001000000000000000000000000000000000000000".into(),
            "-1",
        ),
        // map<long, OptBig>: an entry takes at least 5 bytes; 3 entries in
        // 15 bytes, each map node about 1 MiB without the bound.
        (
            "mapoptbig_count_3",
            "HMOB",
            "1300000003000000000000000000000000000000000000".into(),
            "-1",
        ),
        // sequence<Empty>: no byte on the wire. 3 elements decode; 1025
        // and 10 million are refused (zero-wire budget of the decode).
        ("seqempty_fits", "HQE", "0400000003000000".into(), "OK"),
        (
            "seqempty_count_1025",
            "HQE",
            "0400000001040000".into(),
            "ZW1025",
        ),
        ("seqempty_count_10m", "HQE", "0400000080969800".into(), "-1"),
        // 1025 elements with 200 bytes left after the sequence: the
        // storage budget would pass, the zero-wire budget refuses.
        (
            "seqempty_count_1025_left_200",
            "HQE",
            format!("0400000001040000{}", "00".repeat(200)),
            "ZW1025_LEFT",
        ),
        // sequence<sequence<Empty>>: two inner sequences, the first of
        // count 0xFFFFFFFF.
        (
            "seqseqempty_inner_max",
            "HQQE",
            "100000000200000004000000ffffffff04000000ffffffff".into(),
            "-1",
        ),
        // sequence<Wrap> (an array of 2^20 Empty: a DHEADER on the wire,
        // rule 9, 1 MiB in C++): one element in 12 bytes is refused.
        (
            "seqwrap_one",
            "HWR",
            "080000000100000000000000".into(),
            "-1",
        ),
        // No byte on the wire, 2^n bytes in C++ (E4: 16, E10: 1 KiB): the
        // storage floor is 1024 bytes with nothing left, so one E10
        // decodes, two do not, nor 1024 (1 MiB without the bound).
        ("seqe4_one", "HQE4", "0400000001000000".into(), "OK"),
        ("seqe10_one", "HQE10", "0400000001000000".into(), "OK"),
        (
            "seqe10_count_2",
            "HQE10",
            "0400000002000000".into(),
            "ZW_E10_2",
        ),
        (
            "seqe10_count_1024",
            "HQE10",
            "0400000000040000".into(),
            "-1",
        ),
        // D33: 1 MiB on the wire however deep (DEEP_BLOCK: one complete).
        ("seqdeep_truncated", "HQD", "0400000001000000".into(), "-1"),
    ];
    // sequence<E4> of 128 (2048 bytes) with 1024 bytes left after the
    // count: within 64 times the body but beyond the 1024 zero-wire bytes of
    // the decode (the body does not lift them), refused; with 2048 it
    // decodes (the sequence ends after its count: 8 bytes read).
    v.push((
        "seqe4_128_over_cap",
        "HQE4",
        format!("0400000080000000{}", "00".repeat(1024)),
        "ZW_E4_128",
    ));
    v
}

/// The budgets hold for the whole decode, not per collection. A 64 KiB
/// body of `sequence<sequence<Empty>>` with 8191 inner sequences of 1024
/// elements (each within `CDR_MAX_ZERO_WIRE_ELEMENTS`) is refused (at the
/// second inner sequence, the third with 2048); with every inner count
/// 0xFFFFFFFF at the first; 4 inner sequences of 256 (1024 in all) decode,
/// 5 of 205 (1025) only with 2048. `sequence<sequence<E12>>` (E12: 4 KiB in
/// C++, no byte on the wire) of 8192 inner sequences of 100 elements, a
/// 65544-byte body: every inner sequence passed the per-collection bounds
/// of earlier releases (734 MB requested in all); the 1024 zero-wire bytes
/// of the decode refuse the first inner sequence (400 KiB).
const ZERO_WIRE_BLOCK: &str = "        static unsigned char big[65544];
        std::size_t n = 8191u;
        std::size_t bl = 8u + 8u * n;
        put32(big, bl - 4u); put32(big + 4, n);
        for (std::size_t i = 0; i < n; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 1024u); }
        run<HQQE>(\"seqseqempty_64k_per_decode\", big, bl, -1);
        for (std::size_t i = 0; i < n; ++i) { put32(big + 12u + 8u * i, 0xFFFFFFFFu); }
        run<HQQE>(\"seqseqempty_64k_inner_max\", big, bl, -1);
        bl = 8u + 8u * 4u;
        put32(big, bl - 4u); put32(big + 4, 4u);
        for (std::size_t i = 0; i < 4u; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 256u); }
        run<HQQE>(\"seqseqempty_4x256\", big, bl, static_cast<int>(bl));
        bl = 8u + 8u * 5u;
        put32(big, bl - 4u); put32(big + 4, 5u);
        for (std::size_t i = 0; i < 5u; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 205u); }
        run<HQQE>(\"seqseqempty_5x205\", big, bl, ZW1025);
        n = 8192u;
        bl = 8u + 8u * n;
        put32(big, bl - 4u); put32(big + 4, n);
        for (std::size_t i = 0; i < n; ++i) { put32(big + 8u + 8u * i, 4u); put32(big + 12u + 8u * i, 100u); }
        run<HQQE12>(\"seqseqe12_64k\", big, bl, -1);
";

/// Valid data whose C++ storage is many times its wire size decodes within
/// the default budget: all-absent `@optional long long` elements (1 byte
/// on the wire, 16 in C++) one and 1000 of them; 100 empty
/// `map<long, long>` (4 bytes on the wire each, a `std::map` in C++); 1000
/// APPENDABLE elements of an older writer whose type had no member yet (a
/// DHEADER of 0, 36 bytes of defaulted `long a[9]` in C++).
const STORAGE_RATIO_BLOCK: &str = "        static unsigned char body[8u + 4u * 1000u];
        std::size_t bl = zseq(body, 1u, 1u);
        run<HQOLL>(\"seqoptll_absent_1\", body, bl, static_cast<int>(bl));
        bl = zseq(body, 1000u, 1u);
        run<HQOLL>(\"seqoptll_absent_1000\", body, bl, static_cast<int>(bl));
        bl = zseq(body, 100u, 4u);
        run<HQML>(\"seqmap_empty_100\", body, bl, static_cast<int>(bl));
        bl = zseq(body, 1000u, 4u);
        run<HQV2>(\"seqapp_older_writer_1000\", body, bl, static_cast<int>(bl));
";

/// Types nested deeper than 32 levels keep their least wire size:
/// `sequence<S34>` of 1025 present elements (4 bytes each, over
/// `CDR_MAX_ZERO_WIRE_ELEMENTS`) and `sequence<D33>` of one complete
/// element (1 MiB) decode.
const DEEP_BLOCK: &str = "        std::size_t bl = 8u + 4u * 1025u;
        unsigned char* body = static_cast<unsigned char*>(std::calloc(1, 8u + 4u * 262144u));
        body[0] = static_cast<unsigned char>((bl - 4u) & 0xFFu); body[1] = static_cast<unsigned char>((bl - 4u) >> 8);
        body[4] = 0x01; body[5] = 0x04;
        run<HQS34>(\"seqs34_count_1025\", body, bl, static_cast<int>(bl));
        std::memset(body, 0, 8u + 4u * 262144u);
        bl = 8u + 4u * 262144u;
        body[0] = static_cast<unsigned char>((bl - 4u) & 0xFFu); body[1] = static_cast<unsigned char>(((bl - 4u) >> 8) & 0xFFu); body[2] = static_cast<unsigned char>((bl - 4u) >> 16);
        body[4] = 1;
        run<HQD>(\"seqdeep_one_complete\", body, bl, static_cast<int>(bl));
        std::free(body);
";

/// Nested collections share the budget, through nested structures too:
/// 100 inner sequences of one absent `Opt4K` (1 byte on the wire, 16 KiB
/// in C++) in a 1205-byte body, each within the budget alone (64 times
/// the body), are refused once their sum exceeds it; 3 inner sequences of
/// 10 absent `OptLL` decode; `sequence<WE>` (`WE` a structure holding a
/// `sequence<Empty>`, decoded by its own `decode_cdr2_le_at`) with 5
/// elements of 205 Empty (1025 bytes) is refused (decodes with 2048), 4 of
/// 256 decode.
const NESTED_BLOCK: &str = "        static unsigned char body[1205];
        std::size_t bl = nested(body, 100u, 1u);
        run<HQQO4K>(\"seqseqopt4k_100\", body, bl, -1);
        bl = nested(body, 3u, 10u);
        run<HQQOLL>(\"seqseqoptll_3x10\", body, bl, static_cast<int>(bl));
        std::memset(body, 0, sizeof body);
        bl = 8u + 8u * 5u;
        put32(body, bl - 4u); put32(body + 4, 5u);
        for (std::size_t i = 0; i < 5u; ++i) { put32(body + 8u + 8u * i, 4u); put32(body + 12u + 8u * i, 205u); }
        run<HQW>(\"seqstructseqempty_5x205\", body, bl, ZW1025);
        bl = 8u + 8u * 4u;
        put32(body, bl - 4u); put32(body + 4, 4u);
        for (std::size_t i = 0; i < 4u; ++i) { put32(body + 8u + 8u * i, 4u); put32(body + 12u + 8u * i, 256u); }
        run<HQW>(\"seqstructseqempty_4x256\", body, bl, static_cast<int>(bl));
";

/// Wall-clock limit of one hostile run (sanitized).
const RUN_LIMIT: Duration = Duration::from_secs(60);

fn find_cxx() -> Option<&'static str> {
    ["c++", "g++", "clang++"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn program() -> String {
    let mut body = String::new();
    for (label, ty, hex, want) in cases() {
        let want = if want == "OK" {
            "static_cast<int>(bl)"
        } else {
            want
        };
        let _ = writeln!(
            body,
            "    {{\n        std::size_t bl = unhex(\"{hex}\", in);\n        run<{ty}>(\"{label}\", in, bl, {want});\n    }}"
        );
    }
    format!(
        "#include <cstdio>\n#include <cstdlib>\n#include <cstring>\n#include <new>\n#include \"hostile.hpp\"\n\
         #if CDR_MAX_ZERO_WIRE_ELEMENTS >= 1025u\n#define ZW1025 static_cast<int>(bl)\n#define ZW1025_LEFT 8\n#else\n#define ZW1025 (-1)\n#define ZW1025_LEFT (-1)\n#endif\n\
         #if CDR_MAX_ZERO_WIRE_ELEMENTS >= 2048u\n#define ZW_E10_2 static_cast<int>(bl)\n#define ZW_E4_128 8\n#else\n#define ZW_E10_2 (-1)\n#define ZW_E4_128 (-1)\n#endif\n\
         #if defined(__GNUC__) && !defined(__clang__)\n#pragma GCC diagnostic ignored \"-Wmismatched-new-delete\"\n#endif\n\
         static std::size_t g_limit = static_cast<std::size_t>(-1);\n\
         static std::size_t g_total = 0;\n\
         static const char* g_label = \"\";\n\
         static void note(std::size_t n) {{\n\
         \x20   if (g_limit == static_cast<std::size_t>(-1)) return;\n\
         \x20   g_total += n;\n\
         \x20   if (g_total > g_limit) {{ std::printf(\"FAIL %s requested %lu bytes (limit %lu)\\n\", g_label, static_cast<unsigned long>(g_total), static_cast<unsigned long>(g_limit)); std::fflush(stdout); std::_Exit(1); }}\n\
         }}\n\
         void* operator new(std::size_t n) {{ note(n); void* p = std::malloc(n != 0 ? n : 1); if (p == nullptr) throw std::bad_alloc(); return p; }}\n\
         void operator delete(void* p) noexcept {{ std::free(p); }}\n\
         void operator delete(void* p, std::size_t) noexcept {{ std::free(p); }}\n\
         static unsigned char in[4096];\n\
         static void put32(unsigned char* b, std::size_t v) {{ for (int k = 0; k < 4; ++k) {{ b[k] = static_cast<unsigned char>((v >> (8 * k)) & 0xFFu); }} }}\n\
         static std::size_t nested(unsigned char* b, std::size_t outer, std::size_t k) {{ std::size_t at = 8u; std::memset(b, 0, 8u); for (std::size_t i = 0; i < outer; ++i) {{ while (at % 4u != 0) {{ b[at++] = 0; }} put32(b + at, 4u + k); put32(b + at + 4u, k); std::memset(b + at + 8u, 0, k); at += 8u + k; }} put32(b, at - 4u); put32(b + 4, outer); return at; }}\n\
         static std::size_t zseq(unsigned char* b, std::size_t count, std::size_t each) {{ std::size_t bl = 8u + count * each; std::memset(b, 0, bl); put32(b, bl - 4u); put32(b + 4, count); return bl; }}\n\
         static int fails = 0;\n\
         static std::size_t unhex(const char* s, unsigned char* o) {{ std::size_t n = std::strlen(s) / 2; for (std::size_t i = 0; i < n; ++i) {{ unsigned int x = 0; std::sscanf(s + 2 * i, \"%2x\", &x); o[i] = static_cast<unsigned char>(x); }} return n; }}\n\
         template <typename T>\n\
         static void run(const char* label, const unsigned char* src, std::size_t bl, int want) {{\n\
         \x20   T* d = new T();\n\
         \x20   g_label = label;\n\
         \x20   g_total = 0;\n\
         \x20   g_limit = (CDR_MAX_ALLOC_RATIO * bl > CDR_MAX_ZERO_WIRE_ELEMENTS ? CDR_MAX_ALLOC_RATIO * bl : CDR_MAX_ZERO_WIRE_ELEMENTS) + 2u * bl;\n\
         \x20   int rc = d->decode_cdr2_le(src, bl);\n\
         \x20   g_limit = static_cast<std::size_t>(-1);\n\
         \x20   std::printf(\"REQ %s %lu\\n\", label, static_cast<unsigned long>(g_total));\n\
         \x20   delete d;\n\
         \x20   if (rc != want) {{ std::printf(\"FAIL %s rc %d want %d\\n\", label, rc, want); fails++; }}\n\
         \x20   else {{ std::printf(\"OK %s\\n\", label); }}\n\
         }}\n\
         int main() {{\n{body}\
         \x20   {{\n{ZERO_WIRE_BLOCK}    }}\n\
         \x20   {{\n{DEEP_BLOCK}    }}\n\
         \x20   {{\n{STORAGE_RATIO_BLOCK}    }}\n\
         \x20   {{\n{NESTED_BLOCK}    }}\n\
         \x20   return fails != 0 ? 1 : 0;\n}}\n"
    )
}

fn supports_asan(cxx: &str, dir: &Path) -> bool {
    let src = dir.join("probe.cpp");
    fs::write(&src, "int main() { return 0; }\n").expect("probe");
    Command::new(cxx)
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
fn cpp_decoder_refuses_hostile_bodies_within_bounds() {
    let Some(cxx) = find_cxx() else {
        eprintln!("no C++ compiler found; C++ hostile-body test not run");
        return;
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_cpp_hostile");
    fs::create_dir_all(&dir).expect("dir");
    let mut p = Parser::try_new(&idl()).expect("lexer");
    let ast = p.parse().expect("parse");
    let header = CppGenerator::new().generate(&ast).expect("codegen");
    fs::write(dir.join("hostile.hpp"), header).expect("header");
    let src = dir.join("hostile.cpp");
    fs::write(&src, program()).expect("program");
    let asan = supports_asan(cxx, &dir);
    if !asan {
        eprintln!("{cxx} has no AddressSanitizer; hostile bodies checked without it");
    }
    for (name, extra) in [
        ("default", None),
        ("zw2048", Some("-DCDR_MAX_ZERO_WIRE_ELEMENTS=2048u")),
    ] {
        let exe = dir.join(format!("hostile_{name}"));
        let mut cmd = Command::new(cxx);
        cmd.args(["-std=c++17", "-g", "-O1", "-Wall", "-Wextra", "-Werror"]);
        if asan {
            cmd.args(["-fsanitize=address,undefined", "-fno-sanitize-recover=all"]);
        }
        if let Some(d) = extra {
            cmd.arg(d);
        }
        let out = cmd.arg("-o").arg(&exe).arg(&src).output().expect("c++");
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
                && stdout.lines().filter(|l| l.starts_with("OK ")).count() == cases().len() + 15
                && !stderr.contains("Sanitizer"),
            "{name}:\n{stdout}\n{stderr}"
        );
    }
}
