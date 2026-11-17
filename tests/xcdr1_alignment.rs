// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! XCDR1 alignment of 8-byte elements (`long long`, `double`) in sequences
//! and arrays, in every backend that reads or writes XCDR1.
//!
//! DDS-XTypes 1.3 7.4.1 (`PLAIN_CDR`, version 1, `MAXALIGN` = 8): a
//! primitive is aligned to its own size, 8-byte ones to 8, from the first
//! byte of the serialized body (after the 4-byte encapsulation header); a
//! sequence is a 4-byte length and its elements, each aligned when it is
//! written, so an empty sequence pads nothing; an array has no length word.
//! Version 2 (7.4.3.1) caps the alignment at 4: the two layouts differ
//! exactly where an 8-byte element does not already sit on a multiple of 8.
//!
//! The expected bytes are laid out by hand from that rule (the comment of
//! each case gives the offsets); they are not produced by any hddsgen
//! encoder. The XCDR2 form of the same value is the long-verified one
//! (`xtypes_datapath_*`, Cyclone DDS vectors), so a backend that only
//! decodes XCDR1 is checked by decoding the XCDR1 bytes and encoding the
//! result as XCDR2, which must give the XCDR2 bytes.
//!
//! The generated codecs of every backend are little endian. The only
//! big-endian XCDR1 the generators read is the key-only payload of the Rust
//! backend, checked in `rust_key_payload_...` below.
//!
//! The Rust and micro tests need an HDDS checkout (`HDDSGEN_HDDS_DIR`) and
//! are `#[ignore]`d: `HDDSGEN_HDDS_DIR=<hdds> cargo test --test
//! xcdr1_alignment -- --ignored`.

#![allow(clippy::expect_used, clippy::panic, clippy::too_many_lines)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, Parser};

const IDL: &str = "
@final struct SeqO { octet o; sequence<double> s; };
@final struct ArrL { long l; double d[2]; };
@final struct SeqN { long a; long b; sequence<double> s; sequence<long long> t; };
@final struct SeqE { long a; long b; sequence<double> s; short z; };
typedef double Dbl;
@final struct SeqA { long a; long b; sequence<Dbl, 4> s; };
@final struct Cell { octet c; double v; };
@final struct Nest { octet o; Cell n; sequence<Cell> si; };
@final struct SeqSeq { long a; sequence<sequence<double> > ss; };
@final struct Mat { long l; unsigned long long m[2][2]; };
";

/// One value, its XCDR1 and XCDR2 little-endian bodies (hex), and the value
/// in Rust and in micro syntax.
struct Case {
    name: &'static str,
    x1: &'static str,
    x2: &'static str,
    rust: &'static str,
    micro: &'static str,
}

const CASES: &[Case] = &[
    // o at 0, length at 4 (3 pad), the first double at 8 in both versions:
    // the length word ends on a multiple of 8, so this alone cannot tell
    // the versions apart.
    Case {
        name: "SeqO",
        x1: "0100000002000000000000000000f83f00000000000000c0",
        x2: "0100000002000000000000000000f83f00000000000000c0",
        rust: "SeqO { o: 1, s: vec![1.5, -2.0] }",
        micro: "SeqO { o: 1, s: V(&[1.5, -2.0]) }",
    },
    // l at 0, d[0] at 8 (XCDR1, 4 pad) or at 4 (XCDR2).
    Case {
        name: "ArrL",
        x1: "0700000000000000000000000000f83f00000000000000c0",
        x2: "07000000000000000000f83f00000000000000c0",
        rust: "ArrL { l: 7, d: [1.5, -2.0] }",
        micro: "ArrL { l: 7, d: [1.5, -2.0] }",
    },
    // a 0, b 4, length 8..12, s at 16 (XCDR1, 4 pad) or 12; t length at 32
    // (XCDR1) or 28, t[0] at 40 (4 pad) or 32.
    Case {
        name: "SeqN",
        x1: "01000000020000000200000000000000000000000000f83f00000000000000c001000000000000000300000000000000",
        x2: "010000000200000002000000000000000000f83f00000000000000c0010000000300000000000000",
        rust: "SeqN { a: 1, b: 2, s: vec![1.5, -2.0], t: vec![3] }",
        micro: "SeqN { a: 1, b: 2, s: V(&[1.5, -2.0]), t: V(&[3]) }",
    },
    // An empty sequence aligns nothing: z follows the length word.
    Case {
        name: "SeqE",
        x1: "0100000002000000000000000500",
        x2: "0100000002000000000000000500",
        rust: "SeqE { a: 1, b: 2, s: vec![], z: 5 }",
        micro: "SeqE { a: 1, b: 2, s: V(&[]), z: 5 }",
    },
    // A bounded sequence of an alias of double: length 8..12, element at 16
    // (XCDR1) or 12.
    Case {
        name: "SeqA",
        x1: "0100000002000000010000000000000000000000000000c0",
        x2: "01000000020000000100000000000000000000c0",
        rust: "SeqA { a: 1, b: 2, s: vec![-2.0] }",
        micro: "SeqA { a: 1, b: 2, s: V(&[-2.0]) }",
    },
    // o 0, n.c 1, n.v at 8 (XCDR1) or 4, si length at 16 / 12, si[0].c at
    // 20 / 16, si[0].v at 24 / 20: the alignment of a structure is that of
    // its members, counted from the start of the body. XCDR2 puts a DHEADER
    // (16) before the length of a sequence of non-primitive elements.
    Case {
        name: "Nest",
        x1: "0901000000000000000000000000f83f010000000200000000000000000000c0",
        x2: "09010000000000000000f83f10000000010000000200000000000000000000c0",
        rust: "Nest { o: 9, n: Cell { c: 1, v: 1.5 }, si: vec![Cell { c: 2, v: -2.0 }] }",
        micro: "Nest { o: 9, n: Cell { c: 1, v: 1.5 }, si: V(&[Cell { c: 2, v: -2.0 }]) }",
    },
    // a 0, outer length 4, inner length 8, first element at 16 (XCDR1) or
    // 12; second inner length at 24 / 20, its element at 32 / 24. XCDR2
    // puts a DHEADER (28) before the outer length, the inner sequences
    // being of a primitive type have none.
    Case {
        name: "SeqSeq",
        x1: "07000000020000000100000000000000000000000000f83f010000000000000000000000000000c0",
        x2: "070000001c0000000200000001000000000000000000f83f0100000000000000000000c0",
        rust: "SeqSeq { a: 7, ss: vec![vec![1.5], vec![-2.0]] }",
        micro: "SeqSeq { a: 7, ss: V(&[V(&[1.5]), V(&[-2.0])]) }",
    },
    // A two-dimensional array: m at 8 (XCDR1) or 4, no length word.
    Case {
        name: "Mat",
        x1: "07000000000000000100000000000000020000000000000003000000000000000400000000000000",
        x2: "070000000100000000000000020000000000000003000000000000000400000000000000",
        rust: "Mat { l: 7, m: [[1, 2], [3, 4]] }",
        micro: "Mat { l: 7, m: [[1, 2], [3, 4]] }",
    },
];

fn generate(backend: &Backend, idl: &str) -> String {
    let ast = Parser::try_new(idl).expect("lexer").parse().expect("IDL");
    backend.generator().generate(&ast).expect("generate")
}

fn work_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("xcdr1_alignment")
        .join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    dir
}

fn tool_exists(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{cmd:?}\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// Every case printed `OK <name>` and nothing printed `FAIL`.
fn all_ok(out: &str) {
    let ok = out.lines().filter(|l| l.starts_with("OK ")).count();
    assert!(
        !out.contains("FAIL") && ok == CASES.len(),
        "expected {} OK lines\n{out}",
        CASES.len()
    );
}

fn hdds_dir(krate: &str) -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        dir.join("crates").join(krate).join("Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={:?} must name an HDDS checkout (crates/{krate}): \
         these tests did NOT run",
        dir.display().to_string()
    );
    dir
}

fn run_crate(name: &str, deps: &str, files: &[(&str, String)]) -> String {
    let root = work_dir(name);
    fs::create_dir_all(root.join("src")).expect("crate dir");
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [dependencies]\n{deps}\n[workspace]\n"
    );
    fs::write(root.join("Cargo.toml"), manifest).expect("manifest");
    for (file, content) in files {
        fs::write(root.join("src").join(file), content).expect("source");
    }
    run(Command::new(env!("CARGO"))
        .args(["run", "--offline", "--quiet"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("target")))
}

// ---------------------------------------------------------------- C

#[test]
fn c_decodes_xcdr1_with_8_byte_elements_at_8() {
    let Some(cc) = ["cc", "gcc", "clang"].into_iter().find(|c| tool_exists(c)) else {
        eprintln!("no C compiler found; test not run");
        return;
    };
    let mut body = String::new();
    for c in CASES {
        let f = c.name.to_ascii_lowercase();
        let _ = writeln!(
            body,
            "    {{\n        {n} d;\n        int r;\n        size_t bl = unhex(\"{x1}\", in);\n\
             \x20       memset(&d, 0xA5, sizeof d);\n        hdds_x2c_arena_init(&ar, heap, sizeof heap);\n\
             \x20       r = {f}_decode_cdr1_le_alloc(&d, in, bl, hdds_x2c_arena_alloc, &ar);\n\
             \x20       if (r != (int)bl) {{ printf(\"FAIL {n} used %d of %d\\n\", r, (int)bl); fails++; }}\n\
             \x20       else {{ r = {f}_encode_cdr2_le(&d, out, sizeof out); hexs(out, r, got);\n\
             \x20           if (strcmp(got, \"{x2}\") == 0) {{ printf(\"OK {n}\\n\"); }} else {{ printf(\"FAIL {n} got %s\\n\", got); fails++; }} }}\n    }}",
            n = c.name,
            x1 = c.x1,
            x2 = c.x2
        );
    }
    let program = format!(
        "#include \"types.h\"\n#include <stdio.h>\n#include <string.h>\n#include <stdlib.h>\n\
         static uint8_t heap[65536];\nstatic hdds_x2c_arena ar;\nstatic uint8_t in[1024];\nstatic uint8_t out[1024];\n\
         static char got[2100];\n\
         static void hexs(const uint8_t* b, int n, char* s) {{ int i; s[0] = 0; for (i = 0; i < n; ++i) {{ sprintf(s + 2 * i, \"%02x\", b[i]); }} }}\n\
         static size_t unhex(const char* s, uint8_t* o) {{ size_t n = strlen(s) / 2; size_t i; unsigned int x; for (i = 0; i < n; ++i) {{ sscanf(s + 2 * i, \"%2x\", &x); o[i] = (uint8_t)x; }} return n; }}\n\
         int main(void) {{\n    int fails = 0;\n{body}    return fails != 0;\n}}\n"
    );
    for (dir_name, std_flag) in [("c99", "-std=c99"), ("c89", "-std=c89")] {
        let dir = work_dir(&format!("c_{dir_name}"));
        let std = if dir_name == "c89" {
            hddsgen::codegen::CStandard::C89
        } else {
            hddsgen::codegen::CStandard::C99
        };
        let ast = Parser::try_new(IDL).expect("lexer").parse().expect("IDL");
        let header = {
            use hddsgen::codegen::CodeGenerator;
            hddsgen::codegen::c::CGenerator::with_standard(std)
                .generate(&ast)
                .expect("generate")
        };
        fs::write(dir.join("types.h"), header).expect("header");
        fs::write(dir.join("main.c"), &program).expect("program");
        let mut cmd = Command::new(cc);
        cmd.current_dir(&dir).args([
            std_flag,
            "-pedantic-errors",
            "-Wall",
            "-Wextra",
            "-Werror",
            "main.c",
            "-o",
            "main",
        ]);
        run(&mut cmd);
        let out = run(&mut Command::new(dir.join("main")));
        all_ok(&out);
    }
}

// ---------------------------------------------------------------- C++

#[test]
fn cpp_decodes_xcdr1_with_8_byte_elements_at_8() {
    let Some(cxx) = ["c++", "g++", "clang++"]
        .into_iter()
        .find(|c| tool_exists(c))
    else {
        eprintln!("no C++ compiler found; test not run");
        return;
    };
    let mut body = String::new();
    for c in CASES {
        let _ = writeln!(
            body,
            "    {{\n        {n} d;\n        auto in = unhex(\"{x1}\");\n\
             \x20       int r = d.decode_cdr1_le(in.data(), in.size());\n\
             \x20       if (r != (int)in.size()) {{ std::printf(\"FAIL {n} used %d of %d\\n\", r, (int)in.size()); ++fails; }}\n\
             \x20       else {{ std::uint8_t out[1024]; r = d.encode_cdr2_le(out, sizeof out); std::string got = hexs(out, r);\n\
             \x20           if (got == \"{x2}\") {{ std::printf(\"OK {n}\\n\"); }} else {{ std::printf(\"FAIL {n} got %s\\n\", got.c_str()); ++fails; }} }}\n    }}",
            n = c.name,
            x1 = c.x1,
            x2 = c.x2
        );
    }
    let program = format!(
        "#include \"types.hpp\"\n#include <cstdio>\n#include <string>\n#include <vector>\n\
         static std::string hexs(const std::uint8_t* b, int n) {{ std::string s; char t[3]; for (int i = 0; i < n; ++i) {{ std::snprintf(t, sizeof t, \"%02x\", b[i]); s += t; }} return s; }}\n\
         static std::vector<std::uint8_t> unhex(const std::string& s) {{ std::vector<std::uint8_t> o; for (std::size_t i = 0; i + 1 < s.size(); i += 2) {{ o.push_back(static_cast<std::uint8_t>(std::stoi(s.substr(i, 2), nullptr, 16))); }} return o; }}\n\
         int main() {{\n    int fails = 0;\n{body}    return fails != 0;\n}}\n"
    );
    let dir = work_dir("cpp");
    fs::write(dir.join("types.hpp"), generate(&Backend::Cpp, IDL)).expect("header");
    fs::write(dir.join("main.cpp"), program).expect("program");
    run(Command::new(cxx).current_dir(&dir).args([
        "-std=c++17",
        "-Wall",
        "-Wextra",
        "-Werror",
        "main.cpp",
        "-o",
        "main",
    ]));
    let out = run(&mut Command::new(dir.join("main")));
    all_ok(&out);
}

// ---------------------------------------------------------------- Python

#[test]
fn python_decodes_xcdr1_with_8_byte_elements_at_8() {
    if !tool_exists("python3") {
        eprintln!("no python3 found; test not run");
        return;
    }
    let dir = work_dir("python");
    fs::write(dir.join("gen.py"), generate(&Backend::Python, IDL)).expect("module");
    let mut runner = String::from(
        "import sys\nimport gen\nfails = 0\n\
         def case(name, x1, x2):\n\
         \x20   global fails\n\
         \x20   data = bytes.fromhex(x1)\n\
         \x20   v, n = getattr(gen, name).decode_cdr1_le(data)\n\
         \x20   if n != len(data):\n\
         \x20       print('FAIL %s used %d of %d' % (name, n, len(data))); fails += 1; return\n\
         \x20   got = v.encode_cdr2_le().hex()\n\
         \x20   if got == x2:\n\
         \x20       print('OK ' + name)\n\
         \x20   else:\n\
         \x20       print('FAIL %s got %s' % (name, got)); fails += 1\n",
    );
    for c in CASES {
        let _ = writeln!(runner, "case('{}', '{}', '{}')", c.name, c.x1, c.x2);
    }
    runner.push_str("sys.exit(1 if fails else 0)\n");
    fs::write(dir.join("run.py"), runner).expect("runner");
    let out = run(Command::new("python3")
        .arg("-I")
        .current_dir(&dir)
        .arg("-c")
        .arg("import sys; sys.path.insert(0, '.'); exec(open('run.py').read())"));
    all_ok(&out);
}

// ---------------------------------------------------------------- TypeScript

#[test]
fn typescript_decodes_xcdr1_with_8_byte_elements_at_8() {
    const NODE_ARGS: [&str; 2] = ["--experimental-transform-types", "--no-warnings"];
    let dir = work_dir("typescript");
    let probe = dir.join("probe.ts");
    fs::write(&probe, "const x: number = 1; console.log(x);\n").expect("probe");
    if !Command::new("node")
        .args(NODE_ARGS)
        .arg(&probe)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("node with --experimental-transform-types not found; test not run");
        return;
    }
    let mut src = generate(&Backend::TypeScript, IDL);
    src.push_str(
        "\nfunction hex(b: Uint8Array): string { return Array.from(b).map((x) => x.toString(16).padStart(2, \"0\")).join(\"\"); }\n\
         function unhex(s: string): Uint8Array { const b = new Uint8Array(s.length / 2); for (let i = 0; i < b.length; i++) { b[i] = parseInt(s.slice(2 * i, 2 * i + 2), 16); } return b; }\n\
         function chk(name: string, dec: (b: Cdr1Buffer) => any, enc: (o: any, b: Cdr2Buffer) => void, x1: string, x2: string): void {\n\
         \x20 try {\n\
         \x20   const bytes = unhex(x1);\n\
         \x20   const buf = new Cdr1Buffer(bytes);\n\
         \x20   const d = dec(buf);\n\
         \x20   if (buf.position !== bytes.length) { console.log(`FAIL ${name} used ${buf.position} of ${bytes.length}`); return; }\n\
         \x20   const out = new Cdr2Buffer(new Uint8Array(4096).fill(0xaa));\n\
         \x20   enc(d, out);\n\
         \x20   const got = hex(out.toBytes());\n\
         \x20   console.log(got === x2 ? `OK ${name}` : `FAIL ${name} got ${got}`);\n\
         \x20 } catch (e) { console.log(`FAIL ${name} error ${e}`); }\n\
         }\n",
    );
    for c in CASES {
        let _ = writeln!(
            src,
            "chk(\"{n}\", decode{n}Cdr1, encode{n}, \"{x1}\", \"{x2}\");",
            n = c.name,
            x1 = c.x1,
            x2 = c.x2
        );
    }
    let file = dir.join("align.ts");
    fs::write(&file, src).expect("source");
    let out = run(Command::new("node").args(NODE_ARGS).arg(&file));
    all_ok(&out);
}

// ---------------------------------------------------------------- Rust

fn rust_main() -> String {
    let mut body = String::new();
    for c in CASES {
        for (ver, hex) in [("xcdr1", c.x1), ("xcdr2", c.x2)] {
            let _ = writeln!(
                body,
                "    chk::<{n}>(\"{n} {ver}\", &{v}, \"{hex}\", |v, b| v.encode_{ver}_le(b), |v| v.max_{ver}_size(), |b| {n}::decode_{ver}_le(b));",
                n = c.name,
                v = c.rust,
            );
        }
    }
    format!(
        "#![allow(dead_code, unused_imports, non_snake_case, clippy::all)]\n\
         #[allow(clippy::all)]\nmod generated {{ include!(\"generated.rs\"); }}\n\
         use generated::*;\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         fn unhex(s: &str) -> Vec<u8> {{ (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }}\n\
         fn chk<T: PartialEq + std::fmt::Debug>(name: &str, v: &T, exp: &str,\n\
             enc: impl Fn(&T, &mut [u8]) -> Result<usize, hdds::CdrError>,\n\
             size: impl Fn(&T) -> usize,\n\
             dec: impl Fn(&[u8]) -> Result<(T, usize), hdds::CdrError>) {{\n\
             let mut buf = vec![0xAAu8; 4096];\n\
             match enc(v, &mut buf) {{\n\
                 Ok(n) if hex(&buf[..n]) == exp && size(v) == n => println!(\"OK enc {{name}}\"),\n\
                 Ok(n) => println!(\"FAIL enc {{name}} size={{}}\\n  got {{}}\\n  exp {{exp}}\", size(v), hex(&buf[..n])),\n\
                 Err(e) => println!(\"FAIL enc {{name}} error {{e:?}}\"),\n\
             }}\n\
             let mut short = vec![0u8; exp.len() / 2 - 1];\n\
             match enc(v, &mut short) {{\n\
                 Err(hdds::CdrError::BufferTooSmall) => println!(\"OK short {{name}}\"),\n\
                 other => println!(\"FAIL short {{name}} {{other:?}}\"),\n\
             }}\n\
             let bytes = unhex(exp);\n\
             match dec(&bytes) {{\n\
                 Ok((d, n)) if &d == v && n == bytes.len() => println!(\"OK dec {{name}}\"),\n\
                 Ok((d, n)) => println!(\"FAIL dec {{name}} used {{n}} of {{}}\\n  got {{d:?}}\", bytes.len()),\n\
                 Err(e) => println!(\"FAIL dec {{name}} error {{e:?}}\"),\n\
             }}\n\
         }}\n\
         fn main() {{\n{body}}}\n"
    )
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test xcdr1_alignment -- --ignored"]
fn rust_codec_aligns_8_byte_elements_at_8_under_xcdr1() {
    let hdds = hdds_dir("hdds");
    let deps = format!(
        "hdds = {{ path = \"{}\" }}\n",
        hdds.join("crates/hdds").display()
    );
    let out = run_crate(
        "align_rust",
        &deps,
        &[
            ("generated.rs", generate(&Backend::Rust, IDL)),
            ("main.rs", rust_main()),
        ],
    );
    let ok = |k: &str| {
        out.lines()
            .filter(|l| l.starts_with(&format!("OK {k} ")))
            .count()
    };
    assert!(
        !out.contains("FAIL")
            && ok("enc") == CASES.len() * 2
            && ok("short") == CASES.len() * 2
            && ok("dec") == CASES.len() * 2,
        "{out}"
    );
}

// Key-only payloads: the KeyHolder at XCDR1 (8-byte alignment) and XCDR2,
// little endian as written and big endian as read, hand-laid out by the same
// rule (multi-byte fields swap, offsets and padding do not move).
const KEY_IDL: &str = "
@final struct KeyArr { @key long l; @key double d[2]; long tail; };
@final struct KeySeq { @key long a; @key long b; @key sequence<double> s; long tail; };
";

/// (type, value, XCDR2 LE, XCDR1 LE, XCDR2 BE, XCDR1 BE)
const KEYS: &[(&str, &str, &str, &str, &str, &str)] = &[
    (
        "KeyArr",
        "KeyArr { l: 7, d: [1.5, -2.0], tail: 3 }",
        "07000000000000000000f83f00000000000000c0",
        "0700000000000000000000000000f83f00000000000000c0",
        "000000073ff8000000000000c000000000000000",
        "00000007000000003ff8000000000000c000000000000000",
    ),
    (
        "KeySeq",
        "KeySeq { a: 1, b: 2, s: vec![1.5, -2.0], tail: 3 }",
        "010000000200000002000000000000000000f83f00000000000000c0",
        "01000000020000000200000000000000000000000000f83f00000000000000c0",
        "0000000100000002000000023ff8000000000000c000000000000000",
        "000000010000000200000002000000003ff8000000000000c000000000000000",
    ),
];

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test xcdr1_alignment -- --ignored"]
fn rust_key_payload_aligns_8_byte_elements_in_both_byte_orders() {
    let hdds = hdds_dir("hdds");
    let deps = format!(
        "hdds = {{ path = \"{}\" }}\n",
        hdds.join("crates/hdds").display()
    );
    let mut body = String::new();
    for (name, value, x2, x1, x2be, x1be) in KEYS {
        let _ = writeln!(
            body,
            "    check(\"{name}\", &{value}, \"{x2}\", \"{x1}\", \"{x2be}\", \"{x1be}\");"
        );
    }
    let main = format!(
        "#![allow(dead_code, unused_imports, non_snake_case, clippy::all)]\n\
         #[allow(clippy::all)]\nmod generated {{ include!(\"generated.rs\"); }}\n\
         use generated::*;\n\
         use hdds::dds::DDS;\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         fn unhex(s: &str) -> Vec<u8> {{ (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }}\n\
         fn read<T: DDS>(id: u16, payload: &str) -> Option<[u8; 16]> {{\n\
             let mut p = vec![(id >> 8) as u8, (id & 0xff) as u8, 0, 0];\n\
             p.extend_from_slice(&unhex(payload));\n\
             T::key_hash_of_key_payload(&p)\n\
         }}\n\
         fn check<T: DDS>(name: &str, v: &T, x2: &str, x1: &str, x2be: &str, x1be: &str) {{\n\
             let mut ok = true;\n\
             let want = v.compute_key();\n\
             for (what, got, exp) in [\n\
                 (\"write xcdr2\", hex(&v.encode_key(hdds::CdrVersion::Xcdr2)), x2),\n\
                 (\"write xcdr1\", hex(&v.encode_key(hdds::CdrVersion::Xcdr1)), x1),\n\
             ] {{\n\
                 if got != exp {{ println!(\"FAIL {{name}} {{what}}: got {{got}} exp {{exp}}\"); ok = false; }}\n\
             }}\n\
             // CDR2_LE, CDR_LE, CDR2_BE, CDR_BE (DDS-XTypes 1.3 Table 60).\n\
             for (what, id, payload) in [(\"read xcdr2 le\", 0x0007u16, x2), (\"read xcdr1 le\", 0x0001, x1), (\"read xcdr2 be\", 0x0006, x2be), (\"read xcdr1 be\", 0x0000, x1be)] {{\n\
                 if read::<T>(id, payload) != Some(want) {{ println!(\"FAIL {{name}} {{what}}\"); ok = false; }}\n\
             }}\n\
             // The XCDR2 layout under an XCDR1 identifier is another value (or none).\n\
             for (what, id, payload) in [(\"xcdr2 layout as xcdr1 le\", 0x0001u16, x2), (\"xcdr2 layout as xcdr1 be\", 0x0000, x2be)] {{\n\
                 if read::<T>(id, payload) == Some(want) {{ println!(\"FAIL {{name}} {{what}} was read as the key\"); ok = false; }}\n\
             }}\n\
             if ok {{ println!(\"OK {{name}}\"); }}\n\
         }}\n\
         fn main() {{\n{body}}}\n"
    );
    let out = run_crate(
        "align_rust_keys",
        &deps,
        &[
            ("generated.rs", generate(&Backend::Rust, KEY_IDL)),
            ("main.rs", main),
        ],
    );
    let ok = out.lines().filter(|l| l.starts_with("OK ")).count();
    assert!(!out.contains("FAIL") && ok == KEYS.len(), "{out}");
}

// ---------------------------------------------------------------- micro

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test xcdr1_alignment -- --ignored"]
fn micro_codec_aligns_8_byte_elements_at_8() {
    let hdds = hdds_dir("hdds-micro");
    let deps = format!(
        "hdds-micro = {{ path = \"{}\", features = [\"heapless\"] }}\nheapless = \"0.8\"\n",
        hdds.join("crates/hdds-micro").display()
    );
    let mut body = String::new();
    for c in CASES {
        let _ = writeln!(
            body,
            "    chk::<{n}>(\"{n}\", &{v}, \"{x1}\", {n}::ENCODED_SIZE_MAX, |v, e| v.encode(e), |d| {n}::decode(d));",
            n = c.name,
            v = c.micro,
            x1 = c.x1
        );
    }
    let main = format!(
        "#![allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, clippy::all)]\n\
         #[path = \"generated.rs\"]\nmod generated;\n\
         use generated::*;\n\
         fn V<T: Clone, const N: usize>(v: &[T]) -> heapless::Vec<T, N> {{ heapless::Vec::from_slice(v).expect(\"fits\") }}\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         fn unhex(s: &str) -> Vec<u8> {{ (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect() }}\n\
         fn chk<T: PartialEq + core::fmt::Debug>(name: &str, v: &T, exp: &str, max: usize,\n\
             enc: impl Fn(&T, &mut CdrEncoder) -> Result<(), CdrError>,\n\
             dec: impl Fn(&mut CdrDecoder) -> Result<T, CdrError>) {{\n\
             let mut buf = [0xAAu8; 2048];\n\
             let mut e = CdrEncoder::new(&mut buf);\n\
             match enc(v, &mut e) {{\n\
                 Ok(()) => {{ let got = hex(e.finish()); if got == exp {{ println!(\"OK enc {{name}}\"); }} else {{ println!(\"FAIL enc {{name}}\\n  got {{got}}\\n  exp {{exp}}\"); }} }}\n\
                 Err(err) => println!(\"FAIL enc {{name}} {{err:?}}\"),\n\
             }}\n\
             let bytes = unhex(exp);\n\
             let mut d = CdrDecoder::new(&bytes);\n\
             match dec(&mut d) {{\n\
                 Ok(x) if &x == v => println!(\"OK dec {{name}}\"),\n\
                 Ok(x) => println!(\"FAIL dec {{name}} got {{x:?}}\"),\n\
                 Err(err) => println!(\"FAIL dec {{name}} {{err:?}}\"),\n\
             }}\n\
             if bytes.len() <= max {{ println!(\"OK max {{name}}\"); }} else {{ println!(\"FAIL max {{name}} {{max}} < {{}}\", bytes.len()); }}\n\
         }}\n\
         fn main() {{\n{body}}}\n"
    );
    let out = run_crate(
        "align_micro",
        &deps,
        &[
            ("generated.rs", generate(&Backend::Micro, IDL)),
            ("main.rs", main),
        ],
    );
    let ok = |k: &str| {
        out.lines()
            .filter(|l| l.starts_with(&format!("OK {k} ")))
            .count()
    };
    assert!(
        !out.contains("FAIL")
            && ok("enc") == CASES.len()
            && ok("dec") == CASES.len()
            && ok("max") == CASES.len(),
        "{out}"
    );
}
