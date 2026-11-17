// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Module scoping of the generated C and C-micro code.
//!
//! C has one namespace: a type declared in a module is named with its full
//! module path (`a::T` is `a_T`, `m::inner::N` is `m_inner_N`), and every
//! reference is resolved with the IDL 4.2 scoping rules (an absolute name
//! `::m::inner::N`; a relative name looked up from the innermost enclosing
//! module outwards, so `T` inside module `b` is `b::T`, not the top-level
//! `T`). Two modules each declaring `struct T` with different members give
//! two types; a structure holding `a::T`, `b::T`, the top-level `T` and
//! types of a nested module from outside it is encoded and decoded.
//!
//! Expected bytes, written by hand from DDS-XTypes 1.3 7.4.3.5.3 (FINAL
//! structures: members in order, alignment min(size, 4) from the start of
//! the body, zero padding; an enumeration 4 bytes): `Both` with
//! ta = {1, 2}, tb = {3, 4}, n = {5, 6}, e = E2, ul.t = {7, 8},
//! ui = {{9, 10}, E1}, tt = {9}, dv = {base {11, 12}, z 13},
//! na = {14, 15}, un = (E1) {16, 17}:
//!
//! ```text
//! 01 000000 | 02000000        a::T  (octet, pad, long)
//! 0300 0000 | 0400000000000000 b::T (short, pad, long long at 4)
//! 05 000000 | 06000000        m::inner::N
//! 02000000                    m::inner::E = E2
//! 0700 0000 | 0800000000000000 b::UseLocal.t (b::T)
//! 09 000000 | 0a000000 | 01000000  m::UseInner { inner::N, inner::E = E1 }
//! 0900000000000000            ::T (long long)
//! 0b 000000 | 0c000000 | 0d000000  m::Derived : inner::N (base first)
//! 0e 000000 | 0f000000        m::NAlias (typedef of inner::N)
//! 01000000 | 10 000000 | 11000000  m::U, discriminator inner::E = E1
//! ```
//!
//! Skipped, with a message, without a C compiler.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::codegen::c::CGenerator;
use hddsgen::codegen::CodeGenerator;
use hddsgen::{Backend, Parser};

const IDL: &str = "
@final struct T { long long w; };
module a { @final struct T { octet x; long y; }; };
module b {
  @final struct T { short p; long long q; };
  @final struct UseLocal { T t; };
};
module m {
  module inner {
    @final struct N { octet k; long v; };
    enum E { E0, E1, E2 };
  };
  @final struct UseInner { inner::N n; inner::E e; };
  @final struct Derived : inner::N { long z; };
  typedef inner::N NAlias;
  @final union U switch (inner::E) { case inner::E1: inner::N n; case inner::E2: long l; };
};
@final struct Both { a::T ta; b::T tb; ::m::inner::N n; m::inner::E e; b::UseLocal ul; m::UseInner ui; T tt;
  m::Derived dv; m::NAlias na; m::U un; };
";

const BOTH: &str = "0100000002000000030000000400000000000000050000000600000002000000070000000800000000000000090000000a000000010000000900000000000000\
                    0b0000000c0000000d0000000e0000000f000000010000001000000011000000";
const A_T: &str = "0100000002000000";
const B_T: &str = "030000000400000000000000";

const FILL: &str = "v.ta.x = 1; v.ta.y = 2; v.tb.p = 3; v.tb.q = 4; v.n.k = 5; v.n.v = 6; v.e = M_INNER_E_E2; \
                    v.ul.t.p = 7; v.ul.t.q = 8; v.ui.n.k = 9; v.ui.n.v = 10; v.ui.e = M_INNER_E_E1; v.tt.w = 9; \
                    v.dv.base.k = 11; v.dv.base.v = 12; v.dv.z = 13; v.na.k = 14; v.na.v = 15; \
                    v.un._d = M_INNER_E_E1; v.un._u.n.k = 16; v.un._u.n.v = 17;";
const CHECK: &str = "d.ta.x == 1 && d.ta.y == 2 && d.tb.p == 3 && d.tb.q == 4 && d.n.k == 5 && d.n.v == 6 \
                     && d.e == M_INNER_E_E2 && d.ul.t.p == 7 && d.ul.t.q == 8 && d.ui.n.k == 9 && d.ui.n.v == 10 \
                     && d.ui.e == M_INNER_E_E1 && d.tt.w == 9 && d.dv.base.k == 11 && d.dv.base.v == 12 \
                     && d.dv.z == 13 && d.na.k == 14 && d.na.v == 15 && d.un._d == M_INNER_E_E1 \
                     && d.un._u.n.k == 16 && d.un._u.n.v == 17";

const HELPERS: &str = "#include <stdio.h>\n\
static char got[512];\n\
static int fails = 0;\n\
static void hexs(const uint8_t* b, unsigned n) { unsigned i; got[0] = 0; for (i = 0; i < n; ++i) { sprintf(got + 2 * i, \"%02x\", b[i]); } }\n\
static void expect(const char* label, int ok) { if (ok) { printf(\"OK %s\\n\", label); } else { printf(\"FAIL %s %s\\n\", label, got); fails++; } }\n";

fn find_cc() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn parse(idl: &str) -> hddsgen::ast::IdlFile {
    let mut p = Parser::try_new(idl).expect("lexer");
    p.parse().expect("parse")
}

fn build_and_run(
    cc: &str,
    dir: &Path,
    header: &str,
    program: &str,
    include: Option<&Path>,
) -> String {
    fs::create_dir_all(dir).expect("dir");
    fs::write(dir.join("modules.h"), header).expect("header");
    let src = dir.join("modules.c");
    fs::write(&src, program).expect("program");
    let exe = dir.join("modules");
    let mut cmd = Command::new(cc);
    cmd.args([
        "-std=c99",
        "-pedantic-errors",
        "-Wall",
        "-Wextra",
        "-Werror",
    ]);
    if let Some(i) = include {
        cmd.arg("-I").arg(i);
    }
    let out = cmd.arg("-o").arg(&exe).arg(&src).output().expect("cc");
    assert!(
        out.status.success(),
        "compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(run.status.success() && !stdout.contains("FAIL"), "{stdout}");
    stdout
}

#[test]
fn c_types_of_two_modules_stay_apart() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C module scoping test not run");
        return;
    };
    let header = CGenerator::new().generate(&parse(IDL)).expect("codegen");
    let program = format!(
        "#include \"modules.h\"\n{HELPERS}\
         int main(void) {{\n\
         \x20   uint8_t out[256];\n\
         \x20   Both v; Both d; a_T at; b_T bt; int n;\n\
         \x20   memset(&v, 0, sizeof v);\n\
         \x20   {FILL}\n\
         \x20   n = both_encode_cdr2_le(&v, out, sizeof out); hexs(out, n < 0 ? 0 : (unsigned)n);\n\
         \x20   expect(\"Both encode\", strcmp(got, \"{BOTH}\") == 0);\n\
         \x20   memset(&d, 0xA5, sizeof d);\n\
         \x20   n = both_decode_cdr2_le(&d, out, sizeof(\"{BOTH}\") / 2);\n\
         \x20   expect(\"Both decode\", n == (int)(sizeof(\"{BOTH}\") / 2) && {CHECK});\n\
         \x20   at.x = 1; at.y = 2; n = a_t_encode_cdr2_le(&at, out, sizeof out); hexs(out, n < 0 ? 0 : (unsigned)n);\n\
         \x20   expect(\"a::T\", strcmp(got, \"{A_T}\") == 0);\n\
         \x20   bt.p = 3; bt.q = 4; n = b_t_encode_cdr2_le(&bt, out, sizeof out); hexs(out, n < 0 ? 0 : (unsigned)n);\n\
         \x20   expect(\"b::T\", strcmp(got, \"{B_T}\") == 0);\n\
         \x20   expect(\"type name\", strcmp(m_inner_n_type_name(), \"m::inner::N\") == 0 && strcmp(t_type_name(), \"T\") == 0\
         \x20       && strcmp(both_fields[0].type_name, \"a::T\") == 0 && strcmp(both_fields[2].type_name, \"m::inner::N\") == 0);\n\
         \x20   return fails ? 1 : 0;\n\
         }}\n"
    );
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_c_modules");
    let out = build_and_run(cc, &dir, &header, &program, None);
    assert_eq!(
        out.lines().filter(|l| l.starts_with("OK ")).count(),
        5,
        "{out}"
    );
}

#[test]
fn c_micro_types_of_two_modules_stay_apart() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C-micro module scoping test not run");
        return;
    };
    let header = Backend::CMicro
        .generator()
        .generate(&parse(IDL))
        .expect("codegen");
    let program = format!(
        "#include \"modules.h\"\n{HELPERS}\
         int main(void) {{\n\
         \x20   uint8_t out[256];\n\
         \x20   hdds_cdr_t c;\n\
         \x20   Both v; Both d; a_T at; b_T bt; int rc;\n\
         \x20   memset(&v, 0, sizeof v);\n\
         \x20   {FILL}\n\
         \x20   hdds_cdr_init(&c, out, sizeof out); rc = both_encode(&v, &c); hexs(out, rc ? 0 : c.pos);\n\
         \x20   expect(\"Both encode\", strcmp(got, \"{BOTH}\") == 0);\n\
         \x20   memset(&d, 0xA5, sizeof d);\n\
         \x20   hdds_cdr_init(&c, out, (uint32_t)(sizeof(\"{BOTH}\") / 2)); rc = both_decode(&d, &c);\n\
         \x20   expect(\"Both decode\", rc == 0 && c.pos == sizeof(\"{BOTH}\") / 2 && {CHECK});\n\
         \x20   at.x = 1; at.y = 2; hdds_cdr_init(&c, out, sizeof out); rc = a__t_encode(&at, &c); hexs(out, rc ? 0 : c.pos);\n\
         \x20   expect(\"a::T\", strcmp(got, \"{A_T}\") == 0);\n\
         \x20   bt.p = 3; bt.q = 4; hdds_cdr_init(&c, out, sizeof out); rc = b__t_encode(&bt, &c); hexs(out, rc ? 0 : c.pos);\n\
         \x20   expect(\"b::T\", strcmp(got, \"{B_T}\") == 0);\n\
         \x20   return fails ? 1 : 0;\n\
         }}\n"
    );
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_c_micro_modules");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let out = build_and_run(cc, &dir, &header, &program, Some(&include));
    assert_eq!(
        out.lines().filter(|l| l.starts_with("OK ")).count(),
        4,
        "{out}"
    );
}

/// A one-byte enumeration declared in a nested module, used as a member
/// and as a union discriminator from the enclosing module, in the C-micro
/// backend (its type, member and function names all come from one naming
/// function). Expected bytes from Cyclone DDS 0.11 (`idlc -l c`, then
/// `dds_stream_write_sampleLE` under XCDR2 into a buffer pre-filled with
/// 0xff): `HoldS { e = S2, u = (S1) a = 0x11223344, z = 7 }` is
/// `02 01 0000 44332211 07` (enum 1 byte, discriminator 1 byte, padding to
/// 4, long, octet); `{ e = S0, u = (S2) b = 0x55, z = 9 }` is `00 02 55 09`.
const SCOPED_ENUM_IDL: &str = "
module p {
  module q { @bit_bound(8) enum Small { S0, S1, S2 }; };
  @final union SU switch (q::Small) { case q::S1: long a; case q::S2: octet b; };
  @final struct HoldS { q::Small e; SU u; octet z; };
};
";
const HOLDS_A: &str = "020100004433221107";
const HOLDS_B: &str = "00025509";

#[test]
fn c_micro_scoped_enum_as_member_and_discriminator() {
    let Some(cc) = find_cc() else {
        eprintln!("no C compiler found; C-micro scoped enumeration test not run");
        return;
    };
    let header = Backend::CMicro
        .generator()
        .generate(&parse(SCOPED_ENUM_IDL))
        .expect("codegen");
    let program = format!(
        "#include \"modules.h\"\n{HELPERS}\
         static int roundtrip(const char* label, const p_HoldS* v, const char* hex) {{\n\
         \x20   uint8_t out[64]; hdds_cdr_t c; p_HoldS d; int rc; unsigned n = (unsigned)(strlen(hex) / 2);\n\
         \x20   memset(out, 0xEE, sizeof out);\n\
         \x20   hdds_cdr_init(&c, out, sizeof out); rc = p__hold_s_encode(v, &c); hexs(out, rc ? 0 : c.pos);\n\
         \x20   expect(label, strcmp(got, hex) == 0);\n\
         \x20   memset(&d, 0xA5, sizeof d);\n\
         \x20   hdds_cdr_init(&c, out, n); rc = p__hold_s_decode(&d, &c);\n\
         \x20   return rc == 0 && c.pos == n && d.e == v->e && d.u._d == v->u._d && d.z == v->z\n\
         \x20       && (v->u._d == P_Q_SMALL_S1 ? d.u._u.a == v->u._u.a : d.u._u.b == v->u._u.b);\n\
         }}\n\
         int main(void) {{\n\
         \x20   p_HoldS v;\n\
         \x20   p_q_Small e = P_Q_SMALL_S2;\n\
         \x20   p_SU u;\n\
         \x20   memset(&v, 0, sizeof v); memset(&u, 0, sizeof u);\n\
         \x20   v.e = e; v.u._d = P_Q_SMALL_S1; v.u._u.a = 0x11223344; v.z = 7;\n\
         \x20   expect(\"HoldS S1 decode\", roundtrip(\"HoldS S1 encode\", &v, \"{HOLDS_A}\"));\n\
         \x20   v.e = P_Q_SMALL_S0; v.u._d = P_Q_SMALL_S2; v.u._u.b = 0x55; v.z = 9;\n\
         \x20   expect(\"HoldS S2 decode\", roundtrip(\"HoldS S2 encode\", &v, \"{HOLDS_B}\"));\n\
         \x20   (void)u;\n\
         \x20   return fails ? 1 : 0;\n\
         }}\n"
    );
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtypes_datapath_c_micro_scoped_enum");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let out = build_and_run(cc, &dir, &header, &program, Some(&include));
    assert_eq!(
        out.lines().filter(|l| l.starts_with("OK ")).count(),
        4,
        "{out}"
    );
}

/// Two IDL names with the same C name are refused by both backends,
/// never merged into one type.
#[test]
fn c_name_collisions_are_refused() {
    let idl = "module a { struct b_T { long x; }; }; module a_b { struct T { long y; }; };";
    let ast = parse(idl);
    for err in [
        CGenerator::new().generate(&ast).expect_err("c").to_string(),
        Backend::CMicro
            .generator()
            .generate(&ast)
            .expect_err("c-micro")
            .to_string(),
    ] {
        assert!(
            err.contains("`a::b_T` and `a_b::T` both map to the C name `a_b_T`"),
            "{err}"
        );
    }
}
