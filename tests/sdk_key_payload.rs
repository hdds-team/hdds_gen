// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Key-only DATA serializes the `KeyHolder` in declaration order and with
//! the original extensibility (DDS-XTypes 1.3 7.2.2.4.7, RTPS 2.5 9.4.5.3).
//! The protected hash always uses MD5 of CDR BE, including 8-byte alignment
//! (DDS-Security 1.1 7.3.4), unlike the normal `PLAIN_CDR2` `KeyHash`.

#![allow(clippy::expect_used, clippy::panic)]

use hddsgen::{Backend, Parser};
use std::{fs, path::Path, process::Command};

const IDL: &str = "
module m { @final struct T { @key long id; }; };
@final struct EncoderNames { @key long saved__xenc; @key long saved_encode_cdr2_le_at; @key long saved_hdds_enc; };
@final struct Align { @key octet t; long ignored; @key long long v; };
@final struct Text { @key string<12> label; @key long long id; long ignored; };
@appendable struct Inner { @key long k; long ignored; };
@mutable struct Reordered { @id(9) @key long a; @id(1) @key short b; long ignored; };
@final struct Outer { @key Inner nested; long ignored; };
@mutable struct Plain { long a; long b; };
@final struct AllNested { @key Plain nested; long ignored; };
@final union Choice switch (@key long) { case 1: long value; default: string text; };
@final struct UnionKey { @key Choice pick; long ignored; };
";
const ALIGN: &str = "010000000200000000000000";
const INNER: &str = "0400000003000000";
const REORDERED: &str = "0e000000090000a001000000010000900200";
const ALL_NESTED: &str = "1000000000000020010000000100002002000000";
const TEXT_PROTECTED: &str = "a49a087bc2bb7cd4bcd8b8bfdf5e8761";
const TEXT_PAYLOAD: &str = "05000000424c5545000000000200000000000000";
const PROTECTED: &str = "85730d43c28ae59cf2f12ddf0ed4a912";

fn generate(backend: &Backend) -> String {
    let ast = Parser::try_new(IDL).expect("lexer").parse().expect("IDL");
    backend.generator().generate(&ast).expect("generate")
}

fn run(cmd: &mut Command) {
    let out = cmd.output().expect("run tool");
    assert!(
        out.status.success(),
        "{cmd:?}\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn compile(dir: &Path, tool: &str, filename: &str, source: &str) {
    fs::write(dir.join(filename), source).expect("driver");
    run(Command::new(tool)
        .current_dir(dir)
        .args(["-Wall", "-Wextra", "-Werror", filename, "-o", "driver"]));
    run(&mut Command::new(dir.join("driver")));
}

#[test]
fn c_key_payload_and_protected_hash_follow_the_spec() {
    c_vectors(ALL_NESTED);
}

fn c_vectors(nested: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("types.h"), generate(&Backend::C)).expect("header");
    let driver = format!(
        r#"#include "types.h"
#include <assert.h>
#include <stdio.h>
static void eq(const uint8_t* bytes, int n, const char* expected) {{
    char hex[512]; int i; assert(n >= 0 && n < 256);
    for (i = 0; i < n; ++i) sprintf(hex + i*2, "%02x", bytes[i]);
    hex[n*2] = 0; assert(strcmp(hex, expected) == 0);
}}
struct HddsDataWriter *hdds_writer_create_keyed(struct HddsParticipant *p, const char *topic, const char *type, const struct HddsQoS *q, uint32_t ext) {{
    (void)p; (void)topic; (void)q; assert(ext == 0); assert(strcmp(type, strcmp(topic, "scoped") == 0 ? "m::T" : "Align") == 0); return NULL;
}}
struct HddsDataWriter *hdds_publisher_create_writer_keyed(struct HddsPublisher *p, const char *topic, const char *type, const struct HddsQoS *q, uint32_t ext) {{
    (void)p; (void)topic; (void)q; assert(ext == 0); assert(strcmp(type, strcmp(topic, "scoped") == 0 ? "m::T" : "Align") == 0); return NULL;
}}
int main(void) {{
    assert(align_create_keyed_writer(NULL, "topic", NULL) == NULL);
    assert(align_create_keyed_writer_from_publisher(NULL, "topic", NULL) == NULL);
    assert(m_t_create_keyed_writer(NULL, "scoped", NULL) == NULL);
    assert(m_t_create_keyed_writer_from_publisher(NULL, "scoped", NULL) == NULL);
    uint8_t b[256]; int n;
    EncoderNames names = {{1, 2, 3}};
    n = encodernames_encode_key_cdr2_le(&names, b, sizeof(b)); eq(b, n, "010000000200000003000000");
    Align a = {{1, 99, 2}};
    Inner in = {{3, 99}};
    Reordered r = {{1, 2, 99}};
    Outer o; UnionKey u; AllNested all; Text text;
    memset(&text, 0, sizeof(text)); strcpy(text.label, "BLUE"); text.id=2; text.ignored=99;
    n = align_encode_key_cdr2_le(&a, b, sizeof(b)); eq(b, n, "{ALIGN}");
    n = inner_encode_key_cdr2_le(&in, b, sizeof(b)); eq(b, n, "{INNER}");
    n = reordered_encode_key_cdr2_le(&r, b, sizeof(b)); eq(b, n, "{REORDERED}");
    o.nested = in; o.ignored = 99;
    n = outer_encode_key_cdr2_le(&o, b, sizeof(b)); eq(b, n, "{INNER}");
    u.pick._d = 1; u.pick._u.value = 99; u.ignored = 99;
    n = unionkey_encode_key_cdr2_le(&u, b, sizeof(b)); eq(b, n, "01000000");
    all.nested.a=1; all.nested.b=2; all.ignored=99;
    n = allnested_encode_key_cdr2_le(&all, b, sizeof(b)); eq(b, n, "{nested}");
    assert(choice_compute_key(&u.pick, b) == 1); eq(b, 16, "00000001000000000000000000000000");
    n = text_encode_key_cdr2_le(&text, b, sizeof(b)); eq(b, n, "{TEXT_PAYLOAD}");
    assert(text_compute_protected_key(&text, b) == 1); eq(b, 16, "{TEXT_PROTECTED}");
    assert(align_compute_protected_key(&a, b) == 1); eq(b, 16, "{PROTECTED}");
    return 0;
}}
"#
    );
    compile(dir.path(), "cc", "driver.c", &driver);
}

#[test]
fn cpp_key_payload_and_protected_hash_follow_the_spec() {
    cpp_vectors(ALL_NESTED);
}

fn cpp_vectors(nested: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("types.hpp"), generate(&Backend::Cpp)).expect("header");
    let driver = format!(
        r#"#include "types.hpp"
#include <cassert>
#include <cstdio>
static void eq(const uint8_t* bytes, int n, const char* expected) {{
    char hex[512]; assert(n >= 0 && n < 256);
    for (int i = 0; i < n; ++i) std::sprintf(hex + i*2, "%02x", bytes[i]);
    hex[n*2] = 0; assert(std::strcmp(hex, expected) == 0);
}}
int main() {{
    uint8_t b[256]; Align a; a.t=1; a.ignored=99; a.v=2;
    EncoderNames names; names.saved__xenc=1; names.saved_encode_cdr2_le_at=2; names.saved_hdds_enc=3;
    eq(b, names.encode_key_cdr2_le(b, sizeof(b)), "010000000200000003000000");
    Inner in; in.k=3; in.ignored=99;
    Reordered r; r.a=1; r.b=2; r.ignored=99;
    Outer o; o.nested=in; o.ignored=99;
    AllNested all; all.nested.a=1; all.nested.b=2; all.ignored=99;
    Text text; text.label.fill(0); std::memcpy(text.label.data(), "BLUE", 4); text.id=2; text.ignored=99;
    UnionKey u; u.pick._d=1; u.pick.value=99; u.ignored=99;
    eq(b, a.encode_key_cdr2_le(b, sizeof(b)), "{ALIGN}");
    eq(b, in.encode_key_cdr2_le(b, sizeof(b)), "{INNER}");
    eq(b, r.encode_key_cdr2_le(b, sizeof(b)), "{REORDERED}");
    eq(b, o.encode_key_cdr2_le(b, sizeof(b)), "{INNER}");
    eq(b, u.encode_key_cdr2_le(b, sizeof(b)), "01000000");
    eq(b, all.encode_key_cdr2_le(b, sizeof(b)), "{nested}");
    assert(u.pick.hdds_compute_key(b, false)); eq(b, 16, "00000001000000000000000000000000");
    eq(b, text.encode_key_cdr2_le(b, sizeof(b)), "{TEXT_PAYLOAD}");
    assert(text.hdds_compute_key(b, true)); eq(b, 16, "{TEXT_PROTECTED}");
    bool (*legacy)(const void*, std::uint8_t*) noexcept = &AlignPubSubType::compute_key;
    assert(legacy(&a, b)); eq(b, 16, "01000000000000000000000200000000");
    assert(AlignPubSubType::compute_protected_key(&a, b)); eq(b, 16, "{PROTECTED}");
    assert(a.hdds_compute_key(b, true)); eq(b, 16, "{PROTECTED}");
    return 0;
}}
"#
    );
    compile(dir.path(), "c++", "driver.cpp", &driver);
}

#[test]
fn python_key_payload_and_protected_hash_follow_the_spec() {
    python_vectors(ALL_NESTED);
}

fn python_vectors(nested: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("types.py"), generate(&Backend::Python)).expect("module");
    let driver = "from generated import *\n".to_string();
    fs::rename(dir.path().join("types.py"), dir.path().join("generated.py")).expect("rename");
    let driver = driver
        + &format!(
            r#"
assert Align.type_name() == "Align"
names = EncoderNames(saved__xenc=1, saved_encode_cdr2_le_at=2, saved_hdds_enc=3)
assert names.encode_key_cdr2_le().hex() == "010000000200000003000000"
a = Align(t=1, ignored=99, v=2)
i = Inner(k=3, ignored=99)
assert a.encode_key_cdr2_le().hex() == '{ALIGN}'
assert i.encode_key_cdr2_le().hex() == '{INNER}'
assert Reordered(a=1, b=2, ignored=99).encode_key_cdr2_le().hex() == '{REORDERED}'
assert Outer(nested=i, ignored=99).encode_key_cdr2_le().hex() == '{INNER}'
assert UnionKey(pick=Choice(_discriminator=1, _value=99), ignored=99).encode_key_cdr2_le().hex() == '01000000'
assert AllNested(nested=Plain(a=1, b=2), ignored=99).encode_key_cdr2_le().hex() == '{nested}'
assert Choice(_discriminator=1, _value=99).compute_key().hex() == '00000001000000000000000000000000'
assert Text(label='BLUE', id=2, ignored=99).encode_key_cdr2_le().hex() == '{TEXT_PAYLOAD}'
assert Text(label='BLUE', id=2, ignored=99).compute_key(protected_key=True).hex() == '{TEXT_PROTECTED}'
assert a.compute_key(protected_key=True).hex() == '{PROTECTED}'
"#
        );
    fs::write(dir.path().join("driver.py"), driver).expect("driver");
    run(Command::new("python3")
        .current_dir(dir.path())
        .arg("driver.py"));
}

#[test]
fn typescript_key_payload_and_protected_hash_follow_the_spec() {
    typescript_vectors(ALL_NESTED);
}

fn typescript_vectors(nested: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(
        dir.path().join("generated.ts"),
        generate(&Backend::TypeScript),
    )
    .expect("module");
    let driver = format!(
        r#"import * as T from './generated.ts';
import assert from 'node:assert/strict';
function payload(fn: any, value: any): string {{ const b = new T.Cdr2Buffer(new Uint8Array(256)); fn(value, b); return Buffer.from(b.toBytes()).toString('hex'); }}
const a = {{t:1, ignored:99, v:2n}}, i = {{k:3, ignored:99}};
assert.equal(payload(T.encodeKeyEncoderNames, {{saved__xenc:1, saved_encode_cdr2_le_at:2, saved_hdds_enc:3}}), "010000000200000003000000");
assert.equal(payload(T.encodeKeyAlign, a), '{ALIGN}');
assert.equal(payload(T.encodeKeyInner, i), '{INNER}');
assert.equal(payload(T.encodeKeyReordered, {{a:1, b:2, ignored:99}}), '{REORDERED}');
assert.equal(payload(T.encodeKeyOuter, {{nested:i, ignored:99}}), '{INNER}');
assert.equal(payload(T.encodeKeyUnionKey, {{pick:{{discriminator:1, value:99}}, ignored:99}}), '01000000');
assert.equal(payload(T.encodeKeyAllNested, {{nested:{{a:1, b:2}}, ignored:99}}), '{nested}');
assert.equal(Buffer.from(T.computeKeyChoice({{discriminator:1, value:99}})).toString('hex'), '00000001000000000000000000000000');
assert.equal(payload(T.encodeKeyText, {{label:'BLUE', id:2n, ignored:99}}), '{TEXT_PAYLOAD}');
assert.equal(Buffer.from(T.computeKeyText({{label:'BLUE', id:2n, ignored:99}}, true)).toString('hex'), '{TEXT_PROTECTED}');
assert.equal(Buffer.from(T.computeKeyAlign(a, true)).toString('hex'), '{PROTECTED}');
"#
    );
    fs::write(dir.path().join("driver.ts"), driver).expect("driver");
    run(Command::new("node").current_dir(dir.path()).args([
        "--experimental-transform-types",
        "--no-warnings",
        "driver.ts",
    ]));
}

/// Run the generated Rust implementation as the byte-for-byte reference,
/// then execute every SDK encoder against those bytes. Run this ignored
/// test binary directly: its child Cargo command takes the shared lock.
#[test]
#[ignore = "needs HDDSGEN_HDDS_DIR and a populated Rust dependency cache"]
fn nested_mutable_key_payload_matches_generated_rust() {
    let hdds = std::env::var("HDDSGEN_HDDS_DIR").expect("HDDS checkout");
    assert!(Path::new(&hdds).join("crates/hdds/Cargo.toml").is_file());
    let dir = tempfile::tempdir().expect("reference crate");
    fs::create_dir(dir.path().join("src")).expect("source directory");
    let dependency = Path::new(&hdds).join("crates/hdds");
    fs::write(dir.path().join("Cargo.toml"), format!(
        "[package]\nname = \"key_payload_reference\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n[dependencies]\nhdds = {{ path = {dependency:?} }}\n")).expect("manifest");
    fs::write(
        dir.path().join("src/generated.rs"),
        generate(&Backend::Rust),
    )
    .expect("Rust codec");
    fs::write(
        dir.path().join("src/main.rs"),
        r#"
mod generated;
use hdds::DdsTrait;
fn main() {
    let value = generated::AllNested { nested: generated::Plain { a: 1, b: 2 }, ignored: 99 };
    for byte in value.encode_key(hdds::CdrVersion::Xcdr2) { print!("{byte:02x}"); }
}
"#,
    )
    .expect("reference driver");
    let output = Command::new("cargo")
        .args(["run", "--offline", "--quiet"])
        .current_dir(dir.path())
        .env("CARGO_TARGET_DIR", Path::new(&hdds).join("target"))
        .output()
        .expect("Rust reference");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = String::from_utf8(output.stdout).expect("hex bytes");
    assert_eq!(bytes, ALL_NESTED);
    c_vectors(&bytes);
    cpp_vectors(&bytes);
    python_vectors(&bytes);
    typescript_vectors(&bytes);
}
