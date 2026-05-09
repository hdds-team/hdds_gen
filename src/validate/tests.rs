// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Unit tests for the semantic validator.

#![allow(clippy::pedantic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

use super::{validate, Level};
use crate::ast::*;
use crate::parser::Parser;
use crate::types::Annotation;
fn parse_idl(input: &str) -> IdlFile {
    Parser::try_new(input)
        .expect("lex IDL fixture")
        .parse()
        .expect("parse IDL fixture")
}

#[test]
fn validate_numeric_min_max_ok() {
    let input = r#"struct S { @min(0) @max(10) int32_t x; };"#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.is_empty(), "unexpected diags: {:?}", diags);
}

#[test]
fn validate_fixed_accepts_numeric_annotations() {
    let input = r#"struct Money { @min(0) @max(1000) fixed<10,2> amount; };"#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        diags.is_empty(),
        "fixed field should accept numeric annotations: {:?}",
        diags
    );
}

#[test]
fn validate_numeric_range_inconsistent() {
    let input = r#"struct S { @range(min=10, max=5) double y; };"#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        diags.iter().any(|d| matches!(d.level, Level::Error)),
        "expected error"
    );
}

#[test]
fn validate_enum_duplicate_names() {
    let input = r#"
        enum Color { Red, Green, Red };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("duplicate enumerator 'Red'")));
}

#[test]
fn validate_unit_on_non_numeric_warns() {
    let input = r#"struct S { @unit("m") string name; };"#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| matches!(d.level, Level::Warning) && d.message.contains("@unit on non-numeric")));
}

#[test]
fn validate_forward_decl_resolution() {
    let input = r#"
        struct Bar;
        struct Foo { Bar b; };
        struct Bar { int32_t x; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .all(|d| !d.message.contains("Unresolved type reference")));
}

#[test]
fn validate_unresolved_named_type_warns() {
    let input = r#"struct A { UnknownType t; };"#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("Unresolved type reference: UnknownType")));
}

#[test]
fn validate_fqn_ambiguous_and_fqn_ok() {
    let input = r#"
        module A { struct X { int32_t a; }; };
        module B { struct X { int32_t b; }; };
        struct UseAmb { X field; };
        struct UseOk { A::X ax; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("Ambiguous type reference: X")));
    // ensure fully-qualified reference did not produce unresolved/ambiguous
    assert!(!diags
        .iter()
        .any(|d| d.message.contains("A::X") && d.message.contains("Unresolved")));
}

#[test]
fn validate_ambiguous_ranking_prefers_current_module() {
    let input = r#"
        module A { module Inner { struct X { int32_t a; }; struct Use { X f; }; }; };
        module B { struct X { int32_t b; }; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    // Expect an ambiguous error for unqualified X used inside A::Inner::Use
    // and the first candidate should be A::Inner::X (nearest by context)
    let msg = diags
        .iter()
        .find(|d| d.message.starts_with("Ambiguous type reference: X"))
        .map(|d| d.message.clone())
        .unwrap_or_default();
    assert!(msg.contains("candidates:"));
    // Check ordering hint
    // Extract substring after candidates:
    let tail = msg
        .split_once("candidates:")
        .map_or("", |(_, remainder)| remainder);
    assert!(
        !tail.is_empty(),
        "ambiguous message missing candidates list: {msg}"
    );
    assert!(tail.trim_start().starts_with("A::Inner::X"));
}

#[test]
fn validate_struct_duplicate_id() {
    let input = r#"struct S { @id(1) int32_t a; @id(1) int32_t b; };"#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| matches!(d.level, Level::Error) && d.message.contains("duplicate @id(1)")));
}

#[test]
fn validate_union_duplicate_id() {
    let input = r#"union U switch(int32_t) { case 1: int32_t a; case 2: int32_t b; };"#;
    let mut ast = parse_idl(input);
    // Attach @id(7) to both union case fields programmatically
    assert!(
        matches!(ast.definitions[0], Definition::Union(_)),
        "expected union"
    );
    if let Definition::Union(u) = &mut ast.definitions[0] {
        for c in &mut u.cases {
            c.field.annotations.push(Annotation::Id(7));
        }
    }
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| matches!(d.level, Level::Error) && d.message.contains("duplicate @id(7)")));
}

#[test]
fn autoid_sequential_struct_violation() {
    let input = r#"
        @autoid(SEQUENTIAL)
        struct S {
            @id(2) int32_t a;
            @id(1) int32_t b;
        };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("@autoid(SEQUENTIAL) violated")));
}

#[test]
fn union_multiple_defaults_violation() {
    let input = r#"
        union U switch(int32_t) {
            default: int32_t a;
            default: int32_t b;
        };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("multiple default labels")));
}

#[test]
fn union_field_annotations_parsed() {
    let input = r#"
        union U switch(int32_t) {
            case 1: @id(7) int32_t a;
        };
    "#;
    let ast = parse_idl(input);
    // ensure the annotation exists
    let u = match &ast.definitions[0] {
        Definition::Union(u) => u,
        _ => unreachable!(),
    };
    assert!(u.cases[0]
        .field
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::Id(7))));
}

#[test]
fn custom_annotation_decl_and_usage_ok() {
    let input = r#"
        @annotation MyAnn {
            int32_t value;
            string name default "unknown";
        };

        @MyAnn(value=10, name="hey")
        struct Foo { int32_t x; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        diags.iter().all(|d| !matches!(d.level, Level::Warning)),
        "unexpected diags: {:?}",
        diags
    );
}

#[test]
fn custom_annotation_missing_required_param() {
    let input = r#"
        @annotation MyAnn { int32_t value; string name; };
        @MyAnn(value=3)
        struct Foo { int32_t x; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(
        |d| matches!(d.level, Level::Error) && d.message.contains("missing required parameter")
    ));
}

#[test]
fn custom_annotation_positional_ok() {
    let input = r#"
        @annotation MyAnn { int32_t a; string b; };
        @MyAnn(1, "two")
        struct Foo { int32_t x; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    // allow only warnings (unknown annotations elsewhere)
    assert!(
        diags.iter().all(|d| !matches!(d.level, Level::Error)),
        "unexpected errors: {:?}",
        diags
    );
}

#[test]
fn custom_annotation_unknown_param_error() {
    let input = r#"
        @annotation MyAnn { int32_t value; };
        @MyAnn(value=1, extra=2)
        struct Foo { int32_t x; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| d.message.contains("unknown parameter 'extra'")));
}

#[test]
fn non_serialized_on_type_is_error() {
    let input = r#"
        @non_serialized
        struct S { int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error)
            && d.message
                .contains("@non_serialized is invalid at type level")
    }));
}

#[test]
fn data_representation_on_member_is_error() {
    let input = r#"
        struct S { @data_representation(XCDR2) int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error)
            && d.message.contains("@data_representation")
            && d.message.contains("invalid on a member")
    }));
}

#[test]
fn data_representation_xcdr1_on_mutable_struct_is_error() {
    // @mutable + @data_representation(XCDR1) asks for PL_CDR v1 wire, which is
    // out of scope of the XCDR1 WIP. The parser should reject it up front so
    // codegen does not silently fall back to XCDR2.
    let input = r#"
        @mutable
        @data_representation(XCDR1)
        struct M { int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        diags.iter().any(|d| {
            matches!(d.level, Level::Error)
                && d.message.contains("@data_representation(XCDR1)")
                && d.message.contains("@mutable")
        }),
        "expected rejection diag, got: {:?}",
        diags
    );
}

#[test]
fn data_representation_plain_cdr_on_mutable_struct_is_error() {
    // PLAIN_CDR is the XCDR v1 encoding -- same rejection as XCDR1.
    let input = r#"
        @mutable
        @data_representation(PLAIN_CDR)
        struct M { int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        diags.iter().any(|d| {
            matches!(d.level, Level::Error)
                && d.message.contains("@data_representation(PLAIN_CDR)")
                && d.message.contains("@mutable")
        }),
        "expected rejection diag, got: {:?}",
        diags
    );
}

#[test]
fn data_representation_xcdr2_on_mutable_struct_is_ok() {
    // @mutable + XCDR2 is the expected supported combination (PL_CDR2 wire).
    let input = r#"
        @mutable
        @data_representation(XCDR2)
        struct M { int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        !diags.iter().any(|d| matches!(d.level, Level::Error)
            && d.message.contains("@data_representation")
            && d.message.contains("@mutable")),
        "unexpected mutable-vs-XCDR2 diag: {:?}",
        diags
    );
}

#[test]
fn data_representation_xcdr1_on_mutable_union_is_error() {
    // The same rule applies to `@mutable union` -- unions share the PL_CDR2
    // rewire, so pinning them to XCDR1 is equally unsupported.
    let input = r#"
        @mutable
        @data_representation(XCDR1)
        union U switch(long) { case 0: int32_t a; default: int32_t b; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        diags.iter().any(|d| {
            matches!(d.level, Level::Error)
                && d.message.contains("@data_representation(XCDR1)")
                && d.message.contains("@mutable")
        }),
        "expected rejection diag, got: {:?}",
        diags
    );
}

#[cfg(feature = "interfaces")]
#[test]
fn interface_oneway_must_return_void_and_no_raises() {
    let input = r#"
        interface Net {
            oneway int32_t ping(in int32_t seq);
            oneway void notify(in int32_t code);
        };
    "#;
    let ast = parse_idl(input);
    let ds = validate(&ast);
    assert!(ds
        .iter()
        .any(|d| d.message.contains("oneway requires void return")));
}

#[cfg(feature = "interfaces")]
#[test]
fn interface_raises_must_reference_declared_exceptions() {
    let input = r#"
        interface Dev { void op() raises(NotFound); };
    "#;
    let ast = parse_idl(input);
    let ds = validate(&ast);
    assert!(ds
        .iter()
        .any(|d| d.message.contains("raises unknown exception 'NotFound'")));
}

#[cfg(feature = "interfaces")]
#[test]
fn interface_duplicate_operation_and_attribute_names() {
    let input = r#"
        interface A {
            attribute int32_t dup;
            void dup();
            void op();
            void op();
        };
    "#;
    let ast = parse_idl(input);
    let ds = validate(&ast);
    assert!(ds
        .iter()
        .any(|d| d.message.contains("duplicate operation 'op'")));
    assert!(ds
        .iter()
        .any(|d| d.message.contains("conflicts with attribute")));
}

#[cfg(feature = "interfaces")]
#[test]
fn interface_duplicate_parameter_names() {
    let input = r#"
        interface P {
            void add(in int32_t x, in int32_t x);
        };
    "#;
    let ast = parse_idl(input);
    let ds = validate(&ast);
    assert!(ds
        .iter()
        .any(|d| d.message.contains("duplicate parameter name 'x'")));
}

#[test]
fn external_on_struct_is_ok() {
    let input = r#"
        @external
        struct S { int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        !diags.iter().any(|d| d.message.contains("@external")),
        "@external should be valid on struct: {:?}",
        diags
    );
}

#[test]
fn external_on_struct_member_is_ok() {
    let input = r#"
        struct S { @external int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        !diags.iter().any(|d| d.message.contains("@external")),
        "@external should be valid on struct member: {:?}",
        diags
    );
}

#[test]
fn external_on_union_is_ok() {
    let input = r#"
        @external
        union U switch(int32_t) { case 1: int32_t a; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(
        !diags.iter().any(|d| d.message.contains("@external")),
        "@external should be valid on union: {:?}",
        diags
    );
}

#[test]
fn external_on_enum_is_error() {
    let input = r#"
        @external
        enum E { A, B };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error) && d.message.contains("@external is invalid on enums")
    }));
}

#[test]
fn external_on_bitset_is_error() {
    let input = r#"
        @external
        bitset B { bitfield<4> flags; };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error) && d.message.contains("@external is invalid on bitsets")
    }));
}

#[test]
fn external_on_typedef_is_error() {
    let input = r#"
        @external
        typedef int32_t MyInt;
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error) && d.message.contains("@external is invalid on typedefs")
    }));
}

#[test]
fn external_on_bitmask_is_error() {
    let input = r#"
        @external
        bitmask Flags { FLAG_A, FLAG_B };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error) && d.message.contains("@external is invalid on bitmasks")
    }));
}

#[test]
fn bitmask_duplicate_flag_is_error() {
    let input = r#"
        bitmask Flags { A, B, A };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags
        .iter()
        .any(|d| { matches!(d.level, Level::Error) && d.message.contains("duplicate flag 'A'") }));
}

#[test]
fn bitmask_empty_is_error() {
    let input = r#"
        bitmask Empty { };
    "#;
    let ast = parse_idl(input);
    let diags = validate(&ast);
    assert!(diags.iter().any(|d| {
        matches!(d.level, Level::Error) && d.message.contains("must declare at least one flag")
    }));
}
