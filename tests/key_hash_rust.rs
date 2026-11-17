// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated Rust and micro code, generated, compiled
//! and run against the HDDS crates (RTPS 2.5 9.6.4.8).
//!
//! The generated code depends on `hdds` (Rust backend) and `hdds-micro`
//! (micro backend), which live in the HDDS repository, not here. Point
//! `HDDSGEN_HDDS_DIR` at an HDDS checkout (the directory holding
//! `crates/hdds` and `crates/hdds-micro`) and run them with `-- --ignored`:
//! they are ignored by default (reported as such, never as passed) and fail
//! when requested without that checkout. Cargo runs offline: the HDDS
//! dependencies must already be in the local registry cache.
//!
//! Expected values: RTPS 2.5 9.6.4.8 Examples 1 to 3 (SPEC), and an
//! independent Python transcription of the algorithm, `struct.pack('>..')`
//! with alignment capped at 4 then pad or `hashlib.md5` (REF).

#![allow(clippy::expect_used)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use hddsgen::{Backend, Parser};

const GOLDEN_IDL: &str = include_str!("fixtures/key_hash_golden.idl");

/// Extra files compiled into the golden crates: a union and nothing keyed,
/// a union only in a non-keyed struct (both must compile although no keyed
/// type reaches the union's hidden methods), and an `@external` key (no
/// `unused_parens` around its dereference).
const EXTRA_IDLS: &[&str] = &[
    "union U switch (long) { case 1: long a; case 2: string b; };",
    "union U switch (long) { case 1: long a; case 2: string b; };\n\
     struct S { U u; long x; };",
    "struct Box1 { @key @external long id; long x; };",
];

/// Micro only: keys this backend must refuse because its `KeyHash` does not
/// read base members, reached through a nested struct and through a union
/// case.
const MICRO_REFUSED_IDL: &str = "
struct BaseK { @key long id; long payload; };
struct DerivedK : BaseK { @key short sub; long more; };
struct Holder { @key DerivedK d; };
union UB switch (long) { case 1: DerivedK d; case 2: long l; };
struct UHolder { @key UB u; long x; };
";

/// The extra generated files and the module declarations that include them.
fn extra_files(backend: &Backend, path_attr: bool) -> (String, Vec<(String, String)>) {
    let mut idls: Vec<&str> = EXTRA_IDLS.to_vec();
    if path_attr {
        idls.push(MICRO_REFUSED_IDL);
    }
    let mut mods = String::new();
    let mut files = Vec::new();
    for (i, idl) in idls.iter().enumerate() {
        let file = format!("extra_{i}.rs");
        if path_attr {
            let _ = writeln!(mods, "#[path = \"{file}\"]\nmod extra_{i};");
        } else {
            let _ = writeln!(mods, "mod extra_{i} {{ include!(\"{file}\"); }}");
        }
        files.push((file, generate(backend, idl)));
    }
    (mods, files)
}
const EXT_IDL: &str = include_str!("fixtures/key_hash_ext.idl");

/// Union keys with a boolean discriminator and with scoped enumerator
/// labels (`X::S1`, `::X::S2`), in module `X` of the extended set (Rust;
/// the micro set carries them inline). Expected values: REF (Cyclone DDS
/// 0.11 refuses a union in a key).
const UNION_LABEL_KEYS_IDL: &str = "
  union UBK switch (boolean) { case TRUE: long t; case FALSE: short f; };
  @final struct BoolU { @key UBK u; long x; };
  union UEK switch (Small) { case X::S1: long a; case ::X::S2: short b; default: octet o; };
  @final struct EnumU { @key UEK u; };
";

fn rust_ext_idl() -> String {
    EXT_IDL.replacen("\n};\n", &format!("\n{UNION_LABEL_KEYS_IDL}}};\n"), 1)
}

/// Why these tests are `#[ignore]`d by default: they need an HDDS checkout.
/// Run them with `HDDSGEN_HDDS_DIR=<hdds> cargo test --test key_hash_rust --
/// --ignored`; requested without one, they fail rather than pass.
fn hdds_dir() -> PathBuf {
    let dir = PathBuf::from(std::env::var_os("HDDSGEN_HDDS_DIR").unwrap_or_default());
    assert!(
        dir.join("crates/hdds/Cargo.toml").is_file()
            && dir.join("crates/hdds-micro/Cargo.toml").is_file(),
        "HDDSGEN_HDDS_DIR={:?} must name an HDDS checkout (crates/hdds and \
         crates/hdds-micro): these KeyHash compile tests did NOT run",
        dir.display().to_string()
    );
    dir
}

fn generate(backend: &Backend, idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    backend.generator().generate(&ast).expect("codegen")
}

/// Write a crate under the test's scratch directory and `cargo run` it
/// (`cargo build` for a library: `lib.rs` among the files).
fn run_crate(name: &str, manifest_deps: &str, files: &[(&str, String)]) -> String {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("key_hash")
        .join(name);
    fs::create_dir_all(root.join("src")).expect("crate dir");
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\npublish = false\n\n\
         [dependencies]\n{manifest_deps}\n[workspace]\n"
    );
    fs::write(root.join("Cargo.toml"), manifest).expect("manifest");
    for (file, content) in files {
        fs::write(root.join("src").join(file), content).expect("source");
    }
    let is_lib = files.iter().any(|(f, _)| *f == "lib.rs");
    let verb = if is_lib { "build" } else { "run" };
    let output = Command::new(env!("CARGO"))
        .args([verb, "--offline", "--quiet"])
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

/// Golden vectors on the types of `key_hash_golden.idl`: (label, Rust
/// value expression with `S` the string constructor, expected `KeyHash`).
const GOLDEN: &[(&str, &str, &str)] = &[
    // SPEC Example 1.
    (
        "LongKey",
        "LongKey { id: 0x1234_5678, x: 9 }",
        "12345678000000000000000000000000",
    ),
    // SPEC Example 2: `long long` aligned at 4, maximum 28, MD5.
    (
        "SpecEx2",
        "SpecEx2 { label: S(\"BLUE\"), id: 0x1234_5678_9abc_def0, x: 1, y: 2 }",
        "f91a59e32e4535d9a69cd5d9f5b6e36e",
    ),
    // SPEC Example 3: @id reorders, nested key, 13 bytes but maximum 21.
    (
        "SpecEx3",
        "SpecEx3 { label: S(\"BLUE\"), m_nested: Nested { m_long: 0x1234_5678, u: 1, w: 2 }, x: 1, y: 2 }",
        "374b96e2e727237f016cc4cebb6eb71e",
    ),
    // REF: unbounded string, MD5 although nine bytes fit.
    (
        "StrKey",
        "StrKey { label: S(\"BLUE\"), x: 1 }",
        "cac217c318363f8ef1160eeedef9e886",
    ),
    // REF: unbounded string of 50 characters, a 55-byte serialization:
    // the MD5 padding edge (55 and 56 bytes, one and two blocks; 64, 65).
    (
        "StrKey55",
        "StrKey { label: S(&\"Q\".repeat(50)), x: 1 }",
        "fa67225c93f8071a5bbcf6df1bf9bfcd",
    ),
    // REF: unbounded string of 51 characters, a 56-byte serialization:
    // the MD5 padding edge (55 and 56 bytes, one and two blocks; 64, 65).
    (
        "StrKey56",
        "StrKey { label: S(&\"Q\".repeat(51)), x: 1 }",
        "424bcbae3ad0fa3c57426e73ad1261af",
    ),
    // REF: unbounded string of 59 characters, a 64-byte serialization:
    // the MD5 padding edge (55 and 56 bytes, one and two blocks; 64, 65).
    (
        "StrKey64",
        "StrKey { label: S(&\"Q\".repeat(59)), x: 1 }",
        "4b60eb0dd74deb107a2c9cad4e781130",
    ),
    // REF: unbounded string of 60 characters, a 65-byte serialization:
    // the MD5 padding edge (55 and 56 bytes, one and two blocks; 64, 65).
    (
        "StrKey65",
        "StrKey { label: S(&\"Q\".repeat(60)), x: 1 }",
        "e7c43e8aa8fbb48e72bc38d01eb741a5",
    ),
    // REF: string<8>, maximum 13, padded; the length counts the NUL.
    (
        "StrKey8",
        "StrKey8 { label: S(\"BLUE\"), x: 1 }",
        "00000005424c55450000000000000000",
    ),
    // REF: string<11>, maximum 16, the boundary, padded.
    (
        "Str11",
        "Str11 { label: S(\"BLUE\"), x: 1 }",
        "00000005424c55450000000000000000",
    ),
    // REF: string<12>, maximum 17, one past the boundary: MD5.
    (
        "Str12",
        "Str12 { label: S(\"BLUE\"), x: 1 }",
        "cac217c318363f8ef1160eeedef9e886",
    ),
    // REF: the shapes demo's `ShapeType`, string<128>: maximum 133, MD5.
    (
        "ShapeType",
        "ShapeType { color: S(\"BLUE\"), x: 1, y: 2, shapesize: 3 }",
        "cac217c318363f8ef1160eeedef9e886",
    ),
    // REF: two-member key.
    (
        "TwoKey",
        "TwoKey { a: 1, b: 2, x: 3 }",
        "00000001000200000000000000000000",
    ),
    // REF: @mutable with @id reordering, the short first.
    (
        "TwoKeyId",
        "TwoKeyId { a: 1, b: 2, x: 3 }",
        "00020000000000010000000000000000",
    ),
    // REF: nested struct key, its non-key member left out...
    (
        "Outer",
        "Outer { in_: Inner { k: 0x1122_3344, u: 7 }, tag: 0x55, x: 1 }",
        "11223344550000000000000000000000",
    ),
    // ...so changing it does not move the KeyHash.
    (
        "OuterU",
        "Outer { in_: Inner { k: 0x1122_3344, u: -7 }, tag: 0x55, x: 2 }",
        "11223344550000000000000000000000",
    ),
    // REF: nested struct without key contributes all members.
    (
        "OuterNoKey",
        "OuterNoKey { p: Inner2 { a: 1, b: 2 }, x: 1 }",
        "00000001000000020000000000000000",
    ),
    // REF: `long long` aligned at 4 behind an octet.
    (
        "AlignKey",
        "AlignKey { t: 1, v: 2 }",
        "01000000000000000000000200000000",
    ),
    // REF: maximum exactly 16, padded.
    (
        "Key16",
        "Key16 { a: 1, b: 2 }",
        "00000000000000010000000000000002",
    ),
    // REF: maximum 16 only with alignment capped at 4 (20 at 8): padded.
    (
        "CapKey",
        "CapKey { t: 1, v: 2, c: 3 }",
        "01000000000000000000000200000003",
    ),
    // REF: key larger than 16 bytes, MD5.
    (
        "BigKey",
        "BigKey { a: 1, b: 2, c: 3 }",
        "8898488e44b5e511051f83ce5e917bb8",
    ),
];

/// REF vectors on the types of `key_hash_ext.idl` (Rust main backend).
const EXT: &[(&str, &str, &str)] = &[
    (
        "Scalars",
        "Scalars { b: true, c: 'A', wc: 'Z', i8v: -2, s: 0x0102, d: 1.5, us: 0xbeef, f: -1.0, nk: 9 }",
        "777ac52171394c4abb7947cd8f237982",
    ),
    (
        "Enums",
        "Enums { c: Color::BLUE, sm: Small::S2, md: Mid::M1, f8: 3, f32: 2, f64: 1 << 40, ml: -5 }",
        "e2265fcd85db8fc8343f51c33f672a8b",
    ),
    (
        "Colls",
        "Colls { sq: vec![1, 2], bsq: vec![9, 8, 7], arr: [1, 2, 3], oct2: [[1, 2], [3, 4]], ws: S(\"h\\u{e9}\"), n6: S(\"abc\") }",
        "7b44e4a1ef2ab87d5bc2b6b8d253beca",
    ),
    (
        "Unions",
        "Unions { u: U::B(S(\"xy\")), uk: UK::G(2.5), tail: 0x77 }",
        "45ec7942ba3bf7038d5ebc3abfc11459",
    ),
    (
        "Hashed",
        "Hashed { color: 0x11, shapesize: 0x22 }",
        "00000022000000110000000000000000",
    ),
    (
        "FinalIds",
        "FinalIds { a: 0x0a0b_0c0d, b: 1 }",
        "010000000a0b0c0d0000000000000000",
    ),
    (
        "MutSeq",
        "MutSeq { a: 1, b: 2, c: S(\"Q\") }",
        "ae5426ae810065bd695fddf3baf7b5c1",
    ),
    // REF: union keys, boolean discriminator (1 byte) and enumerator
    // labels written scoped; the default case takes the smallest free
    // value (S0).
    (
        "BoolUT",
        "BoolU { u: UBK::T(0x1122_3344), x: 1 }",
        "01000000112233440000000000000000",
    ),
    (
        "BoolUF",
        "BoolU { u: UBK::F(0x22), x: 1 }",
        "00000022000000000000000000000000",
    ),
    (
        "EnumUA",
        "EnumU { u: UEK::A(0x55) }",
        "01000000000000550000000000000000",
    ),
    (
        "EnumUB",
        "EnumU { u: UEK::B(0x33) }",
        "02000033000000000000000000000000",
    ),
    (
        "EnumUDef",
        "EnumU { u: UEK::O(0x44) }",
        "00440000000000000000000000000000",
    ),
    // Cyclone DDS 0.11: sequence<char> / sequence<wchar> keys.
    (
        "SeqChars",
        "SeqChars { c: vec!['x', 'y'], w: vec!['A', 'B'], x: 1 }",
        "7cfc7a60d95c3037ff7785cfd6822d1c",
    ),
    (
        "SeqCharU",
        "SeqCharU { c: vec!['x', 'y'], x: 1 }",
        "62a27c5288ec9d48152240ad0670d151",
    ),
    // Cyclone DDS 0.11: module-level @autoid(HASH).
    (
        "ModHashed",
        "generated::MH::ModHashed { color: 0x11, shapesize: 0x22 }",
        "00000022000000110000000000000000",
    ),
];

/// The key-only payload `encode_key` returns, XCDR2 then XCDR1, hex, and the
/// same payload big-endian. Hand-derived from `XTypes` 1.3 7.4.3.5.3 (rules
/// 17, 21, 22, 23, 25): the byte order moves every multi-byte field, the
/// framing and the lengths stay the same.
/// A sample, its key-only payload at XCDR2 then XCDR1, and the same two
/// payloads big-endian under the encapsulation identifier of their version.
type KeyPayload = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<(u16, &'static str, u16, &'static str)>,
);

const PAYLOADS: &[KeyPayload] = &[
    // FINAL: the key members, little endian, no header.
    (
        "LongKey",
        "LongKey { id: 0x1234_5678, x: 9 }",
        "78563412",
        "78563412",
        Some((0x0006, "12345678", 0x0000, "12345678")),
    ),
    // FINAL: the `long long` at 4 under XCDR2, at 8 under XCDR1.
    (
        "AlignKey",
        "AlignKey { t: 1, v: 2 }",
        "010000000200000000000000",
        "01000000000000000200000000000000",
        // The octet keeps its single byte: only the multi-byte members move.
        Some((
            0x0006,
            "010000000000000000000002",
            0x0000,
            "01000000000000000000000000000002",
        )),
    ),
    // MUTABLE (generator member ids 2 and 1): DHEADER 14, EMHEADER1 with M
    // and LC 2 / LC 1; XCDR1: PID_EXTENDED (always 0x7F01, 7.4.1.2.2) with
    // the member's must-understand flag in the 32-bit member header
    // (0x4000_0000 | id), then PID_LIST_END with its must-understand flag
    // (0x7F02, Table 34; what Cyclone DDS writes).
    (
        "TwoKeyId",
        "TwoKeyId { a: 1, b: 2, x: 3 }",
        "0e000000020000a001000000010000900200",
        "017f0800020000400400000001000000017f0800010000400200000002000000027f0000",
        Some((
            0x000a,
            "0000000ea000000200000001900000010002",
            0x0002,
            "7f0100084000000200000004000000017f0100084000000100000002000200007f020000",
        )),
    ),
    // A MUTABLE nested KeyHolder whose members are not keys (its type has
    // none, so all are kept): no M flag, in EMHEADER1 (XCDR2) and in the
    // 32-bit member header behind PID 0x7F01 (XCDR1). Member ids 0 and 1
    // (SEQUENTIAL, XTypes 1.3 7.3.1.2.1.1). XCDR1: what Cyclone DDS 0.11
    // writes; XCDR2: Cyclone writes the same bytes with the M flag on `a`
    // and `b` (it flags every member reached through a key), where the
    // specification ties the flag to the member's own attribute
    // (7.2.2.4.4.4.6), false for InnerM's members.
    (
        "OuterM",
        "OuterM { p: InnerM { a: 1, b: 2 } }",
        "1000000000000020010000000100002002000000",
        "017f0800000000000400000001000000017f0800010000000400000002000000027f0000",
        Some((
            0x0006,
            "0000001020000000000000012000000100000002",
            0x0000,
            "7f0100080000000000000004000000017f0100080000000100000004000000027f020000",
        )),
    ),
    // FINAL, key members only: the non-key member is absent.
    (
        "OuterNoKey",
        "OuterNoKey { p: Inner2 { a: 1, b: 2 }, x: 1 }",
        "0100000002000000",
        "0100000002000000",
        Some((0x0006, "0000000100000002", 0x0000, "0000000100000002")),
    ),
];

fn rust_main(cases: &[(&str, &str, &str)], module: &str) -> String {
    let mut body = String::new();
    for (label, value, expected) in cases {
        let _ = writeln!(body, "    chk(\"{label}\", &{value}, \"{expected}\");");
    }
    format!(
        "#![allow(dead_code, unused_imports, unused_variables, non_snake_case, non_camel_case_types, clippy::all)]\n\
         #![deny(unused_parens)]\n\
         #[allow(clippy::all)]\nmod generated {{ include!(\"generated.rs\"); }}\n\
         use generated::{module}::*;\n\
         use hdds::dds::DDS;\n\
         #[allow(non_snake_case)]\nfn S(s: &str) -> String {{ s.to_string() }}\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         static mut FAILS: u32 = 0;\n\
         fn chk<T: DDS>(label: &str, v: &T, expected: &str) {{\n\
         \x20   let got = hex(&v.compute_key());\n\
         \x20   if got == expected {{ println!(\"OK {{label}}\"); }} else {{\n\
         \x20       println!(\"FAIL {{label}}: got {{got}} expected {{expected}}\");\n\
         \x20       unsafe {{ FAILS += 1; }}\n\
         \x20   }}\n\
         }}\n\
         fn pay<T: DDS>(label: &str, v: &T, x2: &str, x1: &str) {{\n\
         \x20   let g2 = hex(&v.encode_key(hdds::CdrVersion::Xcdr2));\n\
         \x20   let g1 = hex(&v.encode_key(hdds::CdrVersion::Xcdr1));\n\
         \x20   if g2 == x2 && g1 == x1 {{ println!(\"OK payload {{label}}\"); }} else {{\n\
         \x20       println!(\"FAIL payload {{label}}: xcdr2 {{g2}} xcdr1 {{g1}}\");\n\
         \x20       unsafe {{ FAILS += 1; }}\n\
         \x20   }}\n\
         }}\n\
         fn unhex(s: &str) -> Vec<u8> {{\n\
         \x20   (0..s.len()).step_by(2)\n\
         \x20       .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect(\"hex\"))\n\
         \x20       .collect()\n\
         }}\n\
         /// The inverse: the key-only payload, encapsulated, names the same\n\
         /// instance as the sample, under the encapsulation identifier of the\n\
         /// version the payload was written at, in every byte order.\n\
         fn back<T: DDS>(label: &str, v: &T, payloads: &[(u16, &str)]) {{\n\
         \x20   let want = v.compute_key();\n\
         \x20   let mut ok = true;\n\
         \x20   for (kind, payload) in payloads {{\n\
         \x20       // The 4-byte encapsulation header: the identifier, then the\n\
         \x20       // two options bytes (DDS-XTypes 1.3 7.6.3.1.2).\n\
         \x20       let mut p = vec![(*kind >> 8) as u8, (*kind & 0xff) as u8, 0, 0];\n\
         \x20       p.extend_from_slice(&unhex(payload));\n\
         \x20       if T::key_hash_of_key_payload(&p) != Some(want) {{\n\
         \x20           println!(\"FAIL back {{label}} kind {{kind:#06x}}\");\n\
         \x20           ok = false;\n\
         \x20       }}\n\
         \x20       // Bytes after the key are not what `encode_key` writes: the\n\
         \x20       // reader must refuse them, not hash the prefix it read.\n\
         \x20       let mut trailing = p.clone();\n\
         \x20       trailing.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);\n\
         \x20       if T::key_hash_of_key_payload(&trailing).is_some() {{\n\
         \x20           println!(\"FAIL back {{label}}: trailing bytes after the key\");\n\
         \x20           ok = false;\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   for empty in [0x0009u16, 0x0008] {{\n\
         \x20       let p = vec![(empty >> 8) as u8, (empty & 0xff) as u8, 0, 0];\n\
         \x20       if T::key_hash_of_key_payload(&p).is_some() {{\n\
         \x20           println!(\"FAIL back {{label}}: an empty body named an instance\");\n\
         \x20           ok = false;\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   if ok {{ println!(\"OK back {{label}}\"); }} else {{ unsafe {{ FAILS += 1; }} }}\n\
         }}\n\
         fn bk<T: DDS>(label: &str, v: &T, x2: &str, x1: &str) {{\n\
         \x20   back(label, v, &[(0x0009u16, x2), (0x0001u16, x1)]);\n\
         }}\n\
         /// The same key-only payloads in big-endian form, under the\n\
         /// encapsulation identifiers the type's extensibility announces.\n\
         fn bkb<T: DDS>(label: &str, v: &T, k2: u16, x2: &str, k1: u16, x1: &str) {{\n\
         \x20   back(&format!(\"{{label}} BE\"), v, &[(k2, x2), (k1, x1)]);\n\
         }}\n\
         fn main() {{\n{body}{{PAYLOADS}}{{BACKS}}\
         \x20   if unsafe {{ FAILS }} != 0 {{ std::process::exit(1); }}\n\
         }}\n"
    )
}

fn count_ok(stdout: &str) -> usize {
    stdout.lines().filter(|l| l.starts_with("OK ")).count()
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test key_hash_rust -- --ignored"]
fn rust_backend_key_hash_matches_the_golden_vectors() {
    let hdds = hdds_dir();
    let deps = format!(
        "hdds = {{ path = \"{}\" }}\n",
        hdds.join("crates/hdds").display()
    );

    let mut payloads = String::new();
    let mut backs = String::new();
    for (label, value, x2, x1, be) in PAYLOADS {
        let _ = writeln!(
            payloads,
            "    pay(\"{label}\", &{value}, \"{x2}\", \"{x1}\");"
        );
        let _ = writeln!(backs, "    bk(\"{label}\", &{value}, \"{x2}\", \"{x1}\");");
        if let Some((k2, x2be, k1, x1be)) = be {
            let _ = writeln!(
                backs,
                "    bkb(\"{label}\", &{value}, {k2:#06x}, \"{x2be}\", {k1:#06x}, \"{x1be}\");"
            );
        }
    }
    let both_orders = PAYLOADS.iter().filter(|p| p.4.is_some()).count();
    let (mods, extras) = extra_files(&Backend::Rust, false);
    let main = rust_main(GOLDEN, "P")
        .replace("{PAYLOADS}", &payloads)
        .replace("{BACKS}", &backs)
        .replace(
            "fn main() {\n",
            &format!(
                "{mods}fn main() {{\n    chk(\"Box1\", &extra_2::Box1 {{ id: Box::new(7), x: 1 }}, \"00000007000000000000000000000000\");\n"
            ),
        );
    let mut files = vec![
        ("main.rs".to_string(), main),
        (
            "generated.rs".to_string(),
            generate(&Backend::Rust, GOLDEN_IDL),
        ),
    ];
    files.extend(extras);
    let files: Vec<(&str, String)> = files.iter().map(|(f, c)| (f.as_str(), c.clone())).collect();
    let out = run_crate("kh_rust_golden", &deps, &files);
    assert_eq!(
        count_ok(&out),
        GOLDEN.len() + PAYLOADS.len() + PAYLOADS.len() + both_orders + 1,
        "{out}"
    );

    let mut ext_payloads = String::new();
    let mut ext_backs = String::new();
    for (label, value, x2, x1) in EXT_PAYLOADS {
        let _ = writeln!(
            ext_payloads,
            "    pay(\"{label}\", &{value}, \"{x2}\", \"{x1}\");"
        );
        let _ = writeln!(
            ext_backs,
            "    bk(\"{label}\", &{value}, \"{x2}\", \"{x1}\");"
        );
    }
    let main = rust_main(EXT, "X")
        .replace("{PAYLOADS}", &ext_payloads)
        .replace("{BACKS}", &ext_backs);
    let out = run_crate(
        "kh_rust_ext",
        &deps,
        &[
            ("main.rs", main),
            ("generated.rs", generate(&Backend::Rust, &rust_ext_idl())),
        ],
    );
    assert_eq!(
        count_ok(&out),
        EXT.len() + EXT_PAYLOADS.len() + EXT_PAYLOADS.len(),
        "{out}"
    );
}

/// Key-only payloads of `key_hash_ext.idl` types holding un-annotated, so
/// APPENDABLE, unions (DDS-XTypes 1.3 7.3.1.2.1.8): the union's part sits
/// behind its own DHEADER under XCDR2 (rule 30), as in a sample, and is
/// bare under XCDR1 (rule 29). Hand-derived. `Unions`: `U` = DHEADER 11,
/// disc 2, string "xy"; `UK` (key discriminator, so the discriminator
/// alone) = pad, DHEADER 4, GREEN (1); `tail` 0x77. `Defaults`: `UD` =
/// DHEADER 8, disc 1 (short), pad 2, 0x11223344.
const EXT_PAYLOADS: &[(&str, &str, &str, &str)] = &[
    (
        "Unions",
        "Unions { u: U::B(S(\"xy\")), uk: UK::G(2.5), tail: 0x77 }",
        "0b000000020000000300000078790000040000000100000077",
        "0200000003000000787900000100000077",
    ),
    (
        "Defaults",
        "Defaults { u: UD::A(0x1122_3344) }",
        "080000000100000044332211",
        "0100000044332211",
    ),
];

/// APPENDABLE key-only payloads: a DHEADER under XCDR2 (rule 30), the FINAL
/// form under XCDR1 (rule 29); a nested appendable key member carries its
/// own DHEADER. XCDR2 bodies: Cyclone DDS 0.11's key-only serialization of
/// the same samples (`00090002 060000000d0c0b0a02010000` and
/// `00090003 090000000400000044332211 55000000`, header and trailing
/// padding removed); XCDR1: the members alone, rule 29.
const APPENDABLE_IDL: &str = "
module A {
  @appendable struct AppIds { @id(2) @key long a; @id(1) @key short b; @id(3) long x; };
  @appendable struct AppInner { @key long k; long u; };
  @appendable struct AppOuter { @key AppInner in_; @key octet tag; long x; };
  @mutable union UM switch (long) { case 1: long a; case 2: short b; };
  @final union UF switch (long) { case 1: long a; case 2: short b; };
  @final struct HoldUnionKeys { @key UM m; @key UF f; @key octet t; };
};
";

const APPENDABLE_PAYLOADS: &[KeyPayload] = &[
    (
        "AppIds",
        "AppIds { a: 0x0a0b_0c0d, b: 0x0102, x: 3 }",
        "060000000d0c0b0a0201",
        "0d0c0b0a0201",
        Some((0x0008, "000000060a0b0c0d0102", 0x0000, "0a0b0c0d0102")),
    ),
    (
        "AppOuter",
        "AppOuter { in_: AppInner { k: 0x1122_3344, u: 7 }, tag: 0x55, x: 1 }",
        "09000000040000004433221155",
        "4433221155",
        Some((0x0008, "00000009000000041122334455", 0x0000, "1122334455")),
    ),
    // A MUTABLE union key member: under XCDR2 a DHEADER (16), the
    // discriminator as member 0 with M (EMHEADER1 0xa0000000, LC 2), the
    // case as member 1 (0x20000001); under XCDR1 the two members behind
    // PID_EXTENDED, the discriminator with FLAG_2, then PID_LIST_END (rules
    // 27-28, 7.4.1.2.2, Table 34). A FINAL union key member: the
    // discriminator and the case, nothing else. Then the octet.
    (
        "HoldUnionKeys",
        "HoldUnionKeys { m: UM::A(0x1122_3344), f: UF::B(0x0506), t: 0x55 }",
        "10000000000000a001000000010000204433221102000000060555",
        "017f0800000000400400000001000000017f0800010000000400000044332211027f000002000000060555",
        Some((
            0x0008,
            "00000010a000000000000001200000011122334400000002050655",
            0x0000,
            "7f0100084000000000000004000000017f0100080000000100000004112233447f02000000000002050655",
        )),
    ),
];

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test key_hash_rust -- --ignored"]
fn rust_key_payload_of_appendable_types_has_its_dheader() {
    let hdds = hdds_dir();
    let deps = format!(
        "hdds = {{ path = \"{}\" }}\n",
        hdds.join("crates/hdds").display()
    );
    let mut payloads = String::new();
    let mut backs = String::new();
    let mut both_orders = 0;
    for (label, value, x2, x1, be) in APPENDABLE_PAYLOADS {
        let _ = writeln!(
            payloads,
            "    pay(\"{label}\", &{value}, \"{x2}\", \"{x1}\");"
        );
        let _ = writeln!(backs, "    bk(\"{label}\", &{value}, \"{x2}\", \"{x1}\");");
        if let Some((k2, x2be, k1, x1be)) = be {
            both_orders += 1;
            let _ = writeln!(
                backs,
                "    bkb(\"{label}\", &{value}, {k2:#06x}, \"{x2be}\", {k1:#06x}, \"{x1be}\");"
            );
        }
    }
    let main = rust_main(&[], "A")
        .replace("{PAYLOADS}", &payloads)
        .replace("{BACKS}", &backs);
    let out = run_crate(
        "kh_rust_appendable",
        &deps,
        &[
            ("main.rs", main),
            ("generated.rs", generate(&Backend::Rust, APPENDABLE_IDL)),
        ],
    );
    assert_eq!(
        count_ok(&out),
        APPENDABLE_PAYLOADS.len() + APPENDABLE_PAYLOADS.len() + both_orders,
        "{out}"
    );
}

/// The micro backend maps `char` to `u8`, `wchar` to `u32`, strings to
/// `heapless::String` and bounded sequences to `heapless::Vec`; it has no
/// typedef nor bitmask, so its extended set is narrower.
const MICRO_EXT_IDL: &str = r"
module X {
  enum Color { RED, GREEN, BLUE };
  @bit_bound(8) enum Small { S0, S1, S2 };
  union U switch (long) { case 1: long a; case 2: string b; };
  union UK switch (@key Color) { case RED: long r; case GREEN: double g; case BLUE: octet bl; };
  @final struct Scalars { @key boolean b; @key char c; @key wchar wc; @key int8 i8v; @key short s; @key double d; @key unsigned short us; @key float f; long nk; };
  @final struct Colls { @key sequence<long, 4> sq; @key sequence<octet, 4> bsq; @key long arr[3]; @key wstring<4> ws; };
  @final struct Unions { @key U u; @key UK uk; @key octet tail; };
  @final struct FinalIds { @id(5) @key long a; @id(1) @key octet b; };
  @mutable struct MutSeq { @key long a; @key short b; @key string c; };
  union UBK switch (boolean) { case TRUE: long t; case FALSE: short f; };
  @final struct BoolU { @key UBK u; long x; };
  union UEK switch (Small) { case X::S1: long a; case ::X::S2: short b; default: octet o; };
  @final struct EnumU { @key UEK u; };
  @final struct SeqChars { @key sequence<char, 4> c; @key sequence<wchar, 3> w; long x; };
  @final struct SeqCharU { @key sequence<char> c; long x; };
  struct BaseK { @key long id; long payload; };
  struct DerivedK : BaseK { @key short sub; long more; };
};
@autoid(HASH) module MH {
  @final struct ModHashed { @key long color; @key long shapesize; };
};
";

const MICRO_EXT: &[(&str, &str, &str)] = &[
    (
        "Scalars",
        "Scalars { b: true, c: b'A', wc: 0x5a, i8v: -2, s: 0x0102, d: 1.5, us: 0xbeef, f: -1.0, nk: 9 }",
        "777ac52171394c4abb7947cd8f237982",
    ),
    // REF: 00000002 00000001 00000002 00000003 090807 00 00000001 00000002
    // 00000003 00000004 0068 00e9, MD5 (maximum 4 + 16 + ... > 16).
    (
        "Colls",
        "Colls { sq: V(&[1, 2]), bsq: V(&[9, 8, 7]), arr: [1, 2, 3], ws: V(&[0x68, 0xe9]) }",
        "c7774e0d0f0225342bcd922a9d9d70bb",
    ),
    (
        "Unions",
        "Unions { u: U::B(S(\"xy\")), uk: UK::G(2.5), tail: 0x77 }",
        "45ec7942ba3bf7038d5ebc3abfc11459",
    ),
    (
        "FinalIds",
        "FinalIds { a: 0x0a0b_0c0d, b: 1 }",
        "010000000a0b0c0d0000000000000000",
    ),
    // REF: @mutable without @id, sequential ids, declaration order.
    (
        "MutSeq",
        "MutSeq { a: 1, b: 2, c: S(\"Q\") }",
        "ae5426ae810065bd695fddf3baf7b5c1",
    ),
    // REF: union keys, boolean discriminator (1 byte) and enumerator
    // labels written scoped; the default case takes the smallest free
    // value (S0).
    (
        "BoolUT",
        "BoolU { u: UBK::T(0x1122_3344), x: 1 }",
        "01000000112233440000000000000000",
    ),
    (
        "BoolUF",
        "BoolU { u: UBK::F(0x22), x: 1 }",
        "00000022000000000000000000000000",
    ),
    (
        "EnumUA",
        "EnumU { u: UEK::A(0x55) }",
        "01000000000000550000000000000000",
    ),
    (
        "EnumUB",
        "EnumU { u: UEK::B(0x33) }",
        "02000033000000000000000000000000",
    ),
    (
        "EnumUDef",
        "EnumU { u: UEK::O(0x44) }",
        "00440000000000000000000000000000",
    ),
    // Cyclone DDS 0.11: sequence<char> / sequence<wchar> keys.
    (
        "SeqChars",
        "SeqChars { c: V(&[b'x', b'y']), w: V(&[0x41, 0x42]), x: 1 }",
        "7cfc7a60d95c3037ff7785cfd6822d1c",
    ),
    (
        "SeqCharU",
        "SeqCharU { c: V(&[b'x', b'y']), x: 1 }",
        "62a27c5288ec9d48152240ad0670d151",
    ),
    // Cyclone DDS 0.11: module-level @autoid(HASH).
    (
        "ModHashed",
        "generated::MH::ModHashed { color: 0x11, shapesize: 0x22 }",
        "00000022000000110000000000000000",
    ),
    // The micro KeyHash does not read base members: a key declared in the
    // base is refused (all zeros), and the file still compiles.
    (
        "DerivedK",
        "DerivedK { base: BaseK { id: 1, payload: 2 }, sub: 3, more: 4 }",
        "00000000000000000000000000000000",
    ),
];

fn micro_main(cases: &[(&str, &str, &str)], module: &str) -> String {
    let mut body = String::new();
    for (label, value, expected) in cases {
        let _ = writeln!(
            body,
            "    chk(\"{label}\", {value}.compute_key(), \"{expected}\");"
        );
    }
    format!(
        "#![allow(dead_code, unused_imports, non_snake_case, non_camel_case_types, clippy::all)]\n\
         #![deny(unused_parens)]\n\
         #[path = \"generated.rs\"]\nmod generated;\n\
         use generated::{module}::*;\n\
         fn S<const N: usize>(s: &str) -> heapless::String<N> {{ heapless::String::try_from(s).expect(\"fits\") }}\n\
         fn V<T: Clone, const N: usize>(v: &[T]) -> heapless::Vec<T, N> {{ heapless::Vec::from_slice(v).expect(\"fits\") }}\n\
         fn hex(b: &[u8]) -> String {{ b.iter().map(|x| format!(\"{{x:02x}}\")).collect() }}\n\
         static mut FAILS: u32 = 0;\n\
         fn chk(label: &str, got: [u8; 16], expected: &str) {{\n\
         \x20   let got = hex(&got);\n\
         \x20   if got == expected {{ println!(\"OK {{label}}\"); }} else {{\n\
         \x20       println!(\"FAIL {{label}}: got {{got}} expected {{expected}}\");\n\
         \x20       unsafe {{ FAILS += 1; }}\n\
         \x20   }}\n\
         }}\n\
         fn main() {{\n{body}\
         \x20   if unsafe {{ FAILS }} != 0 {{ std::process::exit(1); }}\n\
         }}\n"
    )
}

#[test]
#[ignore = "needs an HDDS checkout: HDDSGEN_HDDS_DIR=<hdds> cargo test --test key_hash_rust -- --ignored"]
fn micro_backend_key_hash_matches_the_golden_vectors() {
    let hdds = hdds_dir();
    let deps = format!(
        "hdds-micro = {{ path = \"{}\", features = [\"heapless\"] }}\nheapless = \"0.8\"\n",
        hdds.join("crates/hdds-micro").display()
    );
    let generated = generate(&Backend::Micro, GOLDEN_IDL);
    // The generated file, KeyHash writer included, builds in a `no_std`
    // library: the writer needs nothing but `core`.
    run_crate(
        "kh_micro_nostd",
        &deps,
        &[
            (
                "lib.rs",
                "#![no_std]\n#[allow(clippy::all)]\n#[path = \"generated.rs\"]\npub mod generated;\n"
                    .to_string(),
            ),
            ("generated.rs", generated.clone()),
        ],
    );
    let (mods, extras) = extra_files(&Backend::Micro, true);
    let main = micro_main(GOLDEN, "P").replace(
        "fn main() {\n",
        &format!(
            "{mods}fn main() {{\n    chk(\"Box1\", extra_2::Box1 {{ id: 7, x: 1 }}.compute_key(), \"00000007000000000000000000000000\");\n    \
             chk(\"HolderRefused\", extra_3::Holder {{ d: extra_3::DerivedK {{ base: extra_3::BaseK {{ id: 1, payload: 2 }}, sub: 3, more: 4 }} }}.compute_key(), \"00000000000000000000000000000000\");\n"
        ),
    );
    let mut files = vec![
        ("main.rs".to_string(), main),
        ("generated.rs".to_string(), generated),
    ];
    files.extend(extras);
    let files: Vec<(&str, String)> = files.iter().map(|(f, c)| (f.as_str(), c.clone())).collect();
    let out = run_crate("kh_micro_golden", &deps, &files);
    assert_eq!(count_ok(&out), GOLDEN.len() + 2, "{out}");

    let out = run_crate(
        "kh_micro_ext",
        &deps,
        &[
            ("main.rs", micro_main(MICRO_EXT, "X")),
            ("generated.rs", generate(&Backend::Micro, MICRO_EXT_IDL)),
        ],
    );
    assert_eq!(count_ok(&out), MICRO_EXT.len(), "{out}");
}
