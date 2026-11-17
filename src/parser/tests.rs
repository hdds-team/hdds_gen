// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Unit tests for the IDL parser.

#![allow(clippy::pedantic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use super::Parser;
use crate::ast::*;
use crate::types::{Annotation, IdlType, PrimitiveType};

fn as_struct(def: &Definition) -> Option<&Struct> {
    if let Definition::Struct(s) = def {
        Some(s)
    } else {
        None
    }
}

fn as_typedef(def: &Definition) -> Option<&Typedef> {
    if let Definition::Typedef(t) = def {
        Some(t)
    } else {
        None
    }
}

fn as_bitset(def: &Definition) -> Option<&Bitset> {
    if let Definition::Bitset(b) = def {
        Some(b)
    } else {
        None
    }
}

fn as_bitmask(def: &Definition) -> Option<&Bitmask> {
    if let Definition::Bitmask(m) = def {
        Some(m)
    } else {
        None
    }
}

fn as_module(def: &Definition) -> Option<&Module> {
    if let Definition::Module(m) = def {
        Some(m)
    } else {
        None
    }
}

fn as_const(def: &Definition) -> Option<&Const> {
    if let Definition::Const(c) = def {
        Some(c)
    } else {
        None
    }
}

fn as_enum(def: &Definition) -> Option<&Enum> {
    if let Definition::Enum(e) = def {
        Some(e)
    } else {
        None
    }
}

fn as_map(idl: &IdlType) -> Option<(&IdlType, &IdlType, Option<u32>)> {
    if let IdlType::Map { key, value, bound } = idl {
        Some((key.as_ref(), value.as_ref(), *bound))
    } else {
        None
    }
}

#[test]
fn test_parse_simple_struct() {
    let input = r#"
        struct Point {
            int32_t x;
            int32_t y;
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let result = parser.parse();
    assert!(result.is_ok());

    let file = result.expect("expected struct definition to parse without errors");
    assert_eq!(file.definitions.len(), 1);

    let s = as_struct(&file.definitions[0]).expect("Expected struct definition");
    assert_eq!(s.name, "Point");
    assert_eq!(s.fields.len(), 2);
}

#[test]
fn test_parse_struct_with_key() {
    let input = r#"
        struct Data {
            @key int32_t id;
            string value;
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let result = parser.parse();
    assert!(result.is_ok());

    let file = result.expect("expected struct with key to parse");
    if let Definition::Struct(s) = &file.definitions[0] {
        assert!(s.fields[0].is_key());
        assert!(!s.fields[1].is_key());
    }
}

#[test]
fn test_parse_annotations_unit_and_range() {
    let input = r#"
        struct Data {
            @unit("meters") double distance;
            @range(min=0, max=100) int32_t percentage;
            @min(1) @max(10) int32_t count;
            @autoid(SEQUENTIAL) int32_t id;
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let result = parser.parse();
    assert!(result.is_ok(), "parser error: {:?}", result.err());
    let file = result.expect("expected struct with annotations to parse");

    let s = as_struct(&file.definitions[0]).expect("Expected struct");
    assert_eq!(s.fields.len(), 4);

    // @unit("meters")
    let ann0 = &s.fields[0].annotations;
    assert!(ann0
        .iter()
        .any(|a| matches!(a, Annotation::Unit(u) if u == "meters")));

    // @range(min=0, max=100)
    let ann1 = &s.fields[1].annotations;
    assert!(ann1.iter().any(
        |a| matches!(a, Annotation::Range{min, max} if min.value == "0" && max.value == "100")
    ));

    // @min(1) @max(10)
    let ann2 = &s.fields[2].annotations;
    assert!(ann2
        .iter()
        .any(|a| matches!(a, Annotation::Min(v) if v.value == "1")));
    assert!(ann2
        .iter()
        .any(|a| matches!(a, Annotation::Max(v) if v.value == "10")));

    // @autoid(SEQUENTIAL) captured on field annotations (we're permissive here)
    let ann3 = &s.fields[3].annotations;
    assert!(ann3.iter().any(|a| matches!(a, Annotation::AutoId(_))));
}

#[test]
fn parse_wchar_wstring_and_bounds() {
    let input = r#"
        struct Wide {
            wchar ch;
            wstring ws;
            wstring<256> ws_bounded;
        };
    "#;
    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("parse wide");
    let s = as_struct(&file.definitions[0]).expect("Expected struct");
    assert!(matches!(
        s.fields[0].field_type,
        IdlType::Primitive(PrimitiveType::WChar)
    ));
    assert!(matches!(
        s.fields[1].field_type,
        IdlType::Primitive(PrimitiveType::WString)
    ));
    assert_eq!(
        s.fields[2].field_type,
        IdlType::BoundedString {
            wide: true,
            bound: 256
        }
    );
}

/// `sequence<char, N>` and `string<N>` are different types: an element
/// count and no NUL against a length counting the NUL (DDS-XTypes 1.3
/// 7.4.3.5.3 rules 3 and 11).
#[test]
fn bounded_sequence_of_char_is_not_a_bounded_string() {
    let mut parser =
        Parser::try_new("struct S { string<4> a; sequence<char, 4> b; sequence<wchar> c; };")
            .expect("lex");
    let file = parser.parse().expect("parse");
    let s = as_struct(&file.definitions[0]).expect("Expected struct");
    assert_eq!(
        s.fields[0].field_type,
        IdlType::BoundedString {
            wide: false,
            bound: 4
        }
    );
    assert_eq!(
        s.fields[1].field_type,
        IdlType::Sequence {
            inner: Box::new(IdlType::Primitive(PrimitiveType::Char)),
            bound: Some(4)
        }
    );
    assert!(matches!(
        &s.fields[2].field_type,
        IdlType::Sequence { inner, bound: None } if **inner == IdlType::Primitive(PrimitiveType::WChar)
    ));
}

/// Union labels: TRUE / FALSE, characters, negative integers, scoped
/// enumerators.
#[test]
fn union_labels_of_every_literal_kind() {
    let src = "module m { enum K { A, B }; };
        union UB switch (boolean) { case TRUE: long t; case FALSE: short f; };
        union UC switch (char) { case 'a': long x; };
        union UN switch (long) { case -1: long n; case 2: short p; };
        union UK switch (m::K) { case m::A: long a; case ::m::B: short b; };";
    let mut parser = Parser::try_new(src).expect("lex");
    let file = parser.parse().expect("parse");
    let labels: Vec<String> = file
        .definitions
        .iter()
        .filter_map(|d| match d {
            Definition::Union(u) => Some(u),
            _ => None,
        })
        .flat_map(|u| u.cases.iter())
        .flat_map(|c| c.labels.iter())
        .map(|l| match l {
            crate::ast::UnionLabel::Value(v) => v.clone(),
            crate::ast::UnionLabel::Default => "default".to_string(),
        })
        .collect();
    assert_eq!(
        labels,
        vec!["TRUE", "FALSE", "'a'", "-1", "2", "m::A", "::m::B"]
    );
}

#[test]
fn parse_long_double_and_fixed() {
    let input = r#"
        struct S {
            long double ld;
        };
        typedef fixed<10,2> Currency;
    "#;
    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("parse ld/fixed");
    let s = as_struct(&file.definitions[0]).expect("expected struct");
    assert!(matches!(
        s.fields[0].field_type,
        IdlType::Primitive(PrimitiveType::LongDouble)
    ));

    let t = as_typedef(&file.definitions[1]).expect("expected typedef");
    assert!(matches!(
        t.base_type,
        IdlType::Primitive(PrimitiveType::Fixed { .. })
    ));
}

#[test]
fn parse_map_bounded_and_unbounded() {
    // Unbounded
    let input = r#"typedef map<string, int32_t> M;"#;
    let mut p = Parser::try_new(input).expect("lex");
    let file = p.parse().expect("parse map");
    let td = as_typedef(&file.definitions[0]).expect("expected typedef");
    let (key, value, bound) = as_map(&td.base_type).expect("expected map type");
    assert!(matches!(key, IdlType::Primitive(PrimitiveType::String)));
    assert!(matches!(value, IdlType::Primitive(PrimitiveType::Int32)));
    assert_eq!(bound, None);

    // Bounded
    let input = r#"typedef map<string, int32_t, 100> MB;"#;
    let mut p = Parser::try_new(input).expect("lex");
    let file = p.parse().expect("parse bounded map");
    let td = as_typedef(&file.definitions[0]).expect("expected typedef");
    let (key, value, bound) = as_map(&td.base_type).expect("expected map type");
    assert!(matches!(key, IdlType::Primitive(PrimitiveType::String)));
    assert!(matches!(value, IdlType::Primitive(PrimitiveType::Int32)));
    assert_eq!(bound, Some(100));
}

#[test]
fn test_parse_bitset_basic() {
    let input = r#"
        bitset MyBits {
            bitfield<3> field1;
            bitfield<5> field2;
            bitfield<8> field3, @position(20);
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("parse bitset");
    assert_eq!(file.definitions.len(), 1);
    let b = as_bitset(&file.definitions[0]).expect("Expected bitset");
    assert_eq!(b.name, "MyBits");
    assert_eq!(b.fields.len(), 3);
    assert_eq!(b.fields[0].width, 3);
    assert_eq!(b.fields[1].width, 5);
    assert_eq!(b.fields[2].width, 8);
    assert!(b.fields[2]
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::Position(p) if *p == 20)));
}

#[test]
fn test_parse_bitmask_basic() {
    let input = r#"
        bitmask MyFlags {
            FLAG_A,
            FLAG_B,
            @position(5) FLAG_C
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("parse bitmask");
    let m = as_bitmask(&file.definitions[0]).expect("Expected bitmask");
    assert_eq!(m.name, "MyFlags");
    assert_eq!(m.flags.len(), 3);
    assert!(m.flags[2]
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::Position(p) if *p == 5)));
}

#[test]
fn test_parse_module() {
    let input = r#"
        module Example {
            struct Point {
                int32_t x;
            };
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let result = parser.parse();
    assert!(result.is_ok());

    let file = result.expect("expected typedef map to parse");
    if let Definition::Module(m) = &file.definitions[0] {
        assert_eq!(m.name, "Example");
        assert_eq!(m.definitions.len(), 1);
    }
}

/// A reopened module stays where it is reopened (declaration order kept);
/// `scope::merge_reopened_modules` merges for the targets that need it.
#[test]
fn test_module_reopening_keeps_declaration_order() {
    let input = r#"
        module Foo {
            struct A { int32_t x; };
        };
        struct Between { Foo::A a; };
        module Foo {
            struct B { int32_t y; };
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("parse with reopening");
    let kinds: Vec<&str> = file
        .definitions
        .iter()
        .map(|d| match d {
            Definition::Module(m) => m.name.as_str(),
            Definition::Struct(s) => s.name.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(kinds, ["Foo", "Between", "Foo"]);

    let merged = crate::scope::merge_reopened_modules(&file);
    assert_eq!(merged.definitions.len(), 2, "Foo merged, Between kept");
    let m = as_module(&merged.definitions[0]).expect("expected merged module");
    let names: Vec<&str> = m
        .definitions
        .iter()
        .filter_map(|d| match d {
            Definition::Struct(s) => Some(s.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["A", "B"]);
}

#[test]
fn test_parse_const_expressions() {
    let input = r#"
        const int32_t A = 2 + 3 * 4; // 14
        const int32_t B = (2 + 3) * 4; // 20
        const int32_t C = 0x10 + 0b10 + 077; // 16 + 2 + 63 = 81
        const int32_t D = (C << 1) | 1; // 163
        const boolean E = !0 || (0 && 1);
    "#;
    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("const parse");
    assert_eq!(file.definitions.len(), 5);
    assert_eq!(
        as_const(&file.definitions[0])
            .expect("expected const")
            .value,
        "14"
    );
    assert_eq!(
        as_const(&file.definitions[1])
            .expect("expected const")
            .value,
        "20"
    );
    assert_eq!(
        as_const(&file.definitions[2])
            .expect("expected const")
            .value,
        "81"
    );
    assert_eq!(
        as_const(&file.definitions[3])
            .expect("expected const")
            .value,
        "163"
    );
    assert_eq!(
        as_const(&file.definitions[4])
            .expect("expected const")
            .value,
        "true"
    );
}

#[test]
fn test_parse_enum_with_const_expressions() {
    let input = r#"
        enum E {
            A = 1 + 2,
            B = (3 << 2) | 1
        };
    "#;
    let mut parser = Parser::try_new(input).expect("lex");
    let file = parser.parse().expect("parse enum with expressions");
    let e = as_enum(&file.definitions[0]).expect("expected enum");
    assert_eq!(e.variants.len(), 2);
    assert_eq!(e.variants[0].name, "A");
    assert_eq!(e.variants[0].value, Some(3));
    assert_eq!(e.variants[1].name, "B");
    assert_eq!(e.variants[1].value, Some((3 << 2) | 1));
}

#[test]
fn test_parse_default_annotation() {
    let input = r#"
        struct Config {
            @default(42) int32_t port;
            @default("localhost") string host;
            @default(true) boolean enabled;
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let result = parser.parse();
    assert!(result.is_ok(), "Parse failed: {:?}", result.err());

    let file = result.unwrap();
    let s = as_struct(&file.definitions[0]).expect("Expected struct");
    assert_eq!(s.name, "Config");
    assert_eq!(s.fields.len(), 3);

    // Check @default(42) on port
    let port = &s.fields[0];
    assert_eq!(port.name, "port");
    assert_eq!(port.get_default(), Some("42"));

    // Check @default("localhost") on host
    let host = &s.fields[1];
    assert_eq!(host.name, "host");
    assert_eq!(host.get_default(), Some("localhost"));

    // Check @default(true) on enabled
    let enabled = &s.fields[2];
    assert_eq!(enabled.name, "enabled");
    assert_eq!(enabled.get_default(), Some("true"));
}

#[cfg(not(feature = "interfaces"))]
#[test]
fn test_interface_keyword_without_feature_gives_clear_diagnostic() {
    let input = r#"
        interface Greeter {
            string hello(in string name);
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let err = parser.parse().expect_err("interface should fail to parse");
    assert!(
        err.message.contains("--features interfaces"),
        "unexpected message: {}",
        err.message
    );
    assert!(
        err.message.contains("no code is generated"),
        "unexpected message: {}",
        err.message
    );
}

#[cfg(not(feature = "interfaces"))]
#[test]
fn test_exception_keyword_without_feature_gives_clear_diagnostic() {
    let input = r#"
        exception MyError {
            int32_t code;
        };
    "#;

    let mut parser = Parser::try_new(input).expect("lex");
    let err = parser.parse().expect_err("exception should fail to parse");
    assert!(
        err.message.contains("--features interfaces"),
        "unexpected message: {}",
        err.message
    );
    assert!(
        err.message.contains("no code is generated"),
        "unexpected message: {}",
        err.message
    );
}
