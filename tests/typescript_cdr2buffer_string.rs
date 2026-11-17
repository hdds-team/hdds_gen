// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The generated `Cdr2Buffer` keeps `readString()` and `writeString()` (the
//! 1.0.12 signatures) as deprecated aliases of its current string methods:
//! the same wire format (XCDR2: a length counting the NUL, the UTF-8 bytes,
//! the NUL), no second implementation. The runtime part runs the generated
//! file under `node --experimental-transform-types`; skipped, with a
//! message, without a `node` that runs TypeScript.

#![allow(clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::Command;

use hddsgen::{Backend, Parser};

const NODE_ARGS: [&str; 2] = ["--experimental-transform-types", "--no-warnings"];

fn generate() -> String {
    let mut p = Parser::try_new("struct Msg { long x; };").expect("lexer");
    let ast = p.parse().expect("parse");
    Backend::TypeScript
        .generator()
        .generate(&ast)
        .expect("codegen")
}

#[test]
fn cdr2buffer_exposes_deprecated_read_and_write_string() {
    let src = generate();
    let class_at = src.find("export class Cdr2Buffer").expect("class");
    let class_end = src[class_at..]
        .find("\nexport class Cdr1Buffer")
        .map_or(src.len(), |n| class_at + n);
    let class = &src[class_at..class_end];
    assert!(
        class.contains("readString(): string {"),
        "readString missing"
    );
    assert!(
        class.contains("writeString(s: string): void {"),
        "writeString missing"
    );
    assert_eq!(
        class.matches("@deprecated Use getString instead").count(),
        1,
        "readString must be marked deprecated in favour of getString"
    );
    assert_eq!(
        class.matches("@deprecated Use putString instead").count(),
        1,
        "writeString must be marked deprecated in favour of putString"
    );
}

#[test]
fn cdr2buffer_string_round_trip() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("typescript_cdr2buffer_string");
    fs::create_dir_all(&dir).expect("dir");
    let probe = dir.join("probe.ts");
    fs::write(
        &probe,
        "namespace N { export enum E { A } }\nconsole.log(N.E.A);\n",
    )
    .expect("probe");
    if !Command::new("node")
        .args(NODE_ARGS)
        .arg(&probe)
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("node with --experimental-transform-types not found; round trip not run");
        return;
    }
    let mut src = generate();
    src.push_str(
        r#"
const samples = ["", "hello", "h\u00e9llo \u{1F600}"];
const buf = new Cdr2Buffer(new Uint8Array(256));
for (const s of samples) { buf.writeString(s); }
// The bytes are those of putString: length with the NUL, UTF-8, NUL.
const ref = new Cdr2Buffer(new Uint8Array(256));
for (const s of samples) { ref.putString(s, 0); }
const a = buf.toBytes();
const b = ref.toBytes();
if (a.length !== b.length || a.some((v, i) => v !== b[i])) { console.log("FAIL bytes differ from putString"); process.exit(1); }
const rd = new Cdr2Buffer(a);
for (const s of samples) {
  const got = rd.readString();
  if (got !== s) { console.log("FAIL round trip " + JSON.stringify(s) + " -> " + JSON.stringify(got)); process.exit(1); }
}
if (rd.position !== a.length) { console.log("FAIL not every byte consumed"); process.exit(1); }
console.log("OK");
"#,
    );
    let file = dir.join("round_trip.ts");
    fs::write(&file, src).expect("source");
    let out = Command::new("node")
        .args(NODE_ARGS)
        .arg(&file)
        .output()
        .expect("node");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("OK"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
