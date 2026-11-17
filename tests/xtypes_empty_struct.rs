// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! An empty FINAL structure, alone and as the element of a sequence, in
//! the generated C, C-micro and C++ code.
//!
//! ISO C has no empty structure: the C and C-micro holders carry a one-byte
//! placeholder the codec never serializes. The C++ entry points of an empty
//! structure use none of their parameters. Each program is compiled under
//! the strict flags of the data-path test of its backend (C: `-std=c89`,
//! `-std=c99`, `-std=c11`, each `-pedantic-errors -Wall -Wextra -Werror`;
//! C-micro: `-std=c99 -pedantic-errors -Wall -Wextra -Werror`; C++:
//! `-std=c++17 -Wall -Wextra -Werror -O1`, both C++ flavours).
//!
//! Expected bytes (XCDR2 bodies, no encapsulation header):
//!
//! - `Empty` alone: zero bytes (DDS-XTypes 1.3 7.4.3.5.3 rule 17, spec text
//!   lines 6887-6891: a FINAL structure is its members in order). Cyclone
//!   DDS 0.11's `idlc` emits no type nor topic descriptor for an empty
//!   structure (`#if 0 /* empty struct */`), so it gives no vector for it.
//! - `EmptyHolder { q = 3 x Empty, z = 0x11223344 }` and the same with an
//!   empty `q`: from Cyclone DDS 0.11 (`idlc` on this IDL, the element type
//!   given a one-byte definition since `idlc` omits it, then
//!   `dds_stream_write_sampleLE` with XCDR2; the operations of `Empty` are a
//!   lone `DDS_OP_RTS`): a DHEADER (sequence of non-primitive elements,
//!   rule 12), the count, no element byte, then `z`.
//!
//! Skipped, with a message, without a C or C++ compiler.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::codegen::c::CGenerator;
use hddsgen::codegen::cpp::CppGenerator;
use hddsgen::codegen::{CStandard, CodeGenerator};
use hddsgen::{Backend, Parser};

const IDL: &str =
    "@final struct Empty {};\n@final struct EmptyHolder { sequence<Empty> q; long z; };\n";

/// Cyclone DDS 0.11, `EmptyHolder` with three elements.
const HOLDER3: &str = "040000000300000044332211";
/// Cyclone DDS 0.11, `EmptyHolder` with no element.
const HOLDER0: &str = "040000000000000044332211";

fn find(tools: &[&'static str]) -> Option<&'static str> {
    tools
        .iter()
        .copied()
        .find(|cc| Command::new(cc).arg("--version").output().is_ok())
}

fn ast() -> hddsgen::ast::IdlFile {
    let mut p = Parser::try_new(IDL).expect("lexer");
    p.parse().expect("parse")
}

fn dir(name: &str) -> std::path::PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("xtypes_empty_struct")
        .join(name);
    fs::create_dir_all(&d).expect("dir");
    d
}

/// Compiles `src` with `args`, runs it, and checks it printed `checks`
/// lines `OK ...` and no `FAIL`.
fn build_and_run(tool: &str, d: &Path, name: &str, args: &[&str], src: &str, checks: usize) {
    let file = d.join(format!(
        "{name}.{}",
        if tool.contains("++") { "cpp" } else { "c" }
    ));
    fs::write(&file, src).expect("program");
    let exe = d.join(name);
    let out = Command::new(tool)
        .args(args)
        .arg("-o")
        .arg(&exe)
        .arg(&file)
        .output()
        .expect("compiler");
    assert!(
        out.status.success(),
        "{name}: compile failed\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&exe).output().expect("run");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success()
            && !stdout.contains("FAIL")
            && stdout.lines().filter(|l| l.starts_with("OK ")).count() == checks,
        "{name}:\n{stdout}"
    );
}

/// The C helpers shared by the C and C-micro programs.
const C_HELPERS: &str = "#include <stdio.h>\n#include <string.h>\n\
static int fails = 0;\n\
static unsigned int unhex(const char* s, uint8_t* o) { unsigned int n = (unsigned int)(strlen(s) / 2); unsigned int i; unsigned int x; for (i = 0; i < n; ++i) { sscanf(s + 2 * i, \"%2x\", &x); o[i] = (uint8_t)x; } return n; }\n\
static void ok(const char* label, int cond) { if (cond) { printf(\"OK %s\\n\", label); } else { printf(\"FAIL %s\\n\", label); fails++; } }\n";

fn c_program() -> String {
    format!(
        "#include \"empty.h\"\n{C_HELPERS}\
int main(void) {{\n\
    uint8_t in[64];\n\
    uint8_t out[64];\n\
    uint8_t ab[256];\n\
    unsigned int bl;\n\
    int rc;\n\
    Empty e[3];\n\
    Empty one;\n\
    EmptyHolder h;\n\
    EmptyHolder d;\n\
    hdds_x2c_arena ar;\n\
    memset(e, 0, sizeof e);\n\
    memset(&one, 0, sizeof one);\n\
    ok(\"empty_enc\", empty_encode_cdr2_le(&one, out, sizeof out) == 0);\n\
    ok(\"empty_dec\", empty_decode_cdr2_le(&one, in, 0) == 0);\n\
    h.q.data = e; h.q.len = 3; h.z = 0x11223344;\n\
    bl = unhex(\"{HOLDER3}\", in);\n\
    rc = emptyholder_encode_cdr2_le(&h, out, sizeof out);\n\
    ok(\"holder3_enc\", rc == (int)bl && memcmp(out, in, bl) == 0);\n\
    memset(&d, 0xA5, sizeof d);\n\
    hdds_x2c_arena_init(&ar, ab, sizeof ab);\n\
    rc = emptyholder_decode_cdr2_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
    ok(\"holder3_dec\", rc == (int)bl && d.q.len == 3 && d.z == 0x11223344);\n\
    h.q.len = 0;\n\
    bl = unhex(\"{HOLDER0}\", in);\n\
    rc = emptyholder_encode_cdr2_le(&h, out, sizeof out);\n\
    ok(\"holder0_enc\", rc == (int)bl && memcmp(out, in, bl) == 0);\n\
    memset(&d, 0xA5, sizeof d);\n\
    rc = emptyholder_decode_cdr2_le(&d, in, bl);\n\
    ok(\"holder0_dec\", rc == (int)bl && d.q.len == 0 && d.z == 0x11223344);\n\
    return fails ? 1 : 0;\n\
}}\n"
    )
}

fn micro_program() -> String {
    format!(
        "#include \"empty.h\"\n{C_HELPERS}\
int main(void) {{\n\
    uint8_t in[64];\n\
    uint8_t out[64];\n\
    unsigned int bl;\n\
    int32_t rc;\n\
    hdds_cdr_t c;\n\
    Empty one;\n\
    EmptyHolder h;\n\
    EmptyHolder d;\n\
    memset(&one, 0, sizeof one);\n\
    memset(&h, 0, sizeof h);\n\
    hdds_cdr_init(&c, out, sizeof out);\n\
    rc = empty_encode(&one, &c);\n\
    ok(\"empty_enc\", rc == HDDS_CDR_OK && c.pos == 0);\n\
    hdds_cdr_init(&c, in, 0);\n\
    rc = empty_decode(&one, &c);\n\
    ok(\"empty_dec\", rc == HDDS_CDR_OK && c.pos == 0);\n\
    h.q.count = 3; h.z = 0x11223344;\n\
    bl = unhex(\"{HOLDER3}\", in);\n\
    hdds_cdr_init(&c, out, sizeof out);\n\
    rc = empty_holder_encode(&h, &c);\n\
    ok(\"holder3_enc\", rc == HDDS_CDR_OK && c.pos == bl && memcmp(out, in, bl) == 0);\n\
    memset(&d, 0xA5, sizeof d);\n\
    hdds_cdr_init(&c, in, bl);\n\
    rc = empty_holder_decode(&d, &c);\n\
    ok(\"holder3_dec\", rc == HDDS_CDR_OK && c.pos == bl && d.q.count == 3 && d.z == 0x11223344);\n\
    h.q.count = 0;\n\
    bl = unhex(\"{HOLDER0}\", in);\n\
    hdds_cdr_init(&c, out, sizeof out);\n\
    rc = empty_holder_encode(&h, &c);\n\
    ok(\"holder0_enc\", rc == HDDS_CDR_OK && c.pos == bl && memcmp(out, in, bl) == 0);\n\
    memset(&d, 0xA5, sizeof d);\n\
    hdds_cdr_init(&c, in, bl);\n\
    rc = empty_holder_decode(&d, &c);\n\
    ok(\"holder0_dec\", rc == HDDS_CDR_OK && c.pos == bl && d.q.count == 0 && d.z == 0x11223344);\n\
    return fails ? 1 : 0;\n\
}}\n"
    )
}

fn cpp_program(run: bool) -> String {
    if !run {
        return "#include \"empty.hpp\"\nint main() { return 0; }\n".to_string();
    }
    format!(
        "#include \"empty.hpp\"\n#include <cstdio>\n#include <cstring>\n\
static int fails = 0;\n\
static unsigned unhex(const char* s, std::uint8_t* o) {{ unsigned n = static_cast<unsigned>(std::strlen(s) / 2); for (unsigned i = 0; i < n; ++i) {{ unsigned x = 0; std::sscanf(s + 2 * i, \"%2x\", &x); o[i] = static_cast<std::uint8_t>(x); }} return n; }}\n\
static void ok(const char* label, bool cond) {{ if (cond) {{ std::printf(\"OK %s\\n\", label); }} else {{ std::printf(\"FAIL %s\\n\", label); fails++; }} }}\n\
int main() {{\n\
    std::uint8_t in[64];\n\
    std::uint8_t out[64];\n\
    Empty one;\n\
    ok(\"empty_enc\", one.encode_cdr2_le(out, sizeof out) == 0);\n\
    ok(\"empty_dec\", one.decode_cdr2_le(in, 0) == 0);\n\
    EmptyHolder h;\n\
    h.q.resize(3); h.z = 0x11223344;\n\
    unsigned bl = unhex(\"{HOLDER3}\", in);\n\
    int rc = h.encode_cdr2_le(out, sizeof out);\n\
    ok(\"holder3_enc\", rc == static_cast<int>(bl) && std::memcmp(out, in, bl) == 0);\n\
    EmptyHolder d;\n\
    rc = d.decode_cdr2_le(in, bl);\n\
    ok(\"holder3_dec\", rc == static_cast<int>(bl) && d.q.size() == 3 && d.z == 0x11223344);\n\
    h.q.clear();\n\
    bl = unhex(\"{HOLDER0}\", in);\n\
    rc = h.encode_cdr2_le(out, sizeof out);\n\
    ok(\"holder0_enc\", rc == static_cast<int>(bl) && std::memcmp(out, in, bl) == 0);\n\
    EmptyHolder d0;\n\
    rc = d0.decode_cdr2_le(in, bl);\n\
    ok(\"holder0_dec\", rc == static_cast<int>(bl) && d0.q.empty() && d0.z == 0x11223344);\n\
    return fails ? 1 : 0;\n\
}}\n"
    )
}

#[test]
fn c_empty_struct_compiles_strictly_and_round_trips() {
    let Some(cc) = find(&["cc", "gcc", "clang"]) else {
        eprintln!("no C compiler found; C empty-struct test not run");
        return;
    };
    for (std, standard) in [
        ("c89", CStandard::C89),
        ("c99", CStandard::C99),
        ("c11", CStandard::C11),
    ] {
        let d = dir(&format!("c_{std}"));
        let header = CGenerator::with_standard(standard)
            .generate(&ast())
            .expect("codegen");
        fs::write(d.join("empty.h"), header).expect("header");
        let flag = format!("-std={std}");
        build_and_run(
            cc,
            &d,
            "empty",
            &[&flag, "-pedantic-errors", "-Wall", "-Wextra", "-Werror"],
            &c_program(),
            6,
        );
    }
}

#[test]
fn c_micro_empty_struct_compiles_strictly_and_round_trips() {
    let Some(cc) = find(&["cc", "gcc", "clang"]) else {
        eprintln!("no C compiler found; C-micro empty-struct test not run");
        return;
    };
    let d = dir("c_micro");
    let header = Backend::CMicro
        .generator()
        .generate(&ast())
        .expect("codegen");
    fs::write(d.join("empty.h"), header).expect("header");
    let include = Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/c-micro/include");
    let include = include.to_str().expect("utf-8 path");
    build_and_run(
        cc,
        &d,
        "empty",
        &[
            "-std=c99",
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
            include,
        ],
        &micro_program(),
        6,
    );
}

#[test]
fn cpp_empty_struct_compiles_strictly_and_round_trips() {
    let Some(cxx) = find(&["c++", "g++", "clang++"]) else {
        eprintln!("no C++ compiler found; C++ empty-struct test not run");
        return;
    };
    for (name, generator, run, checks) in [
        ("plain", CppGenerator::new(), true, 6),
        (
            "fastdds_compat",
            CppGenerator::with_fastdds_compat(),
            false,
            0,
        ),
    ] {
        let d = dir(&format!("cpp_{name}"));
        fs::write(
            d.join("empty.hpp"),
            generator.generate(&ast()).expect("codegen"),
        )
        .expect("header");
        build_and_run(
            cxx,
            &d,
            "empty",
            &["-std=c++17", "-Wall", "-Wextra", "-Werror", "-O1"],
            &cpp_program(run),
            checks,
        );
    }
}
