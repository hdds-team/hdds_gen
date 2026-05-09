// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Unit tests for the Rust code generator.

#![allow(clippy::expect_used)]

use super::*;
use crate::ast::{
    BitfieldDecl, Bitset, Definition, Enum, EnumVariant, Field, Struct, Typedef, Union, UnionCase,
    UnionLabel,
};
use crate::types::{Annotation, IdlType, PrimitiveType};
use std::error::Error;

type TestResult<T> = std::result::Result<T, Box<dyn Error>>;

#[test]
fn rust_bitset_generates_helpers() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut bs = Bitset::new("Reg");
    let f1 = BitfieldDecl::new(3, "mode");
    let mut f2 = BitfieldDecl::new(5, "value");
    f2.annotations.push(Annotation::Position(4));
    bs.add_field(f1);
    bs.add_field(f2);
    file.add_definition(Definition::Bitset(bs));

    let r#gen = RustGenerator::new();
    let code = r#gen.generate(&file)?;
    assert!(code.contains("pub struct Reg { pub bits: u64 }"));
    assert!(code.contains("impl Reg {"));
    assert!(code.contains("MODE_SHIFT"));
    assert!(code.contains("MODE_MASK"));
    assert!(code.contains("VALUE_SHIFT"));
    assert!(code.contains("VALUE_MASK"));
    assert!(code.contains("pub fn mode(&self) -> u64"));
    assert!(code.contains("pub fn set_mode(&mut self, value: u64)"));
    assert!(code.contains("pub fn with_mode(mut self, value: u64) -> Self"));
    assert!(code.contains("pub const fn zero() -> Self"));
    assert!(code.contains("pub const fn from_bits(bits: u64) -> Self"));
    assert!(code.contains("pub const fn bits(&self) -> u64"));
    Ok(())
}

#[test]
fn typedef_fixed_alias_rust() -> TestResult<()> {
    let mut file = IdlFile::new();
    let td = Typedef {
        name: "Currency".into(),
        base_type: IdlType::Primitive(PrimitiveType::Fixed {
            digits: 10,
            scale: 2,
        }),
        annotations: vec![],
    };
    file.add_definition(Definition::Typedef(td));

    let r#gen = RustGenerator::new();
    let code = r#gen.generate(&file)?;
    assert!(code.contains("pub struct Fixed<"));
    assert!(code.contains("pub type Currency = Fixed<10, 2>;"));
    assert!(code.contains("from_parts"));
    assert!(code.contains("impl<const D: u32, const S: u32> core::fmt::Display for Fixed"));
    Ok(())
}

#[test]
fn codegen_struct_inheritance_rust() -> TestResult<()> {
    let mut file = IdlFile::new();

    let mut base = Struct::new("Base");
    base.add_field(Field::new("id", IdlType::Primitive(PrimitiveType::Int32)));
    let mut derived = Struct::new("Derived");
    derived.base_struct = Some("Base".to_string());
    derived.add_field(Field::new(
        "name",
        IdlType::Primitive(PrimitiveType::String),
    ));

    file.add_definition(Definition::Struct(base));
    file.add_definition(Definition::Struct(derived));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;
    assert!(out.contains("pub struct Derived {"));
    assert!(out.contains("pub base: Base,"));
    assert!(out.contains("pub id: i32"));
    assert!(out.contains("pub name: String"));
    Ok(())
}

#[test]
fn union_with_default_generates_default_impl() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut union = Union::new("Choice", IdlType::Primitive(PrimitiveType::Int32));

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Default],
        field: Field::new("none", IdlType::Primitive(PrimitiveType::String)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("1".into())],
        field: Field::new("number", IdlType::Primitive(PrimitiveType::Int32)),
    });

    file.add_definition(Definition::Union(union));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(out.contains("impl Default for Choice"));
    assert!(out.contains("Self::None(String::new())"));
    Ok(())
}

#[test]
fn union_default_case_encodes_zero_discriminant() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut union = Union::new("Message", IdlType::Primitive(PrimitiveType::Int32));

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("1".into())],
        field: Field::new("text", IdlType::Primitive(PrimitiveType::String)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Default],
        field: Field::new("unknown", IdlType::Primitive(PrimitiveType::Int32)),
    });

    file.add_definition(Definition::Union(union));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // The default case discriminant must encode as 0, not as the case index (1)
    assert!(
        out.contains("0 as i32"),
        "default case should encode discriminant as 0, got:\n{out}"
    );
    Ok(())
}

#[test]
fn union_default_case_avoids_collision_with_case_zero() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut union = Union::new("Event", IdlType::Primitive(PrimitiveType::Int32));

    // case 0 is explicitly used
    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("0".into())],
        field: Field::new("zero_val", IdlType::Primitive(PrimitiveType::Int32)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("1".into())],
        field: Field::new("one_val", IdlType::Primitive(PrimitiveType::Int32)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Default],
        field: Field::new("other", IdlType::Primitive(PrimitiveType::Octet)),
    });

    file.add_definition(Definition::Union(union));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // Default must NOT use 0 (collision with case 0) or 1 (collision with case 1)
    // It should pick 2
    assert!(
        out.contains("2 as i32"),
        "default case should pick discriminant 2 to avoid collision with 0 and 1, got:\n{out}"
    );
    Ok(())
}

#[test]
fn union_default_case_bool_discriminant() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut union = Union::new("Toggle", IdlType::Primitive(PrimitiveType::Boolean));

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("TRUE".into())],
        field: Field::new("on_value", IdlType::Primitive(PrimitiveType::Int32)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Default],
        field: Field::new("off_value", IdlType::Primitive(PrimitiveType::Int32)),
    });

    file.add_definition(Definition::Union(union));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // Boolean discriminant default should encode as "false"
    assert!(
        out.contains("false"),
        "boolean default case should encode discriminant as false, got:\n{out}"
    );
    Ok(())
}

#[test]
fn external_field_generates_box_type() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Node");

    s.add_field(Field::new("id", IdlType::Primitive(PrimitiveType::Int32)));
    s.add_field(
        Field::new("child", IdlType::Named("Node".into())).with_annotation(Annotation::External),
    );
    s.add_field(
        Field::new("label", IdlType::Primitive(PrimitiveType::String))
            .with_annotation(Annotation::External)
            .with_annotation(Annotation::Optional),
    );

    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // @external field -> Box<T>
    assert!(
        out.contains("pub child: Box<Node>"),
        "external field should be Box<T>, got:\n{out}"
    );

    // @external + @optional -> Option<Box<T>>
    assert!(
        out.contains("pub label: Option<Box<String>>"),
        "external+optional field should be Option<Box<T>>, got:\n{out}"
    );

    // Decoder must wrap in Box::new()
    assert!(
        out.contains("Box::new(child"),
        "decoder should wrap external field in Box::new(), got:\n{out}"
    );

    // Builder should accept T and wrap in Box::new in build()
    assert!(
        out.contains("Box::new(self.child"),
        "builder should wrap external field in Box::new(), got:\n{out}"
    );

    Ok(())
}

#[test]
fn external_sequence_generates_box_vec() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Container");

    s.add_field(
        Field::new(
            "items",
            IdlType::Sequence {
                inner: std::boxed::Box::new(IdlType::Primitive(PrimitiveType::Int32)),
                bound: None,
            },
        )
        .with_annotation(Annotation::External),
    );

    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains("pub items: Box<Vec<i32>>"),
        "external sequence should be Box<Vec<T>>, got:\n{out}"
    );

    Ok(())
}

#[test]
fn union_default_case_enum_discriminant() -> TestResult<()> {
    let mut file = IdlFile::new();

    // Define enum Mode { X, Y }
    let mut mode_enum = Enum::new("Mode");
    mode_enum.add_variant(EnumVariant {
        name: "X".into(),
        value: None,
    });
    mode_enum.add_variant(EnumVariant {
        name: "Y".into(),
        value: None,
    });
    file.add_definition(Definition::Enum(mode_enum));

    // Union switch(Mode) with case X, case Y, default
    let mut union = Union::new("U2", IdlType::Named("Mode".into()));

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("X".into())],
        field: Field::new("x_val", IdlType::Primitive(PrimitiveType::Int32)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Value("Y".into())],
        field: Field::new("y_val", IdlType::Primitive(PrimitiveType::Int32)),
    });

    union.add_case(UnionCase {
        labels: vec![UnionLabel::Default],
        field: Field::new("other", IdlType::Primitive(PrimitiveType::Octet)),
    });

    file.add_definition(Definition::Union(union));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // X=0, Y=1, so default must pick 2 to avoid collision
    assert!(
        out.contains("2 as i32"),
        "enum default case should pick discriminant 2 to avoid collision with X(0) and Y(1), got:\n{out}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// XCDR alignment tables
// ---------------------------------------------------------------------------
//
// Spec references:
// - OMG DDS-XTypes v1.3 (formal/2020-02-04) Section 7.4.1.1.1 Table 31
//   for XCDR v1 (doc page 122).
// - OMG DDS-XTypes v1.3 Section 7.4.2 + 7.4.3.2.2 Table 37 for XCDR v2
//   (doc pages 129 and 132).

#[test]
fn xcdr1_alignment_primitives_match_spec_table_31() {
    let one_byte = [
        PrimitiveType::Octet,
        PrimitiveType::UInt8,
        PrimitiveType::Int8,
        PrimitiveType::Boolean,
        PrimitiveType::Char,
    ];
    let two_byte = [
        PrimitiveType::Short,
        PrimitiveType::UnsignedShort,
        PrimitiveType::Int16,
        PrimitiveType::UInt16,
    ];
    let four_byte = [
        PrimitiveType::Long,
        PrimitiveType::UnsignedLong,
        PrimitiveType::Int32,
        PrimitiveType::UInt32,
        PrimitiveType::Float,
        PrimitiveType::WChar,
        PrimitiveType::String,
        PrimitiveType::WString,
    ];
    let eight_byte = [
        PrimitiveType::LongLong,
        PrimitiveType::UnsignedLongLong,
        PrimitiveType::Int64,
        PrimitiveType::UInt64,
        PrimitiveType::Double,
        PrimitiveType::LongDouble,
    ];

    for p in &one_byte {
        assert_eq!(
            RustGenerator::xcdr1_alignment(&IdlType::Primitive(*p)),
            1,
            "XCDR1: {p:?} must align to 1"
        );
    }
    for p in &two_byte {
        assert_eq!(
            RustGenerator::xcdr1_alignment(&IdlType::Primitive(*p)),
            2,
            "XCDR1: {p:?} must align to 2"
        );
    }
    for p in &four_byte {
        assert_eq!(
            RustGenerator::xcdr1_alignment(&IdlType::Primitive(*p)),
            4,
            "XCDR1: {p:?} must align to 4"
        );
    }
    for p in &eight_byte {
        assert_eq!(
            RustGenerator::xcdr1_alignment(&IdlType::Primitive(*p)),
            8,
            "XCDR1: {p:?} must align to 8 per Table 31"
        );
    }
}

#[test]
fn xcdr2_alignment_caps_8_byte_primitives_at_4() {
    // Per Section 7.4.2: INT64, UINT64, FLOAT64, FLOAT128 align on 4 in XCDR v2.
    let capped_at_4 = [
        PrimitiveType::LongLong,
        PrimitiveType::UnsignedLongLong,
        PrimitiveType::Int64,
        PrimitiveType::UInt64,
        PrimitiveType::Double,
        PrimitiveType::LongDouble,
    ];
    for p in &capped_at_4 {
        assert_eq!(
            RustGenerator::xcdr2_alignment(&IdlType::Primitive(*p)),
            4,
            "XCDR2: {p:?} must cap at 4 per Section 7.4.2 (not 8 as in XCDR1)"
        );
    }
}

#[test]
fn xcdr2_alignment_matches_xcdr1_for_types_not_larger_than_4_bytes() {
    // For primitives that align to <= 4 in XCDR1, XCDR2 must return the
    // same value (the MAXALIGN(VERSION2)=4 cap has no effect at or below 4).
    let types = [
        IdlType::Primitive(PrimitiveType::Octet),
        IdlType::Primitive(PrimitiveType::UInt8),
        IdlType::Primitive(PrimitiveType::Int8),
        IdlType::Primitive(PrimitiveType::Boolean),
        IdlType::Primitive(PrimitiveType::Char),
        IdlType::Primitive(PrimitiveType::Short),
        IdlType::Primitive(PrimitiveType::UnsignedShort),
        IdlType::Primitive(PrimitiveType::Int16),
        IdlType::Primitive(PrimitiveType::UInt16),
        IdlType::Primitive(PrimitiveType::Long),
        IdlType::Primitive(PrimitiveType::UnsignedLong),
        IdlType::Primitive(PrimitiveType::Int32),
        IdlType::Primitive(PrimitiveType::UInt32),
        IdlType::Primitive(PrimitiveType::Float),
        IdlType::Primitive(PrimitiveType::WChar),
        IdlType::Primitive(PrimitiveType::String),
        IdlType::Primitive(PrimitiveType::WString),
    ];
    for t in &types {
        assert_eq!(
            RustGenerator::xcdr1_alignment(t),
            RustGenerator::xcdr2_alignment(t),
            "XCDR1 and XCDR2 must match for {t:?} (alignment <= 4)"
        );
    }
}

#[test]
fn xcdr_alignment_non_primitive_types_unchanged_between_versions() {
    // Sequences, maps, and named references align via their u32 length prefix
    // in both versions.
    let seq = IdlType::Sequence {
        inner: Box::new(IdlType::Primitive(PrimitiveType::Double)),
        bound: None,
    };
    let map = IdlType::Map {
        key: Box::new(IdlType::Primitive(PrimitiveType::Int32)),
        value: Box::new(IdlType::Primitive(PrimitiveType::Double)),
        bound: None,
    };
    let named = IdlType::Named("MyStruct".to_string());
    for t in [&seq, &map, &named] {
        assert_eq!(RustGenerator::xcdr1_alignment(t), 4);
        assert_eq!(RustGenerator::xcdr2_alignment(t), 4);
    }
}

#[test]
fn xcdr_alignment_array_inherits_from_inner_type() {
    // Fixed arrays inherit the inner element's alignment, so a double[10]
    // aligns to 8 in XCDR1 and to 4 in XCDR2.
    let array_of_double = IdlType::Array {
        inner: Box::new(IdlType::Primitive(PrimitiveType::Double)),
        size: 10,
    };
    assert_eq!(RustGenerator::xcdr1_alignment(&array_of_double), 8);
    assert_eq!(RustGenerator::xcdr2_alignment(&array_of_double), 4);

    let array_of_u32 = IdlType::Array {
        inner: Box::new(IdlType::Primitive(PrimitiveType::UInt32)),
        size: 4,
    };
    assert_eq!(RustGenerator::xcdr1_alignment(&array_of_u32), 4);
    assert_eq!(RustGenerator::xcdr2_alignment(&array_of_u32), 4);
}

// ---------------------------------------------------------------------------
// Container routing proof
// ---------------------------------------------------------------------------
//
// When `Outer.encode_xcdr1_le` serializes a `sequence<Inner>` field, the
// per-element loop must invoke `elem.encode_xcdr1_le(...)`, not the legacy
// `elem.encode_cdr2_le(...)` which would route through the sub-type's
// primary-version trait impl and drop the caller's XCDR version choice.

fn make_outer_with_inner_sequence() -> IdlFile {
    let mut file = IdlFile::new();
    let mut inner = Struct::new("Inner");
    inner.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    inner.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(inner));

    let mut outer = Struct::new("Outer");
    outer.add_field(Field::new(
        "items",
        IdlType::Sequence {
            inner: Box::new(IdlType::Named("Inner".into())),
            bound: None,
        },
    ));
    file.add_definition(Definition::Struct(outer));
    file
}

#[test]
fn container_outer_xcdr1_body_invokes_sub_xcdr1_not_cdr2() -> TestResult<()> {
    let file = make_outer_with_inner_sequence();
    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // Sanity: both versions of the inner encoder must exist.
    assert!(
        out.contains("pub fn encode_xcdr1_le"),
        "Inner should emit encode_xcdr1_le"
    );
    assert!(
        out.contains("pub fn encode_xcdr2_le"),
        "Inner should emit encode_xcdr2_le"
    );

    // Slice the two encoder bodies for the Outer type.
    let xcdr1_start = out
        .find("impl Outer {\n    pub fn encode_xcdr1_le")
        .expect("Outer::encode_xcdr1_le block present");
    let xcdr1_rest = &out[xcdr1_start..];
    let xcdr1_end = xcdr1_rest
        .find("\n}\n")
        .expect("closing brace of Outer::encode_xcdr1_le found");
    let xcdr1_body = &xcdr1_rest[..xcdr1_end];

    let xcdr2_start = out
        .find("impl Outer {\n    pub fn encode_xcdr2_le")
        .expect("Outer::encode_xcdr2_le block present");
    let xcdr2_rest = &out[xcdr2_start..];
    let xcdr2_end = xcdr2_rest
        .find("\n}\n")
        .expect("closing brace of Outer::encode_xcdr2_le found");
    let xcdr2_body = &xcdr2_rest[..xcdr2_end];

    // The XCDR1 body must call the XCDR1 sub-encoder, never the legacy one.
    assert!(
        xcdr1_body.contains("elem.encode_xcdr1_le("),
        "Outer::encode_xcdr1_le should invoke elem.encode_xcdr1_le(). Body:\n{xcdr1_body}"
    );
    assert!(
        !xcdr1_body.contains("encode_cdr2_le"),
        "Outer::encode_xcdr1_le must not call the legacy encode_cdr2_le. Body:\n{xcdr1_body}"
    );

    // Same contract for the XCDR2 body.
    assert!(
        xcdr2_body.contains("elem.encode_xcdr2_le("),
        "Outer::encode_xcdr2_le should invoke elem.encode_xcdr2_le(). Body:\n{xcdr2_body}"
    );
    assert!(
        !xcdr2_body.contains("encode_cdr2_le"),
        "Outer::encode_xcdr2_le must not call the legacy encode_cdr2_le. Body:\n{xcdr2_body}"
    );
    Ok(())
}

#[test]
fn container_outer_xcdr1_decode_invokes_sub_xcdr1_not_cdr2() -> TestResult<()> {
    let file = make_outer_with_inner_sequence();
    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let xcdr1_start = out
        .find("impl Outer {\n    pub fn decode_xcdr1_le")
        .expect("Outer::decode_xcdr1_le block present");
    let xcdr1_rest = &out[xcdr1_start..];
    let xcdr1_end = xcdr1_rest
        .find("\n}\n")
        .expect("closing brace of Outer::decode_xcdr1_le found");
    let xcdr1_body = &xcdr1_rest[..xcdr1_end];

    let xcdr2_start = out
        .find("impl Outer {\n    pub fn decode_xcdr2_le")
        .expect("Outer::decode_xcdr2_le block present");
    let xcdr2_rest = &out[xcdr2_start..];
    let xcdr2_end = xcdr2_rest
        .find("\n}\n")
        .expect("closing brace of Outer::decode_xcdr2_le found");
    let xcdr2_body = &xcdr2_rest[..xcdr2_end];

    assert!(
        xcdr1_body.contains("Inner>::decode_xcdr1_le"),
        "Outer::decode_xcdr1_le should invoke <Inner>::decode_xcdr1_le. Body:\n{xcdr1_body}"
    );
    assert!(
        !xcdr1_body.contains("decode_cdr2_le"),
        "Outer::decode_xcdr1_le must not call legacy decode_cdr2_le. Body:\n{xcdr1_body}"
    );

    assert!(
        xcdr2_body.contains("Inner>::decode_xcdr2_le"),
        "Outer::decode_xcdr2_le should invoke <Inner>::decode_xcdr2_le. Body:\n{xcdr2_body}"
    );
    assert!(
        !xcdr2_body.contains("decode_cdr2_le"),
        "Outer::decode_xcdr2_le must not call legacy decode_cdr2_le. Body:\n{xcdr2_body}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Union routing proof
// ---------------------------------------------------------------------------
//
// Union cases containing a named sub-type must invoke the sub-type's
// matching XCDR version method, not the legacy trait method.

fn make_tagged_union_with_inner() -> IdlFile {
    let mut file = IdlFile::new();
    let mut inner = Struct::new("Inner");
    inner.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    inner.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(inner));

    let mut u = Union::new("TaggedInner", IdlType::Primitive(PrimitiveType::Int32));
    u.add_case(UnionCase {
        labels: vec![UnionLabel::Value("0".into())],
        field: Field::new("nested", IdlType::Named("Inner".into())),
    });
    u.add_case(UnionCase {
        labels: vec![UnionLabel::Value("1".into())],
        field: Field::new("scalar", IdlType::Primitive(PrimitiveType::Int32)),
    });
    file.add_definition(Definition::Union(u));
    file
}

#[test]
fn union_xcdr1_encode_case_named_invokes_sub_xcdr1() -> TestResult<()> {
    let file = make_tagged_union_with_inner();
    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    // Both union versions must exist as inherent methods.
    assert!(
        out.contains("impl TaggedInner {\n    pub fn encode_xcdr1_le"),
        "TaggedInner should emit encode_xcdr1_le"
    );
    assert!(
        out.contains("impl TaggedInner {\n    pub fn encode_xcdr2_le"),
        "TaggedInner should emit encode_xcdr2_le"
    );
    // And the legacy trait delegator.
    assert!(
        out.contains("impl Cdr2Encode for TaggedInner"),
        "TaggedInner should have a Cdr2Encode trait delegator"
    );

    let xcdr1_start = out
        .find("impl TaggedInner {\n    pub fn encode_xcdr1_le")
        .unwrap();
    let xcdr1_rest = &out[xcdr1_start..];
    let xcdr1_end = xcdr1_rest.find("\n}\n").unwrap();
    let xcdr1_body = &xcdr1_rest[..xcdr1_end];

    let xcdr2_start = out
        .find("impl TaggedInner {\n    pub fn encode_xcdr2_le")
        .unwrap();
    let xcdr2_rest = &out[xcdr2_start..];
    let xcdr2_end = xcdr2_rest.find("\n}\n").unwrap();
    let xcdr2_body = &xcdr2_rest[..xcdr2_end];

    // Each version's case-encoding of the Inner variant must call the
    // matching version on the sub-type.
    assert!(
        xcdr1_body.contains("v.encode_xcdr1_le("),
        "TaggedInner::encode_xcdr1_le should invoke v.encode_xcdr1_le(). Body:\n{xcdr1_body}"
    );
    assert!(
        !xcdr1_body.contains("encode_cdr2_le"),
        "TaggedInner::encode_xcdr1_le must not call legacy encode_cdr2_le. Body:\n{xcdr1_body}"
    );
    assert!(
        xcdr2_body.contains("v.encode_xcdr2_le("),
        "TaggedInner::encode_xcdr2_le should invoke v.encode_xcdr2_le(). Body:\n{xcdr2_body}"
    );
    assert!(
        !xcdr2_body.contains("encode_cdr2_le"),
        "TaggedInner::encode_xcdr2_le must not call legacy encode_cdr2_le. Body:\n{xcdr2_body}"
    );
    Ok(())
}

#[test]
fn union_xcdr1_decode_case_named_invokes_sub_xcdr1() -> TestResult<()> {
    let file = make_tagged_union_with_inner();
    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let xcdr1_start = out
        .find("impl TaggedInner {\n    pub fn decode_xcdr1_le")
        .unwrap();
    let xcdr1_rest = &out[xcdr1_start..];
    let xcdr1_end = xcdr1_rest.find("\n}\n").unwrap();
    let xcdr1_body = &xcdr1_rest[..xcdr1_end];

    let xcdr2_start = out
        .find("impl TaggedInner {\n    pub fn decode_xcdr2_le")
        .unwrap();
    let xcdr2_rest = &out[xcdr2_start..];
    let xcdr2_end = xcdr2_rest.find("\n}\n").unwrap();
    let xcdr2_body = &xcdr2_rest[..xcdr2_end];

    assert!(
        xcdr1_body.contains("Inner::decode_xcdr1_le"),
        "TaggedInner::decode_xcdr1_le should invoke Inner::decode_xcdr1_le. Body:\n{xcdr1_body}"
    );
    assert!(
        !xcdr1_body.contains("decode_cdr2_le"),
        "TaggedInner::decode_xcdr1_le must not call legacy decode_cdr2_le. Body:\n{xcdr1_body}"
    );
    assert!(
        xcdr2_body.contains("Inner::decode_xcdr2_le"),
        "TaggedInner::decode_xcdr2_le should invoke Inner::decode_xcdr2_le. Body:\n{xcdr2_body}"
    );
    assert!(
        !xcdr2_body.contains("decode_cdr2_le"),
        "TaggedInner::decode_xcdr2_le must not call legacy decode_cdr2_le. Body:\n{xcdr2_body}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Delegator routing proof
// ---------------------------------------------------------------------------
//
// The trait delegator emitted by `helpers::emit_cdr_trait_delegator` routes
// `Cdr2Encode::encode_cdr2_le` / `Cdr2Decode::decode_cdr2_le` to the type's
// primary inherent method, picked by `primary_version(repr)` where `repr`
// is read from `@data_representation`. Locks that routing end-to-end from
// the AST annotation to the generated delegator body.

fn slice_between<'a>(src: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = src.find(start)? + start.len();
    let len = src[s..].find(end)?;
    Some(&src[s..s + len])
}

#[test]
fn struct_without_annotation_delegator_targets_xcdr2() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let enc_body = slice_between(
        &out,
        "impl Cdr2Encode for Probe {\n    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {\n",
        "\n    }\n",
    )
    .expect("Cdr2Encode for Probe delegator body not found");
    assert!(
        enc_body.contains("self.encode_xcdr2_le(dst)"),
        "Probe without @data_representation must delegate encode_cdr2_le -> encode_xcdr2_le. \
         Got body:\n{enc_body}"
    );

    let dec_body = slice_between(
        &out,
        "impl Cdr2Decode for Probe {\n    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {\n",
        "\n    }\n",
    )
    .expect("Cdr2Decode for Probe delegator body not found");
    assert!(
        dec_body.contains("Self::decode_xcdr2_le(src)"),
        "Probe without @data_representation must delegate decode_cdr2_le -> decode_xcdr2_le. \
         Got body:\n{dec_body}"
    );
    Ok(())
}

#[test]
fn struct_with_xcdr1_annotation_delegator_targets_xcdr1() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("ProbeV1");
    s.add_annotation(Annotation::DataRepresentation("XCDR1".into()));
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let enc_body = slice_between(
        &out,
        "impl Cdr2Encode for ProbeV1 {\n    fn encode_cdr2_le(&self, dst: &mut [u8]) -> Result<usize, CdrError> {\n",
        "\n    }\n",
    )
    .expect("Cdr2Encode for ProbeV1 delegator body not found");
    assert!(
        enc_body.contains("self.encode_xcdr1_le(dst)"),
        "ProbeV1 with @data_representation(XCDR1) must delegate encode_cdr2_le -> encode_xcdr1_le. \
         Got body:\n{enc_body}"
    );

    let dec_body = slice_between(
        &out,
        "impl Cdr2Decode for ProbeV1 {\n    fn decode_cdr2_le(src: &[u8]) -> Result<(Self, usize), CdrError> {\n",
        "\n    }\n",
    )
    .expect("Cdr2Decode for ProbeV1 delegator body not found");
    assert!(
        dec_body.contains("Self::decode_xcdr1_le(src)"),
        "ProbeV1 with @data_representation(XCDR1) must delegate decode_cdr2_le -> decode_xcdr1_le. \
         Got body:\n{dec_body}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// `Cdr2Encode::encode_cdr2_le_at` REQUIRED method (DDS-XTypes v1.3
// §7.4.3.4.1 Tab.15) emission proof
// ---------------------------------------------------------------------------
//
// The trait delegator must emit `encode_cdr2_le_at` on every generated
// `impl Cdr2Encode for T` block; the runtime trait makes the method
// REQUIRED (no default impl) so any missing emission produces an E0046
// error in downstream crates compiling the generated code.

#[test]
fn struct_delegator_emits_encode_cdr2_le_at() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains(
            "fn encode_cdr2_le_at(\n        &self,\n        dst: &mut [u8],\n        offset: &mut usize,\n    ) -> Result<(), CdrError> {"
        ),
        "Probe delegator must emit encode_cdr2_le_at signature.\nGot:\n{out}"
    );
    let body = slice_between(
        &out,
        "fn encode_cdr2_le_at(\n        &self,\n        dst: &mut [u8],\n        offset: &mut usize,\n    ) -> Result<(), CdrError> {\n",
        "\n    }\n",
    )
    .expect("encode_cdr2_le_at body not found in Probe delegator");
    assert!(
        body.contains("self.encode_xcdr2_le(&mut dst[*offset..])"),
        "encode_cdr2_le_at must delegate to encode_xcdr2_le on Probe (no @data_representation). \
         Got body:\n{body}"
    );
    assert!(
        body.contains("*offset += len;"),
        "encode_cdr2_le_at must advance the global offset cursor. Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn xcdr1_struct_delegator_routes_encode_cdr2_le_at_to_xcdr1() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("ProbeV1");
    s.add_annotation(Annotation::DataRepresentation("XCDR1".into()));
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let body = slice_between(
        &out,
        "fn encode_cdr2_le_at(\n        &self,\n        dst: &mut [u8],\n        offset: &mut usize,\n    ) -> Result<(), CdrError> {\n",
        "\n    }\n",
    )
    .expect("encode_cdr2_le_at body not found in ProbeV1 delegator");
    assert!(
        body.contains("self.encode_xcdr1_le(&mut dst[*offset..])"),
        "ProbeV1 with @data_representation(XCDR1) must delegate encode_cdr2_le_at \
         -> encode_xcdr1_le. Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn dds_keyhash_cdr2_path_uses_encode_cdr2_le_at() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut nested = Struct::new("Nested");
    nested.add_field(Field::new("v", IdlType::Primitive(PrimitiveType::Long)));
    file.add_definition(Definition::Struct(nested));

    let mut outer = Struct::new("Outer");
    let mut key_field = Field::new("k", IdlType::Named("Nested".into()));
    key_field.annotations.push(Annotation::Key);
    outer.add_field(key_field);
    file.add_definition(Definition::Struct(outer));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains("let mut _kbuf = [0u8; 4096];"),
        "KeyHashKind::Cdr2 path must allocate _kbuf scratch buffer.\nGot:\n{out}"
    );
    assert!(
        out.contains("let mut _koffset: usize = 0;"),
        "KeyHashKind::Cdr2 path must initialize _koffset cursor for offset propagation.\nGot:\n{out}"
    );
    assert!(
        out.contains("self.k.encode_cdr2_le_at(&mut _kbuf, &mut _koffset).is_ok()"),
        "KeyHashKind::Cdr2 path must call encode_cdr2_le_at (DDS-XTypes v1.3 \
         §7.4.3.4.1 Tab.15) on the key field, not legacy encode_cdr2_le.\nGot:\n{out}"
    );
    assert!(
        out.contains("for &b in &_kbuf[.._koffset]"),
        "KeyHashKind::Cdr2 path must hash up to the cursor position (_koffset), \
         not a separately tracked length.\nGot:\n{out}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
//
// `emit_dds_trait_impl` emits `fn encode(&self, buf, version)` and
// `fn decode(buf, version)` on `impl ::hdds::api::DDS for T` that dispatch
// on `::hdds::CdrVersion` to the inherent methods `encode_xcdr{1,2}_le` /
// `decode_xcdr{1,2}_le`. Lock both match arms so the trait-level dispatch
// stays aligned with the dual-emission inherents.

#[test]
fn dds_trait_impl_encode_dispatches_on_cdr_version() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains(
            "fn encode(&self, buf: &mut [u8], version: ::hdds::CdrVersion) -> ::hdds::api::Result<usize>"
        ),
        "DDS trait impl must emit the CdrVersion-parametrized encode signature. Got:\n{out}"
    );
    assert!(
        out.contains("::hdds::CdrVersion::Xcdr1 => self.encode_xcdr1_le(buf).map_err(Into::into)"),
        "DDS::encode must dispatch Xcdr1 to encode_xcdr1_le. Got:\n{out}"
    );
    assert!(
        out.contains("::hdds::CdrVersion::Xcdr2 => self.encode_xcdr2_le(buf).map_err(Into::into)"),
        "DDS::encode must dispatch Xcdr2 to encode_xcdr2_le. Got:\n{out}"
    );
    Ok(())
}

#[test]
fn dds_trait_impl_decode_dispatches_on_cdr_version() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains(
            "fn decode(buf: &[u8], version: ::hdds::CdrVersion) -> ::hdds::api::Result<Self>"
        ),
        "DDS trait impl must emit the CdrVersion-parametrized decode signature. Got:\n{out}"
    );
    assert!(
        out.contains(
            "::hdds::CdrVersion::Xcdr1 => Self::decode_xcdr1_le(buf).map(|(val, _)| val).map_err(Into::into)"
        ),
        "DDS::decode must dispatch Xcdr1 to decode_xcdr1_le. Got:\n{out}"
    );
    assert!(
        out.contains(
            "::hdds::CdrVersion::Xcdr2 => Self::decode_xcdr2_le(buf).map(|(val, _)| val).map_err(Into::into)"
        ),
        "DDS::decode must dispatch Xcdr2 to decode_xcdr2_le. Got:\n{out}"
    );
    Ok(())
}

#[test]
fn enum_emits_dual_inherent_methods_and_trait_delegator() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut e = Enum::new("Color");
    e.add_variant(EnumVariant::new("Red", None));
    e.add_variant(EnumVariant::new("Green", None));
    e.add_variant(EnumVariant::new("Blue", None));
    file.add_definition(Definition::Enum(e));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains("impl Color {\n    pub fn encode_xcdr1_le"),
        "Color should emit encode_xcdr1_le inherent method"
    );
    assert!(
        out.contains("impl Color {\n    pub fn encode_xcdr2_le"),
        "Color should emit encode_xcdr2_le inherent method"
    );
    assert!(
        out.contains("impl Color {\n    pub fn decode_xcdr1_le")
            || out.contains("    pub fn decode_xcdr1_le"),
        "Color should emit decode_xcdr1_le inherent method"
    );
    assert!(
        out.contains("impl Cdr2Encode for Color"),
        "Color should emit Cdr2Encode trait delegator"
    );
    assert!(
        out.contains("impl Cdr2Decode for Color"),
        "Color should emit Cdr2Decode trait delegator"
    );
    Ok(())
}
