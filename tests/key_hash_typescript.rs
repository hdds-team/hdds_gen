// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Instance `KeyHash` of generated TypeScript (`computeKey<T>()`), generated
//! and run with `node --experimental-transform-types` (RTPS 2.5 9.6.4.8).
//! Node strips the types without checking them: this test proves the
//! values, not the typing. Skipped, with a message, without a `node` that
//! runs TypeScript.
//!
//! Expected values: the vectors of `tests/key_hash_rust.rs` (SPEC and
//! REF), plus a derived type whose key member is declared in its base
//! (the vector of `tests/key_hash_python.rs`, REF): a derived interface
//! extends its base's, so the base key member is read from the same object.

#![allow(
    clippy::expect_used,
    clippy::needless_raw_string_hashes,
    clippy::needless_pass_by_value
)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::{Backend, Parser};

const GOLDEN_IDL: &str = include_str!("fixtures/key_hash_golden.idl");
const EXT_IDL: &str = include_str!("fixtures/key_hash_ext.idl");
const CHECKS_TS: &str = r#"
let fails = 0;
function hex(b: Uint8Array): string { return Array.from(b).map((x) => x.toString(16).padStart(2, "0")).join(""); }
function chk(label: string, got: Uint8Array, expected: string): void {
  const h = hex(got);
  if (h === expected) { console.log(`OK ${label}`); } else { console.log(`FAIL ${label}: got ${h} expected ${expected}`); fails++; }
}
"#;
const GOLDEN_TS: &str = r#"chk("LongKey", P.computeKeyLongKey({ id: 0x12345678, x: 9 }), "12345678000000000000000000000000");
chk("SpecEx2", P.computeKeySpecEx2({ label: "BLUE", id: 0x123456789abcdef0n, x: 1, y: 2 }), "f91a59e32e4535d9a69cd5d9f5b6e36e");
chk("SpecEx3", P.computeKeySpecEx3({ label: "BLUE", m_nested: { m_long: 0x12345678, u: 1, w: 2 }, x: 1, y: 2 }), "374b96e2e727237f016cc4cebb6eb71e");
chk("StrKey", P.computeKeyStrKey({ label: "BLUE", x: 1 }), "cac217c318363f8ef1160eeedef9e886");
chk("StrKey8", P.computeKeyStrKey8({ label: "BLUE", x: 1 }), "00000005424c55450000000000000000");
chk("Str11", P.computeKeyStr11({ label: "BLUE", x: 1 }), "00000005424c55450000000000000000");
chk("Str12", P.computeKeyStr12({ label: "BLUE", x: 1 }), "cac217c318363f8ef1160eeedef9e886");
chk("ShapeType", P.computeKeyShapeType({ color: "BLUE", x: 1, y: 2, shapesize: 3 }), "cac217c318363f8ef1160eeedef9e886");
chk("TwoKey", P.computeKeyTwoKey({ a: 1, b: 2, x: 3 }), "00000001000200000000000000000000");
chk("TwoKeyId", P.computeKeyTwoKeyId({ a: 1, b: 2, x: 3 }), "00020000000000010000000000000000");
chk("Outer", P.computeKeyOuter({ in_: { k: 0x11223344, u: 7 }, tag: 0x55, x: 1 }), "11223344550000000000000000000000");
chk("OuterU", P.computeKeyOuter({ in_: { k: 0x11223344, u: -7 }, tag: 0x55, x: 2 }), "11223344550000000000000000000000");
chk("OuterNoKey", P.computeKeyOuterNoKey({ p: { a: 1, b: 2 }, x: 1 }), "00000001000000020000000000000000");
chk("AlignKey", P.computeKeyAlignKey({ t: 1, v: 2n }), "01000000000000000000000200000000");
chk("Key16", P.computeKeyKey16({ a: 1n, b: 2n }), "00000000000000010000000000000002");
chk("CapKey", P.computeKeyCapKey({ t: 1, v: 2n, c: 3 }), "01000000000000000000000200000003");
chk("BigKey", P.computeKeyBigKey({ a: 1n, b: 2n, c: 3 }), "8898488e44b5e511051f83ce5e917bb8");
chk("StrKey55", P.computeKeyStrKey({ label: "Q".repeat(50), x: 1 }), "fa67225c93f8071a5bbcf6df1bf9bfcd");
chk("StrKey56", P.computeKeyStrKey({ label: "Q".repeat(51), x: 1 }), "424bcbae3ad0fa3c57426e73ad1261af");
chk("StrKey64", P.computeKeyStrKey({ label: "Q".repeat(59), x: 1 }), "4b60eb0dd74deb107a2c9cad4e781130");
chk("StrKey65", P.computeKeyStrKey({ label: "Q".repeat(60), x: 1 }), "e7c43e8aa8fbb48e72bc38d01eb741a5");
if (fails !== 0) { process.exit(1); }
"#;
const EXT_TS: &str = r#"chk("Scalars", X.computeKeyScalars({ b: true, c: "A", wc: "Z", i8v: -2, s: 0x0102, d: 1.5, us: 0xbeef, f: -1.0, nk: 9 }), "777ac52171394c4abb7947cd8f237982");
chk("Enums", X.computeKeyEnums({ c: X.Color.BLUE, sm: X.Small.S2, md: X.Mid.M1, f8: 3 as X.Flags8, f32: 2 as X.Flags32, f64: (2 ** 40) as X.Flags64, ml: -5 }), "e2265fcd85db8fc8343f51c33f672a8b");
chk("Colls", X.computeKeyColls({ sq: [1, 2], bsq: [9, 8, 7], arr: [1, 2, 3], oct2: [[1, 2], [3, 4]], ws: "hé", n6: "abc" }), "7b44e4a1ef2ab87d5bc2b6b8d253beca");
chk("Unions", X.computeKeyUnions({ u: { discriminator: 2, b: "xy" }, uk: { discriminator: X.Color.GREEN, g: 2.5 }, tail: 0x77 }), "45ec7942ba3bf7038d5ebc3abfc11459");
chk("Hashed", X.computeKeyHashed({ color: 0x11, shapesize: 0x22 }), "00000022000000110000000000000000");
chk("FinalIds", X.computeKeyFinalIds({ a: 0x0a0b0c0d, b: 1 }), "010000000a0b0c0d0000000000000000");
chk("MutSeq", X.computeKeyMutSeq({ a: 1, b: 2, c: "Q" }), "ae5426ae810065bd695fddf3baf7b5c1");
chk("SeqChars", X.computeKeySeqChars({ c: ["x", "y"], w: ["A", "B"], x: 1 }), "7cfc7a60d95c3037ff7785cfd6822d1c");
chk("SeqCharU", X.computeKeySeqCharU({ c: ["x", "y"], x: 1 }), "62a27c5288ec9d48152240ad0670d151");
chk("ModHashed", MH.computeKeyModHashed({ color: 0x11, shapesize: 0x22 }), "00000022000000110000000000000000");
chk("DefaultsCase", X.computeKeyDefaults({ u: { discriminator: 1, a: 5 } }), "00010000000000050000000000000000");
chk("DefaultsDefault", X.computeKeyDefaults({ u: { discriminator: 7, o: 0x42 } }), "00074200000000000000000000000000");
if (fails !== 0) { process.exit(1); }
"#;

const INHERITANCE_IDL: &str = "
struct Base { @key long id; long payload; };
struct Derived : Base { @key short sub; long more; };
";
const INHERITANCE_TS: &str = r#"chk("Derived", computeKeyDerived({ id: 7, payload: 1, sub: 3, more: 4 }), "00000007000300000000000000000000");
chk("BaseIgnoresPayload", computeKeyBase({ id: 1, payload: 3 }), hex(computeKeyBase({ id: 1, payload: 2 })));
if (fails !== 0) { process.exit(1); }
"#;

const NODE_ARGS: [&str; 2] = ["--experimental-transform-types", "--no-warnings"];

fn generate(idl: &str) -> String {
    let mut p = Parser::try_new(idl).expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::TypeScript
        .generator()
        .generate(&ast)
        .expect("codegen")
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

fn run(dir: &Path, name: &str, generated: String, checks: &str) -> usize {
    let file = dir.join(format!("{name}.ts"));
    fs::write(&file, format!("{generated}{CHECKS_TS}{checks}")).expect("source");
    let out = Command::new("node")
        .args(NODE_ARGS)
        .arg(&file)
        .output()
        .expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{name}:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout.lines().filter(|l| l.starts_with("OK ")).count()
}

#[test]
fn typescript_key_hash_matches_the_golden_vectors() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key_hash_typescript");
    fs::create_dir_all(&dir).expect("dir");
    if !node_runs_typescript(&dir) {
        eprintln!(
            "node with --experimental-transform-types not found; TypeScript KeyHash test not run"
        );
        return;
    }
    assert_eq!(run(&dir, "golden", generate(GOLDEN_IDL), GOLDEN_TS), 21);
    assert_eq!(run(&dir, "ext", generate(EXT_IDL), EXT_TS), 12);
    assert_eq!(
        run(&dir, "inh", generate(INHERITANCE_IDL), INHERITANCE_TS),
        2
    );
}
