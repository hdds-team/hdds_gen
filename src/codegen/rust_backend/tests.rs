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
    // XTypes 1.3 ss7.2.2.4.4: Derived's members are Base's followed by its own,
    // flattened — no synthetic `base` field (which no emitter ever populated).
    assert!(!out.contains("pub base: Base,"));
    assert!(out.contains("pub struct Derived {\n    pub id: i32,\n    pub name: String,\n}"));
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
    // Post-1.6.1a-codegen-encode: the inner call routes through the
    // offset-aware `_at` wrapper instead of the legacy sub-buffer pattern.
    assert!(
        xcdr1_body.contains("elem.encode_xcdr1_le_at(dst, &mut offset)"),
        "Outer::encode_xcdr1_le should invoke elem.encode_xcdr1_le_at(dst, &mut offset). \
         Body:\n{xcdr1_body}"
    );
    assert!(
        !xcdr1_body.contains("encode_cdr2_le"),
        "Outer::encode_xcdr1_le must not call the legacy encode_cdr2_le. Body:\n{xcdr1_body}"
    );

    // Same contract for the XCDR2 body.
    assert!(
        xcdr2_body.contains("elem.encode_xcdr2_le_at(dst, &mut offset)"),
        "Outer::encode_xcdr2_le should invoke elem.encode_xcdr2_le_at(dst, &mut offset). \
         Body:\n{xcdr2_body}"
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
    // matching version on the sub-type. Post-1.6.1a-codegen-encode: the
    // inner call routes through the offset-aware `_at` wrapper instead
    // of the legacy sub-buffer pattern.
    assert!(
        xcdr1_body.contains("v.encode_xcdr1_le_at(dst, &mut offset)"),
        "TaggedInner::encode_xcdr1_le should invoke v.encode_xcdr1_le_at(dst, &mut offset). \
         Body:\n{xcdr1_body}"
    );
    assert!(
        !xcdr1_body.contains("encode_cdr2_le"),
        "TaggedInner::encode_xcdr1_le must not call legacy encode_cdr2_le. Body:\n{xcdr1_body}"
    );
    assert!(
        xcdr2_body.contains("v.encode_xcdr2_le_at(dst, &mut offset)"),
        "TaggedInner::encode_xcdr2_le should invoke v.encode_xcdr2_le_at(dst, &mut offset). \
         Body:\n{xcdr2_body}"
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
// `Cdr2Decode::decode_cdr2_le_at` REQUIRED method (DDS-XTypes v1.3
// §7.4.3.4.1 Tab.15) emission proof — symmetric to the encode-side
// lock tests above. Added in 1.6.2a-codegen-rust.
// ---------------------------------------------------------------------------
//
// The trait delegator must emit `decode_cdr2_le_at` on every generated
// `impl Cdr2Decode for T` block; the runtime trait makes the method
// REQUIRED (no default impl) so any missing emission produces an E0046
// error in downstream crates compiling the generated code.

#[test]
fn struct_delegator_emits_decode_cdr2_le_at() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    s.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::Double)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains(
            "fn decode_cdr2_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {"
        ),
        "Probe delegator must emit decode_cdr2_le_at signature.\nGot:\n{out}"
    );
    let body = slice_between(
        &out,
        "fn decode_cdr2_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {\n",
        "\n    }\n",
    )
    .expect("decode_cdr2_le_at body not found in Probe delegator");
    assert!(
        body.contains("Self::decode_xcdr2_le(&src[*offset..])"),
        "decode_cdr2_le_at must delegate to decode_xcdr2_le on Probe (no @data_representation). \
         Got body:\n{body}"
    );
    assert!(
        body.contains("*offset += used;"),
        "decode_cdr2_le_at must advance the global offset cursor. Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn xcdr1_struct_delegator_routes_decode_cdr2_le_at_to_xcdr1() -> TestResult<()> {
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
        "fn decode_cdr2_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {\n",
        "\n    }\n",
    )
    .expect("decode_cdr2_le_at body not found in ProbeV1 delegator");
    assert!(
        body.contains("Self::decode_xcdr1_le(&src[*offset..])"),
        "ProbeV1 with @data_representation(XCDR1) must delegate decode_cdr2_le_at \
         -> decode_xcdr1_le. Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn struct_inherent_emits_decode_xcdr1_le_at_wrapper() -> TestResult<()> {
    // Symmetric with the xcdr2 inherent test below: an @data_representation(XCDR1)
    // struct must emit the inherent `pub fn decode_xcdr1_le_at` wrapper next
    // to the legacy `decode_xcdr1_le`, so an outer XCDR1 decoder can call
    // `Inner::decode_xcdr1_le_at(src, &mut offset)` directly.
    let mut file = IdlFile::new();
    let mut s = Struct::new("ProbeV1");
    s.add_annotation(Annotation::DataRepresentation("XCDR1".into()));
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains(
            "pub fn decode_xcdr1_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {"
        ),
        "ProbeV1 inherent impl must emit decode_xcdr1_le_at wrapper.\nGot:\n{out}"
    );
    let body = slice_between(
        &out,
        "pub fn decode_xcdr1_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {\n",
        "\n    }\n",
    )
    .expect("decode_xcdr1_le_at wrapper body not found");
    assert!(
        body.contains("Self::decode_xcdr1_le(&src[*offset..])"),
        "decode_xcdr1_le_at wrapper must delegate to decode_xcdr1_le on a sub-slice. \
         Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn struct_inherent_emits_decode_xcdr2_le_at_wrapper() -> TestResult<()> {
    // The inherent `impl Probe { ... }` block must include a
    // `decode_{suffix}_le_at` wrapper next to the legacy `decode_{suffix}_le`
    // so outer decoders can use the offset-aware call pattern
    // `Inner::decode_xcdr2_le_at(src, &mut offset)` directly.
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(
        out.contains(
            "pub fn decode_xcdr2_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {"
        ),
        "Probe inherent impl must emit decode_xcdr2_le_at wrapper.\nGot:\n{out}"
    );
    let body = slice_between(
        &out,
        "pub fn decode_xcdr2_le_at(\n        src: &[u8],\n        offset: &mut usize,\n    ) -> Result<Self, CdrError> {\n",
        "\n    }\n",
    )
    .expect("decode_xcdr2_le_at wrapper body not found");
    assert!(
        body.contains("Self::decode_xcdr2_le(&src[*offset..])"),
        "decode_xcdr2_le_at wrapper must delegate to decode_xcdr2_le on a sub-slice. \
         Got body:\n{body}"
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

// ---------------------------------------------------------------------------
// IDL inheritance flattening (XTypes 1.3 ss7.2.2.4.4)
// ---------------------------------------------------------------------------

/// Parse + generate Rust, surfacing the generator error instead of panicking.
fn try_generate_rust_for_idl(idl: &str) -> crate::error::Result<String> {
    let mut parser = crate::Parser::try_new(idl).expect("lexer error");
    let file = parser.parse().expect("parse error");
    RustGenerator::new().generate(&file)
}

fn generate_rust_for_idl(idl: &str) -> String {
    try_generate_rust_for_idl(idl).expect("codegen error")
}

/// Field names declared in `pub struct {name} { ... }`, in emitted order.
fn struct_def_field_names(src: &str, name: &str) -> Vec<String> {
    let head = format!("pub struct {name} {{\n");
    let start = src
        .find(&head)
        .unwrap_or_else(|| panic!("struct {name} not found in:\n{src}"))
        + head.len();
    // Stop at the closing brace line (indented when inside a `module`).
    let body = &src[start..];
    let end = body
        .lines()
        .position(|l| l.trim() == "}")
        .unwrap_or_else(|| panic!("unterminated struct {name} in:\n{src}"));
    body.lines()
        .take(end)
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|l| l.split(':').next())
        .map(str::to_string)
        .collect()
}

#[test]
fn inherited_members_are_flattened_per_xtypes_7_2_2_4_4() {
    let src = generate_rust_for_idl(
        "module P { struct Base { long a; }; struct Derived : Base { long b; }; };",
    );
    assert!(
        src.contains("pub a: i32"),
        "base member must be inlined into Derived:\n{src}"
    );
    assert!(
        !src.contains("pub base:"),
        "synthetic base field must not exist:\n{src}"
    );
    assert_eq!(
        struct_def_field_names(&src, "Derived"),
        vec!["a".to_string(), "b".to_string()],
        "Derived members are the base's followed by its own:\n{src}"
    );
}

#[test]
fn flattened_struct_literals_are_complete_for_derived() {
    // Regression: decode's `Ok((Self { .. }))` and `Builder::build()`'s struct
    // literal are driven off `s.fields`; the synthetic `base` field was never
    // in that list, so every derived type failed to compile with E0063.
    let src = generate_rust_for_idl("struct Base { long a; }; struct Derived : Base { long b; };");
    assert_eq!(
        src.matches("Ok((Self {\n            a,\n            b,\n        }, offset))")
            .count(),
        2,
        "both XCDR1 and XCDR2 decode literals must name every flattened member:\n{src}"
    );
    assert!(
        src.contains("pub struct DerivedBuilder {\n    a: Option<i32>,\n    b: Option<i32>,\n}"),
        "builder must carry a slot for each flattened member:\n{src}"
    );
    assert!(
        src.contains(
            "        Ok(Derived {\n            a: self.a.ok_or(\"a is required\")?,\n            b: self.b.ok_or(\"b is required\")?,\n        })"
        ),
        "build() literal must initialise every flattened member:\n{src}"
    );
    assert!(
        !src.contains("self.base") && !src.contains("pub base:"),
        "no phantom base field may survive flattening:\n{src}"
    );
}

#[test]
fn inherited_members_reach_the_wire_for_derived() {
    let src = generate_rust_for_idl("struct Base { long a; }; struct Derived : Base { long b; };");
    let derived_encode = src
        .split("impl Derived {")
        .nth(1)
        .expect("Derived impl block");
    assert!(
        derived_encode.contains("self.a.to_le_bytes()"),
        "Derived's encoder must serialize the inherited member:\n{src}"
    );
}

#[test]
fn multi_level_inheritance_flattens_deepest_base_first() {
    let src = generate_rust_for_idl(
        "struct B1 { long x; }; struct B2 : B1 { long y; }; struct D : B2 { long z; };",
    );
    assert_eq!(
        struct_def_field_names(&src, "D"),
        vec!["x".to_string(), "y".to_string(), "z".to_string()],
        "transitive bases flatten deepest-first:\n{src}"
    );
}

#[test]
fn qualified_base_name_resolves_across_modules() {
    let src = generate_rust_for_idl(
        "module A { struct Base { long a; }; }; module B { struct Derived : A::Base { long b; }; };",
    );
    assert_eq!(
        struct_def_field_names(&src, "Derived"),
        vec!["a".to_string(), "b".to_string()],
        "module-qualified base must resolve:\n{src}"
    );
}

#[test]
fn bare_base_name_resolves_in_the_enclosing_module() {
    // Two same-named `Base` structs: the bare reference inside module B must
    // bind to B::Base, not to the first-declared A::Base.
    let src = generate_rust_for_idl(
        "module A { struct Base { long wrong; }; }; module B { struct Base { long right; }; struct Derived : Base { long b; }; };",
    );
    assert_eq!(
        struct_def_field_names(&src, "Derived"),
        vec!["right".to_string(), "b".to_string()],
        "bare base binds to the innermost enclosing scope:\n{src}"
    );
}

#[test]
fn flattened_base_fields_keep_their_annotations() {
    let src = generate_rust_for_idl(
        "struct Base { @key long id; @optional long opt; }; struct Derived : Base { long b; };",
    );
    let derived = src
        .split("impl ::hdds::api::DDS for Derived {")
        .nth(1)
        .expect("Derived DDS impl");
    assert!(
        src.contains("pub opt: Option<i32>"),
        "@optional must survive flattening:\n{src}"
    );
    assert!(
        derived.contains("fn has_key() -> bool { true }"),
        "@key on an inherited member must key the derived type:\n{src}"
    );
    assert!(
        derived.contains("// Hash @key field: id"),
        "compute_key must hash the inherited @key member:\n{src}"
    );
}

#[test]
fn extensibility_is_unchanged_by_flattening() {
    // A @final derived type does not inherit the base's mutability, and a
    // @mutable derived type keeps its PL_CDR2 single-emission path with the
    // full flattened member list.
    let final_src = generate_rust_for_idl(
        "@mutable struct MBase { long a; }; @final struct FD : MBase { long b; };",
    );
    let fd = final_src.split("impl FD {").nth(1).expect("FD impl");
    assert!(
        !fd.contains("let emheader ="),
        "a @final derived type must not adopt the @mutable base's PL_CDR2 framing:\n{final_src}"
    );
    assert_eq!(
        struct_def_field_names(&final_src, "FD"),
        vec!["a".to_string(), "b".to_string()]
    );

    let mut_src =
        generate_rust_for_idl("struct FBase { long a; }; @mutable struct MD : FBase { long b; };");
    let md = mut_src.split("impl MD {").nth(1).expect("MD impl");
    assert_eq!(
        md.matches("let emheader =").count(),
        2,
        "@mutable derived type emits one EMHEADER per flattened member:\n{mut_src}"
    );
    // MD is declared last, so its emissions run to end-of-output.
    let md_all = &mut_src[mut_src.find("impl MD {").expect("MD impl")..];
    assert!(
        !md_all.contains("pub fn encode_xcdr1_le"),
        "@mutable derived type keeps its single-version PL_CDR2 emission (no XCDR1 pair):\n{mut_src}"
    );
}

#[test]
fn unresolvable_base_struct_is_a_generator_error() {
    let err = try_generate_rust_for_idl("struct Derived : Missing { long b; };")
        .expect_err("unresolvable base must be a generator error, not a panic");
    assert_eq!(err.kind, crate::error::ErrorKind::UnknownType);
    assert!(
        err.message.contains("Missing") && err.message.contains("Derived"),
        "error must name both the derived struct and the missing base: {err}"
    );
}

#[test]
fn inheritance_cycle_is_a_generator_error() {
    let err = try_generate_rust_for_idl("struct A : B { long x; }; struct B : A { long y; };")
        .expect_err("inheritance cycle must be a generator error, not infinite recursion");
    assert_eq!(err.kind, crate::error::ErrorKind::InvalidSyntax);
    assert!(
        err.message.to_lowercase().contains("cycle"),
        "error must identify the cycle: {err}"
    );

    let self_err = try_generate_rust_for_idl("struct S : S { long x; };")
        .expect_err("self-inheritance must be a generator error");
    assert_eq!(self_err.kind, crate::error::ErrorKind::InvalidSyntax);
}

#[test]
fn embedded_derived_type_object_lists_flattened_members() {
    // The type index the inline TypeObject path queries is built from the
    // flattened AST, so a struct that embeds a derived type advertises the
    // inherited members too.
    let src = generate_rust_for_idl(
        "struct Base { long a; }; struct Derived : Base { long b; }; struct Outer { Derived d; };",
    );
    let outer = src
        .split("impl ::hdds::api::DDS for Outer {")
        .nth(1)
        .expect("Outer DDS impl");
    let nested = outer
        .split(r#"CompleteTypeDetail::new("Derived")"#)
        .nth(1)
        .expect("inline Derived TypeObject");
    assert!(
        nested.contains(r#"CompleteMemberDetail::new("a")"#)
            && nested.contains(r#"CompleteMemberDetail::new("b")"#),
        "embedded Derived's TypeObject must carry the inherited member:\n{src}"
    );
}

#[test]
fn derived_member_redeclaration_is_a_generator_error() {
    // The flattened list is one member sequence; a shadowing name would emit
    // a duplicate Rust field.
    let err = try_generate_rust_for_idl("struct Base { long a; }; struct D : Base { long a; };")
        .expect_err("redeclared inherited member must be a generator error");
    assert_eq!(err.kind, crate::error::ErrorKind::DuplicateDefinition);
}

#[test]
fn flattening_does_not_leak_into_other_backends() {
    // The C/C++/Python backends render inheritance as composition off the same
    // parsed AST; the Rust pass must copy, never mutate in place.
    let mut parser =
        crate::Parser::try_new("struct Base { long a; }; struct Derived : Base { long b; };")
            .expect("lexer error");
    let file = parser.parse().expect("parse error");
    let _ = RustGenerator::new().generate(&file).expect("codegen error");
    let py = crate::codegen::Backend::Python
        .generator()
        .generate(&file)
        .expect("python codegen error");
    assert!(
        py.contains("class Derived(Base)"),
        "Python backend must still see the un-flattened AST:\n{py}"
    );
}

#[test]
fn sequential_autoid_numbers_the_flattened_member_sequence() {
    // XTypes 1.3 ss7.2.2.4.4 + ss7.3.1.2.1.1: with @autoid(SEQUENTIAL) the id
    // is the member's position in the (base-then-own) member sequence, so a
    // derived member's id continues past the inherited ones. Default hashed
    // ids are name-derived and unaffected by flattening.
    let src = generate_rust_for_idl(
        "@autoid(SEQUENTIAL) struct SB { long a; long b; }; @autoid(SEQUENTIAL) @mutable struct SD : SB { long c; };",
    );
    let sd = src.split("impl SD {").nth(1).expect("SD impl");
    for id in 0..=2u32 {
        assert!(
            sd.contains(&format!("(0x0000000{id}u32 & 0x0fff_ffff)")),
            "SD member id {id} missing from the flattened sequence:\n{src}"
        );
    }
}

/// Everything the generator emits for `Derived` (definition, encode/decode
/// impls, DDS impl, builder). All fixtures below declare `Derived` last in its
/// module, so slicing to end-of-output covers its emissions and no sibling's —
/// add a struct after `Derived` and this widens.
fn derived_emission_span(src: &str) -> &str {
    &src[src
        .find("pub struct Derived {")
        .unwrap_or_else(|| panic!("Derived struct def not found in:\n{src}"))..]
}

#[test]
fn cross_module_inherited_named_members_are_requalified() {
    // Generated modules only carry `use super::*`, so a bare `Inner` copied
    // out of module A into module B neither resolves nor, if B declared its
    // own `Inner`, would mean the same type.
    let src = generate_rust_for_idl(
        "module A { struct Inner { long v; }; struct Base { Inner i; sequence<Inner> xs; }; }; module B { struct Inner { double other; }; struct Derived : A::Base { long b; }; };",
    );
    let derived = derived_emission_span(&src);
    assert!(
        derived.contains("pub i: A::Inner,") && derived.contains("pub xs: Vec<A::Inner>"),
        "inherited named members (incl. sequence elements) must be requalified to the base's scope:\n{src}"
    );
    assert!(
        !derived.contains("<Inner>::") && !derived.contains("pub i: Inner,"),
        "no bare `Inner` may survive the copy into module B, where it would bind to B::Inner:\n{src}"
    );
}

#[test]
fn same_module_inherited_named_members_stay_bare() {
    // No scope change, no rewrite: requalifying unconditionally would emit
    // needlessly qualified paths (and churn existing output).
    let src = generate_rust_for_idl(
        "module A { struct Inner { long v; }; struct Base { Inner i; }; struct Derived : Base { long b; }; };",
    );
    let derived = derived_emission_span(&src);
    assert!(
        derived.contains("pub i: Inner,"),
        "a same-module inherited member keeps its bare type name:\n{src}"
    );
}

#[test]
fn root_scoped_inherited_named_members_stay_bare() {
    // A base at crate root: its member's type is visible from any module via
    // the `use super::*` chain, so the bare name is already correct.
    let src = generate_rust_for_idl(
        "struct Inner { long v; }; struct Base { Inner i; }; module B { struct Derived : Base { long b; }; };",
    );
    let derived = derived_emission_span(&src);
    assert!(
        derived.contains("pub i: Inner,"),
        "a root-scoped inherited member type needs no qualification:\n{src}"
    );
}

#[test]
fn root_scoped_inherited_member_shadowed_in_derived_scope_is_a_generator_error() {
    // The "fully qualified" name of a ROOT-scoped type IS its bare name, so
    // a derived module that declares the same name binds the inherited member
    // to its own type — compiling cleanly with the wrong wire layout
    // ([f64,i32] instead of [i32,i32]).
    // A root-anchored `super::Inner` spelling would bind correctly but makes
    // the name-keyed classifiers miss, which silently changes XCDR2 framing,
    // so the collision is refused instead.
    let err = try_generate_rust_for_idl(
        "struct Inner { long v; }; struct Base { Inner i; }; module B { struct Inner { double other; }; struct Derived : Base { long b; }; };",
    )
    .expect_err("a shadowed inherited member type must not silently mis-bind");
    assert_eq!(err.kind, crate::error::ErrorKind::DuplicateDefinition);
    assert!(
        err.message.contains("'Inner'") && err.message.contains("'B::Inner'"),
        "error must name both the inherited type and the shadowing one: {err}"
    );
    assert!(
        err.message.contains("B::Derived") && err.message.contains("'i'"),
        "error must locate the member it applies to: {err}"
    );
}

#[test]
fn shadowed_leading_path_segment_is_a_generator_error() {
    // Same hole one segment along: the qualified spelling `A::Inner` is itself
    // shadowed inside module B, because B declares its own module A.
    let err = try_generate_rust_for_idl(
        "module A { struct Inner { long v; }; struct Base { Inner i; }; }; module B { module A { struct Inner { double other; }; }; struct Derived : A::Base { long b; }; };",
    )
    .expect_err("a shadowed leading path segment must not silently mis-bind");
    assert_eq!(err.kind, crate::error::ErrorKind::DuplicateDefinition);
    assert!(
        err.message.contains("'A::Inner'") && err.message.contains("'B::A::Inner'"),
        "error must name both the inherited type and the shadowing one: {err}"
    );
}

#[test]
fn shadowing_module_does_not_affect_an_unrelated_inherited_member() {
    // Control for the two errors above: a shadowing declaration whose name is
    // not the inherited member's type must not block generation.
    let src = generate_rust_for_idl(
        "module A { struct Inner { long v; }; struct Base { Inner i; }; }; module B { struct Unrelated { double other; }; struct Derived : A::Base { long b; }; };",
    );
    assert!(
        derived_emission_span(&src).contains("pub i: A::Inner,"),
        "an unrelated shadow must not disturb requalification:\n{src}"
    );
}
