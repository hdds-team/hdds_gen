// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Integration tests for code generation backends.

#![allow(
    clippy::expect_used,
    clippy::needless_raw_string_hashes,
    clippy::needless_pass_by_value,
    clippy::single_char_pattern
)]

use hddsgen::{Backend, Parser};

fn parse(idl: &str) -> hddsgen::IdlFile {
    let mut p = Parser::try_new(idl).expect("lexer error");
    p.parse().expect("parse error")
}

fn gen(backend: Backend, idl: &str) -> String {
    let ast = parse(idl);
    backend.generator().generate(&ast).expect("codegen error")
}

// ---------------------------------------------------------------------------
// Rust backend
// ---------------------------------------------------------------------------

#[test]
fn rust_simple_struct() {
    let out = gen(Backend::Rust, "struct Point { int32_t x; int32_t y; };");
    assert!(out.contains("struct Point"), "missing struct: {out}");
    assert!(out.contains("i32"), "missing i32: {out}");
}

#[test]
fn rust_enum() {
    let out = gen(Backend::Rust, "enum Color { RED, GREEN, BLUE };");
    assert!(out.contains("enum Color"), "missing enum: {out}");
    assert!(out.contains("RED"), "missing variant: {out}");
}

#[test]
fn rust_typedef() {
    let out = gen(Backend::Rust, "typedef long MyLong;");
    assert!(out.contains("MyLong"), "missing typedef: {out}");
}

#[test]
fn rust_optional_field() {
    let out = gen(Backend::Rust, "struct S { @optional int32_t maybe; };");
    assert!(out.contains("Option"), "missing Option: {out}");
}

#[test]
fn rust_sequence_field() {
    let out = gen(Backend::Rust, "struct S { sequence<int32_t> items; };");
    assert!(out.contains("Vec"), "missing Vec: {out}");
}

#[test]
fn rust_array_field() {
    let out = gen(Backend::Rust, "struct S { int32_t data[5]; };");
    assert!(out.contains("[i32; 5]"), "missing array type: {out}");
}

#[test]
fn rust_all_primitives() {
    let idl = r#"
        struct AllPrims {
            boolean b;
            octet o;
            char c;
            int8_t i8;
            int16_t i16;
            int32_t i32;
            int64_t i64;
            uint8_t u8;
            uint16_t u16;
            uint32_t u32;
            uint64_t u64;
            float f;
            double d;
            string s;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(out.contains("bool"), "missing bool: {out}");
    assert!(out.contains("f32"), "missing f32: {out}");
    assert!(out.contains("f64"), "missing f64: {out}");
    assert!(out.contains("String"), "missing String: {out}");
}

#[test]
fn rust_mutable_struct() {
    let out = gen(Backend::Rust, "@mutable struct S { int32_t x; };");
    // Should compile - mutable doesn't change Rust output drastically
    assert!(out.contains("struct S"), "missing struct: {out}");
}

// ---------------------------------------------------------------------------
// TypeScript backend
// ---------------------------------------------------------------------------

#[test]
fn ts_interface() {
    let out = gen(
        Backend::TypeScript,
        "struct Point { int32_t x; int32_t y; };",
    );
    assert!(
        out.contains("interface") || out.contains("Point"),
        "missing interface: {out}"
    );
}

#[test]
fn ts_enum() {
    let out = gen(Backend::TypeScript, "enum Color { RED, GREEN, BLUE };");
    assert!(out.contains("Color"), "missing enum: {out}");
    assert!(out.contains("RED"), "missing variant: {out}");
}

#[test]
fn ts_optional_field() {
    let out = gen(
        Backend::TypeScript,
        "struct S { @optional int32_t maybe; };",
    );
    // TS optional is usually "?" or "| undefined"
    assert!(
        out.contains("?") || out.contains("undefined") || out.contains("null"),
        "missing optional marker: {out}"
    );
}

#[test]
fn ts_sequence_to_array() {
    let out = gen(
        Backend::TypeScript,
        "struct S { sequence<int32_t> items; };",
    );
    assert!(
        out.contains("Array") || out.contains("[]") || out.contains("number"),
        "missing array type: {out}"
    );
}

#[test]
fn ts_map_to_record_or_map() {
    let out = gen(Backend::TypeScript, "struct S { map<string, long> m; };");
    assert!(
        out.contains("Record") || out.contains("Map"),
        "missing map type: {out}"
    );
}

#[test]
fn ts_union() {
    let idl = r#"
        union U switch (long) {
            case 1: long x;
            default: octet y;
        };
    "#;
    let out = gen(Backend::TypeScript, idl);
    assert!(!out.is_empty(), "empty output for union");
}

// ---------------------------------------------------------------------------
// C backend
// ---------------------------------------------------------------------------

#[test]
fn c_struct_layout() {
    let out = gen(Backend::C, "struct Point { int32_t x; int32_t y; };");
    assert!(
        out.contains("typedef struct"),
        "missing typedef struct: {out}"
    );
    assert!(out.contains("int32_t"), "missing int32_t: {out}");
}

#[test]
fn c_enum() {
    let out = gen(Backend::C, "enum Color { RED, GREEN, BLUE };");
    assert!(out.contains("Color"), "missing enum: {out}");
}

#[test]
fn c_bounded_string() {
    let out = gen(Backend::C, "struct S { string<64> name; };");
    assert!(out.contains("char"), "missing char type: {out}");
}

#[test]
fn c_sequence() {
    let out = gen(Backend::C, "struct S { sequence<int32_t> items; };");
    assert!(!out.is_empty(), "empty output for sequence");
}

#[test]
fn c_union_tagged() {
    let idl = r#"
        union U switch (long) {
            case 1: long x;
            default: octet y;
        };
    "#;
    let out = gen(Backend::C, idl);
    assert!(
        out.contains("union") || out.contains("switch"),
        "missing union: {out}"
    );
}

#[test]
fn c_optional() {
    let out = gen(Backend::C, "struct S { @optional int32_t maybe; };");
    assert!(
        out.contains("uint8_t has_maybe;"),
        "missing has_maybe flag in struct: {out}"
    );
    // XTypes 1.3 7.4.3.5.3 rule 20: a presence byte, then the value.
    assert!(
        out.contains("hdds_x2c_w8(c, (uint8_t)(v->has_maybe ? 1 : 0))"),
        "missing presence flag encode: {out}"
    );
    assert!(
        out.contains("v->has_maybe = hx_t"),
        "missing presence flag decode: {out}"
    );
}

// ---------------------------------------------------------------------------
// C++ backend
// ---------------------------------------------------------------------------

#[test]
fn cpp_class_struct() {
    let out = gen(Backend::Cpp, "struct Point { int32_t x; int32_t y; };");
    assert!(
        out.contains("class Point") || out.contains("struct Point"),
        "missing class/struct: {out}"
    );
}

#[test]
fn cpp_enum_class() {
    let out = gen(Backend::Cpp, "enum Color { RED, GREEN, BLUE };");
    assert!(out.contains("Color"), "missing enum: {out}");
}

#[test]
fn cpp_optional() {
    let out = gen(Backend::Cpp, "struct S { @optional int32_t maybe; };");
    assert!(
        out.contains("optional") || out.contains("Optional"),
        "missing optional: {out}"
    );
}

#[test]
fn cpp_wstring() {
    let out = gen(Backend::Cpp, "struct S { wstring ws; };");
    assert!(
        out.contains("wstring") || out.contains("std::wstring"),
        "missing wstring: {out}"
    );
}

// ---------------------------------------------------------------------------
// Python backend
// ---------------------------------------------------------------------------

#[test]
fn python_dataclass() {
    let out = gen(Backend::Python, "struct Point { int32_t x; int32_t y; };");
    assert!(
        out.contains("dataclass") || out.contains("class Point"),
        "missing dataclass: {out}"
    );
}

#[test]
fn python_enum() {
    let out = gen(Backend::Python, "enum Color { RED, GREEN, BLUE };");
    assert!(
        out.contains("IntEnum") || out.contains("Enum") || out.contains("Color"),
        "missing enum: {out}"
    );
}

#[test]
fn python_optional() {
    let out = gen(Backend::Python, "struct S { @optional int32_t maybe; };");
    assert!(
        out.contains("Optional") || out.contains("None"),
        "missing optional: {out}"
    );
}

#[test]
fn python_sequence_to_list() {
    let out = gen(Backend::Python, "struct S { sequence<int32_t> items; };");
    assert!(
        out.contains("List") || out.contains("list"),
        "missing list: {out}"
    );
}

#[test]
fn python_encode_decode_present() {
    let out = gen(Backend::Python, "struct S { int32_t x; };");
    assert!(
        out.contains("encode")
            || out.contains("serialize")
            || out.contains("to_bytes")
            || out.contains("pack"),
        "missing encode/serialize: {out}"
    );
}

// ---------------------------------------------------------------------------
// C Micro backend
// ---------------------------------------------------------------------------

#[test]
fn c_micro_basic_struct() {
    let out = gen(Backend::CMicro, "struct S { int32_t x; uint8_t y; };");
    assert!(out.contains("int32_t"), "missing int32_t: {out}");
    assert!(out.contains("uint8_t"), "missing uint8_t: {out}");
}

#[test]
fn c_micro_bounded_string() {
    let out = gen(Backend::CMicro, "struct S { string<32> name; };");
    assert!(out.contains("char"), "missing char: {out}");
}

#[test]
fn c_micro_sequence() {
    let out = gen(
        Backend::CMicro,
        "struct S { sequence<int32_t, 10> items; };",
    );
    assert!(!out.is_empty(), "empty output for bounded sequence");
}

#[test]
fn c_micro_enum() {
    let out = gen(Backend::CMicro, "enum Status { OK, ERROR };");
    assert!(out.contains("Status"), "missing enum: {out}");
}

// ---------------------------------------------------------------------------
// Keyword escaping
// ---------------------------------------------------------------------------

#[test]
fn rust_escapes_type_field() {
    let out = gen(Backend::Rust, "struct S { int32_t type; };");
    assert!(out.contains("r#type"), "keyword 'type' not escaped: {out}");
    assert!(
        !out.contains("pub type:"),
        "raw keyword as struct field: {out}"
    );
}

#[test]
fn rust_escapes_match_field() {
    let out = gen(Backend::Rust, "struct S { int32_t match; };");
    assert!(
        out.contains("r#match"),
        "keyword 'match' not escaped: {out}"
    );
}

#[test]
fn python_escapes_class_field() {
    let out = gen(Backend::Python, "struct S { int32_t class; };");
    assert!(out.contains("class_"), "keyword 'class' not escaped: {out}");
}

#[test]
fn cpp_escapes_class_field() {
    let out = gen(Backend::Cpp, "struct S { int32_t class; };");
    assert!(out.contains("class_"), "keyword 'class' not escaped: {out}");
}

#[test]
fn ts_escapes_type_field() {
    let out = gen(Backend::TypeScript, "struct S { int32_t type; };");
    assert!(out.contains("type_"), "keyword 'type' not escaped: {out}");
}

#[test]
fn c_escapes_register_field() {
    let out = gen(Backend::C, "struct S { int32_t register; };");
    assert!(
        out.contains("register_"),
        "keyword 'register' not escaped: {out}"
    );
}

// ---------------------------------------------------------------------------
// @key annotation codegen: the KeyHolder writes of RTPS 2.5 9.6.4.8
// Covers: typedef string, nested struct, enum, bool, char, Fixed
// ---------------------------------------------------------------------------

#[test]
fn rust_key_typedef_string_writes_a_string() {
    let idl = r#"
        typedef string my_string;
        @extensibility(APPENDABLE)
        struct HelloWorld {
            @key my_string message;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(
        out.contains("w.write_string(&self.message);"),
        "typedef string @key field must be written as a string: {out}"
    );
    assert!(
        out.contains("KeyMaxSize::Unbounded"),
        "an unbounded string key makes the KeyHash an MD5: {out}"
    );
}

#[test]
fn rust_key_nested_struct_writes_its_members() {
    let idl = r#"
        struct Inner { uint32 index; };
        @extensibility(APPENDABLE)
        struct HelloWorld {
            @key Inner message;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(
        out.contains("w.write_u32(self.message.index);"),
        "a nested struct without keys contributes all its members: {out}"
    );
}

#[test]
fn rust_key_enum_is_written_at_its_bit_bound() {
    let idl = r#"
        enum Color { RED, GREEN, BLUE };
        @extensibility(APPENDABLE)
        struct Pixel {
            @key Color color;
            uint32 value;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(
        out.contains("w.write_i32(self.color as i32);"),
        "an enum with the default bit_bound is an int32: {out}"
    );
    assert!(
        !out.contains("self.value)"),
        "non-key member written: {out}"
    );
}

#[test]
fn rust_key_bool_is_one_byte() {
    let idl = r#"
        @extensibility(APPENDABLE)
        struct Flags {
            @key boolean active;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(
        out.contains("w.write_bool(self.active);"),
        "bool @key field must be written as a boolean: {out}"
    );
    assert!(out.contains("KeyMaxSize::Bounded(1)"), "{out}");
}

#[test]
fn rust_key_char_is_one_byte() {
    let idl = r#"
        @extensibility(APPENDABLE)
        struct Token {
            @key char symbol;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(
        out.contains("w.write_u8(self.symbol as u8);"),
        "an IDL char is one byte: {out}"
    );
}

#[test]
fn rust_key_fixed_is_not_covered() {
    let idl = r#"
        @extensibility(APPENDABLE)
        struct Price {
            @key fixed<10,2> amount;
        };
    "#;
    let out = gen(Backend::Rust, idl);
    assert!(
        out.contains("No KeyHolder generated") && !out.contains("fn write_key_holder"),
        "a fixed key has no KeyHolder writer call; the type says so: {out}"
    );
}
