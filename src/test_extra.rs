// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Supplemental unit tests covering parser, lexer, validator, and pretty-printer gaps.

#![allow(clippy::expect_used, clippy::panic, clippy::needless_raw_string_hashes)]

use crate::ast::Definition;
use crate::parser::Parser;
use crate::pretty::to_idl;
use crate::validate::validate;

fn parse(idl: &str) -> crate::ast::IdlFile {
    let mut p = Parser::try_new(idl).expect("lexer error");
    p.parse().expect("parse error")
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[test]
fn nested_modules() {
    let ast = parse("module A { module B { struct S { int32_t x; }; }; };");
    assert_eq!(ast.definitions.len(), 1);
    if let Definition::Module(a) = &ast.definitions[0] {
        assert_eq!(a.name, "A");
        if let Definition::Module(b) = &a.definitions[0] {
            assert_eq!(b.name, "B");
            assert!(matches!(&b.definitions[0], Definition::Struct(_)));
        } else {
            panic!("expected nested module B");
        }
    } else {
        panic!("expected module A");
    }
}

#[test]
fn empty_struct() {
    let ast = parse("struct Empty { };");
    if let Definition::Struct(s) = &ast.definitions[0] {
        assert_eq!(s.name, "Empty");
        assert!(s.fields.is_empty());
    } else {
        panic!("expected struct");
    }
}

#[test]
fn sequence_of_sequence() {
    let ast = parse("struct S { sequence<sequence<int32_t>> nested; };");
    assert_eq!(ast.definitions.len(), 1);
    if let Definition::Struct(s) = &ast.definitions[0] {
        let ty = &s.fields[0].field_type;
        assert!(matches!(ty, crate::types::IdlType::Sequence { .. }));
    } else {
        panic!("expected struct");
    }
}

#[test]
fn union_multi_labels() {
    let idl = r#"
        union U switch (long) {
            case 1:
            case 2:
                long x;
            default:
                octet y;
        };
    "#;
    let ast = parse(idl);
    if let Definition::Union(u) = &ast.definitions[0] {
        assert_eq!(u.name, "U");
        // At least 2 branches
        assert!(u.cases.len() >= 2);
    } else {
        panic!("expected union");
    }
}

#[test]
fn forward_declaration() {
    let ast = parse("struct Foo; struct Foo { int32_t x; };");
    // Forward decl + real struct
    assert!(ast.definitions.len() >= 2);
    assert!(matches!(&ast.definitions[0], Definition::ForwardDecl(_)));
}

#[test]
fn typedef_chain() {
    let idl = "typedef long MyLong; typedef MyLong MyLong2;";
    let ast = parse(idl);
    assert_eq!(ast.definitions.len(), 2);
    assert!(matches!(&ast.definitions[0], Definition::Typedef(_)));
    assert!(matches!(&ast.definitions[1], Definition::Typedef(_)));
}

#[test]
fn enum_with_values() {
    let idl = "enum Color { RED = 0, GREEN = 1, BLUE = 2 };";
    let ast = parse(idl);
    if let Definition::Enum(e) = &ast.definitions[0] {
        assert_eq!(e.variants.len(), 3);
        assert_eq!(e.variants[0].value, Some(0));
        assert_eq!(e.variants[2].value, Some(2));
    } else {
        panic!("expected enum");
    }
}

#[test]
fn const_definition() {
    let ast = parse("const long MAX_SIZE = 100;");
    if let Definition::Const(c) = &ast.definitions[0] {
        assert_eq!(c.name, "MAX_SIZE");
    } else {
        panic!("expected const");
    }
}

#[test]
fn bounded_string_field() {
    let ast = parse("struct S { string<255> name; };");
    if let Definition::Struct(s) = &ast.definitions[0] {
        assert_eq!(s.fields.len(), 1);
    } else {
        panic!("expected struct");
    }
}

#[test]
fn map_type() {
    let ast = parse("struct S { map<string, long> m; };");
    if let Definition::Struct(s) = &ast.definitions[0] {
        assert!(matches!(
            &s.fields[0].field_type,
            crate::types::IdlType::Map { .. }
        ));
    } else {
        panic!("expected struct");
    }
}

// ---------------------------------------------------------------------------
// Lexer (via try_new error propagation)
// ---------------------------------------------------------------------------

#[test]
fn try_new_propagates_lexer_error() {
    // Unterminated string literal should error
    let result = Parser::try_new("struct S { string x = \"unterminated; };");
    assert!(result.is_err());
}

#[test]
fn hex_literal() {
    let ast = parse("const long HEX = 0xFF;");
    if let Definition::Const(c) = &ast.definitions[0] {
        assert_eq!(c.name, "HEX");
    } else {
        panic!("expected const");
    }
}

#[test]
fn octal_literal() {
    let ast = parse("const long OCT = 077;");
    if let Definition::Const(c) = &ast.definitions[0] {
        assert_eq!(c.name, "OCT");
    } else {
        panic!("expected const");
    }
}

#[test]
fn line_comment_ignored() {
    let ast = parse("// comment\nstruct S { int32_t x; };");
    assert_eq!(ast.definitions.len(), 1);
}

#[test]
fn block_comment_ignored() {
    let ast = parse("/* block */ struct S { int32_t x; };");
    assert_eq!(ast.definitions.len(), 1);
}

// ---------------------------------------------------------------------------
// Validator
// ---------------------------------------------------------------------------

#[test]
fn validate_good_struct_no_diags() {
    let ast = parse("struct S { int32_t x; int32_t y; };");
    let diags = validate(&ast);
    assert!(diags.is_empty(), "unexpected diags: {diags:?}");
}

#[test]
fn validate_unknown_named_type() {
    let ast = parse("struct S { UnknownType x; };");
    let diags = validate(&ast);
    assert!(!diags.is_empty(), "expected a diagnostic for unknown type");
}

#[test]
fn validate_duplicate_field_names_no_crash() {
    // Validator may or may not flag duplicates; ensure no panic
    let ast = parse("struct S { int32_t x; int32_t x; };");
    let _diags = validate(&ast);
}

#[test]
fn validate_key_and_optional_no_crash() {
    // Ensure validator handles @key + @optional without panic
    let ast = parse("struct S { @key @optional int32_t x; };");
    let _diags = validate(&ast);
}

#[test]
fn validate_enum_empty_no_crash() {
    let ast = parse("enum E { };");
    let _diags = validate(&ast);
}

#[test]
fn validate_union_float_discriminant() {
    // float is not a valid union discriminant - validator should flag or at least not crash
    // (parsing may reject this too)
    let result = Parser::try_new("union U switch (float) { case 1: long x; };");
    if let Ok(mut p) = result {
        if let Ok(ast) = p.parse() {
            let _diags = validate(&ast);
        }
    }
}

// ---------------------------------------------------------------------------
// Pretty-printer roundtrip
// ---------------------------------------------------------------------------

#[test]
fn pretty_roundtrip_struct() {
    let idl = "struct Point { int32_t x; int32_t y; };";
    let ast = parse(idl);
    let pretty = to_idl(&ast);
    // Re-parse the pretty-printed output
    let ast2 = parse(&pretty);
    assert_eq!(ast.definitions.len(), ast2.definitions.len());
}

#[test]
fn pretty_roundtrip_module() {
    let idl = "module M { struct S { float f; }; };";
    let ast = parse(idl);
    let pretty = to_idl(&ast);
    let ast2 = parse(&pretty);
    assert_eq!(ast.definitions.len(), ast2.definitions.len());
}

#[test]
fn pretty_roundtrip_enum() {
    let idl = "enum Color { RED, GREEN, BLUE };";
    let ast = parse(idl);
    let pretty = to_idl(&ast);
    let ast2 = parse(&pretty);
    assert_eq!(ast.definitions.len(), ast2.definitions.len());
}

#[test]
fn pretty_preserves_annotations() {
    let idl = r#"@mutable struct S { @key int32_t id; };"#;
    let ast = parse(idl);
    let pretty = to_idl(&ast);
    assert!(pretty.contains("@mutable"), "missing @mutable in: {pretty}");
    assert!(pretty.contains("@key"), "missing @key in: {pretty}");
}

#[test]
fn pretty_indentation_nested_module() {
    let idl = "module A { module B { struct S { int32_t x; }; }; };";
    let ast = parse(idl);
    let pretty = to_idl(&ast);
    // Should have indented content inside modules
    assert!(pretty.contains("module A"), "missing module A");
    assert!(pretty.contains("module B"), "missing module B");
}

// ---------------------------------------------------------------------------
// Error recovery / edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_input_parses_ok() {
    let ast = parse("");
    assert!(ast.definitions.is_empty());
}

#[test]
fn multiple_structs() {
    let idl = "struct A { int32_t x; }; struct B { float y; };";
    let ast = parse(idl);
    assert_eq!(ast.definitions.len(), 2);
}

#[test]
fn array_field() {
    let ast = parse("struct S { int32_t data[10]; };");
    if let Definition::Struct(s) = &ast.definitions[0] {
        assert!(matches!(
            &s.fields[0].field_type,
            crate::types::IdlType::Array { size: 10, .. }
        ));
    } else {
        panic!("expected struct");
    }
}

#[test]
fn bitmask_definition() {
    let idl = "bitmask Flags { FLAG_A, FLAG_B, FLAG_C };";
    let ast = parse(idl);
    assert!(matches!(&ast.definitions[0], Definition::Bitmask(_)));
}

#[test]
fn annotation_decl() {
    let idl = "@annotation MyAnnotation { string value; };";
    let ast = parse(idl);
    assert!(matches!(&ast.definitions[0], Definition::AnnotationDecl(_)));
}
