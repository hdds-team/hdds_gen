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
        out.contains("(0i64) as i32"),
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
        out.contains("(2i64) as i32"),
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
        out.contains("Box::new(Node::__hdds_dec(__d)?)"),
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
        out.contains("(2i64) as i32"),
        "enum default case should pick discriminant 2 to avoid collision with X(0) and Y(1), got:\n{out}"
    );
    Ok(())
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

    // Both public entry points exist and pick their version.
    assert!(out.contains("pub fn encode_xcdr1_le(&self, dst: &mut [u8])"));
    assert!(out.contains("pub fn encode_xcdr2_le(&self, dst: &mut [u8])"));
    assert!(out.contains("__hdds_cdr::Enc::new(dst, 0, false)"));
    assert!(out.contains("__hdds_cdr::Enc::new(dst, 0, true)"));

    // The element is encoded through the same cursor, which carries the
    // caller's version: never through the trait's primary version.
    let body = slice_between(&out, "impl Outer {", "pub fn __hdds_dec").expect("Outer codec");
    assert!(
        body.contains("for __x1 in self.items.iter() {")
            && body.contains("(*__x1).__hdds_enc(__e)?;"),
        "Outer must encode each element through the shared cursor. Body:\n{body}"
    );
    assert!(
        !body.contains("encode_cdr2_le"),
        "Outer must not call the trait's encode_cdr2_le. Body:\n{body}"
    );
    Ok(())
}

#[test]
fn container_outer_xcdr1_decode_invokes_sub_xcdr1_not_cdr2() -> TestResult<()> {
    let file = make_outer_with_inner_sequence();
    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    assert!(out.contains("__hdds_cdr::Dec::new(src, 0, false)"));
    assert!(out.contains("__hdds_cdr::Dec::new(src, 0, true)"));
    let start = out.find("impl Outer {").expect("Outer codec");
    let body = slice_between(&out[start..], "pub fn __hdds_dec", "\n}\n").expect("decoder");
    assert!(
        body.contains("Inner::__hdds_dec(__d)?"),
        "Outer must decode each element through the shared cursor. Body:\n{body}"
    );
    assert!(
        !body.contains("decode_cdr2_le"),
        "Outer must not call the trait's decode_cdr2_le. Body:\n{body}"
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

    assert!(out.contains("impl TaggedInner {\n    pub fn encode_xcdr1_le"));
    assert!(out.contains("    pub fn encode_xcdr2_le(&self, dst: &mut [u8])"));
    assert!(out.contains("impl Cdr2Encode for TaggedInner"));
    let body = slice_between(&out, "impl TaggedInner {", "pub fn __hdds_dec").expect("codec");
    assert!(
        body.contains("Self::Nested(__v) => {") && body.contains("(*__v).__hdds_enc(__e)?;"),
        "the Inner case must be encoded through the shared cursor. Body:\n{body}"
    );
    assert!(!body.contains("encode_cdr2_le"), "Body:\n{body}");
    Ok(())
}

#[test]
fn union_xcdr1_decode_case_named_invokes_sub_xcdr1() -> TestResult<()> {
    let file = make_tagged_union_with_inner();
    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let start = out.find("impl TaggedInner {").expect("TaggedInner codec");
    let body = slice_between(&out[start..], "pub fn __hdds_dec", "\n}\n").expect("decoder");
    assert!(
        body.contains("0 => Self::Nested(Inner::__hdds_dec(__d)?),"),
        "the Inner case must be decoded through the shared cursor. Body:\n{body}"
    );
    assert!(!body.contains("decode_cdr2_le"), "Body:\n{body}");
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
// Sec.7.4.3.4.1 Tab.15) emission proof
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

    let body = slice_between(
        &out,
        "fn encode_cdr2_le_at(\n        &self,\n        dst: &mut [u8],\n        offset: &mut usize,\n    ) -> Result<(), CdrError> {\n",
        "\n    }\n",
    )
    .expect("encode_cdr2_le_at body not found in Probe delegator");
    // The `_at` entry point aligns against the start of `dst`, so a value
    // encoded at an offset lands where the enclosing stream expects it.
    assert!(
        body.contains("self.encode_xcdr2_le_at(dst, offset)"),
        "encode_cdr2_le_at must delegate to encode_xcdr2_le_at on Probe (no @data_representation). \
         Got body:\n{body}"
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
        body.contains("self.encode_xcdr1_le_at(dst, offset)"),
        "ProbeV1 with @data_representation(XCDR1) must delegate encode_cdr2_le_at \
         -> encode_xcdr1_le_at. Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn dds_keyhash_nested_key_writes_the_nested_key_holder() -> TestResult<()> {
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

    // RTPS 2.5 9.6.4.8 step 3: a nested struct without key members
    // contributes all its members, through the KeyHolder writer (PLAIN_CDR2
    // big endian), not a little-endian encoder of the nested type.
    assert!(
        out.contains("w.write_i32(self.k.v);"),
        "nested KeyHolder member must be written through the KeyHolder writer.\nGot:\n{out}"
    );
    assert!(
        out.contains("KeyMaxSize::Bounded(4)"),
        "maximum size of {{ long v }} is 4.\nGot:\n{out}"
    );
    assert!(
        !out.contains("_kbuf"),
        "the FNV scratch-buffer KeyHash must be gone.\nGot:\n{out}"
    );
    Ok(())
}

#[test]
fn struct_delegator_emits_decode_cdr2_le_at() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
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
    .expect("decode_cdr2_le_at body not found in Probe delegator");
    assert!(
        body.contains("Self::decode_xcdr2_le_at(src, offset)"),
        "decode_cdr2_le_at must delegate to decode_xcdr2_le_at on Probe (no @data_representation). \
         Got body:\n{body}"
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
        body.contains("Self::decode_xcdr1_le_at(src, offset)"),
        "ProbeV1 with @data_representation(XCDR1) must delegate decode_cdr2_le_at \
         -> decode_xcdr1_le_at. Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn struct_inherent_emits_decode_xcdr1_le_at_wrapper() -> TestResult<()> {
    // The inherent `decode_xcdr1_le_at` reads at `*offset` with the start
    // of `src` as the alignment origin, then advances `*offset`.
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let body = slice_between(
        &out,
        "pub fn decode_xcdr1_le_at(src: &[u8], offset: &mut usize) -> Result<Self, CdrError> {\n",
        "\n    }\n",
    )
    .expect("decode_xcdr1_le_at body not found");
    assert!(
        body.contains("__hdds_cdr::Dec::new(src, *offset, false)")
            && body.contains("*offset = __d.pos;"),
        "Got body:\n{body}"
    );
    Ok(())
}

#[test]
fn struct_inherent_emits_decode_xcdr2_le_at_wrapper() -> TestResult<()> {
    // The inherent `decode_xcdr2_le_at` reads at `*offset` with the start
    // of `src` as the alignment origin, then advances `*offset`.
    let mut file = IdlFile::new();
    let mut s = Struct::new("Probe");
    s.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::Octet)));
    file.add_definition(Definition::Struct(s));

    let r#gen = RustGenerator::new();
    let out = r#gen.generate(&file)?;

    let body = slice_between(
        &out,
        "pub fn decode_xcdr2_le_at(src: &[u8], offset: &mut usize) -> Result<Self, CdrError> {\n",
        "\n    }\n",
    )
    .expect("decode_xcdr2_le_at body not found");
    assert!(
        body.contains("__hdds_cdr::Dec::new(src, *offset, true)")
            && body.contains("*offset = __d.pos;"),
        "Got body:\n{body}"
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
        out.contains("    pub fn encode_xcdr1_le(&self, dst: &mut [u8])"),
        "Color should emit encode_xcdr1_le inherent method"
    );
    assert!(
        out.contains("    pub fn encode_xcdr2_le(&self, dst: &mut [u8])"),
        "Color should emit encode_xcdr2_le inherent method"
    );
    assert!(
        out.contains("    pub fn decode_xcdr1_le"),
        "Color should emit decode_xcdr1_le inherent method"
    );
    // No @bit_bound: the value is an int32 (XTypes 1.3 7.3.1.2.1.6).
    assert!(out.contains("let b = ((*self as i64) as i32).to_le_bytes();"));
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

/// A label naming nothing hddsgen knows (a constant is replaced by its
/// value at parse time) is not a value it computes: the union's key-hash
/// method must not compile (and must not hash it as 0).
#[test]
fn union_key_hash_refuses_a_label_it_cannot_compute() -> TestResult<()> {
    let mut p =
        crate::Parser::try_new("union UK switch (long) { case Z: long a; case 2: short b; };")?;
    let ast = p.parse()?;
    let out = RustGenerator::new().generate(&ast)?;
    assert!(
        out.contains(
            "compile_error!(\"hddsgen: the label of case `a` of `UK` is not a value hddsgen can compute\")"
        ),
        "{out}"
    );
    assert!(!out.contains("(0i64)"), "{out}");
    Ok(())
}

/// The zero-wire weight of a type is computed once per named type: a
/// 40-level chain of structures each holding the previous one twice (2^40
/// paths) generates at once; the emitted weight is capped at `u32::MAX`
/// (a `usize` literal on every target).
#[test]
fn zero_wire_weight_of_a_doubling_chain_is_memoised() -> TestResult<()> {
    use std::fmt::Write as _;
    let mut idl = String::from("@final struct D0 {};\n");
    for i in 1..=40 {
        let _ = writeln!(idl, "@final struct D{i} {{ D{p} a; D{p} b; }};", p = i - 1);
    }
    idl.push_str("@final struct HD { sequence<D40> q; };\n");
    let start = std::time::Instant::now();
    let ast = crate::Parser::try_new(&idl)?.parse()?;
    let out = RustGenerator::new().generate(&ast)?;
    assert!(start.elapsed().as_secs() < 10, "{:?}", start.elapsed());
    assert!(
        out.contains(&format!("__d.storage::<D40>(__n2, {}usize)?;", u32::MAX)),
        "{out}"
    );
    Ok(())
}

/// A structure that reaches itself (here through an `@external` member)
/// counts as taking bytes: its weight is 0, and computing it terminates.
#[test]
fn zero_wire_weight_of_a_cycle_takes_bytes() -> TestResult<()> {
    let ast = crate::Parser::try_new(
        "@final struct N { @external N next; }; @final struct HN { sequence<N> q; };",
    )?
    .parse()?;
    let out = RustGenerator::new().generate(&ast)?;
    assert!(out.contains("__d.storage::<N>(__n2, 0usize)?;"), "{out}");
    Ok(())
}

/// The hdds <-> hddsgen compatibility guard (`compat_guard` module) is
/// emitted exactly once per generated file, before the first use of `hdds`
/// internals -- not once per type (a file with several types must not
/// repeat it). Mutation check: removing the
/// `output.push_str(&compat_guard::compat_guard_block());` call in
/// `generate()` makes this test fail.
#[test]
fn compat_guard_is_emitted_once_before_hdds_use() -> TestResult<()> {
    let ast = crate::Parser::try_new(
        "@key struct A { @key long id; long x; }; @key struct B { @key long id; long y; };",
    )?
    .parse()?;
    let out = RustGenerator::new().generate(&ast)?;

    assert_eq!(
        out.matches("REQUIRES_HDDS_1_5").count(),
        1,
        "the guard must appear exactly once per generated file, not per type: {out}"
    );
    assert!(
        out.contains("hdds::codegen::CODEGEN_ABI >= "),
        "missing the ABI const-assert: {out}"
    );

    let guard_pos = out.find("REQUIRES_HDDS_1_5").expect("guard present");
    let use_pos = out
        .find("use hdds::{Cdr2Encode, Cdr2Decode, CdrError};")
        .expect("hdds import present");
    assert!(
        guard_pos < use_pos,
        "the guard must come before the first use of hdds internals: {out}"
    );
    Ok(())
}

// -- Output that `cargo clippy -- -D warnings` accepts in a user crate ------
// (tests/rust_clippy.rs lints the real thing; these pin each rule's shape.)

fn rust_of(idl: &str) -> TestResult<String> {
    let ast = crate::Parser::try_new(idl)?.parse()?;
    Ok(RustGenerator::new().generate(&ast)?)
}

#[test]
fn enum_decode_widens_only_a_narrow_discriminant() -> TestResult<()> {
    let out = rust_of(
        "@bit_bound(8) enum E8 { A, B }; @bit_bound(16) enum E16 { C, D }; enum E32 { E, F };",
    )?;
    // Identity conversions are clippy::useless_conversion.
    assert!(!out.contains("i32::from(i32::"), "{out}");
    assert!(
        out.contains("Self::__hdds_from_i32(i32::from(i8::from_le_bytes(b)))?"),
        "{out}"
    );
    assert!(
        out.contains("Self::__hdds_from_i32(i32::from(i16::from_le_bytes(b)))?"),
        "{out}"
    );
    assert!(
        out.contains("Self::__hdds_from_i32(i32::from_le_bytes(b))?"),
        "{out}"
    );
    // clippy::len_zero.
    assert!(out.contains("if src.is_empty() {"), "{out}");
    assert!(!out.contains("src.len() < 1 "), "{out}");
    Ok(())
}

#[test]
fn borrowed_places_carry_no_reborrow() -> TestResult<()> {
    let out = rust_of(
        "union U switch (long) { case 1: string s; case 2: sequence<octet> o; };\n\
         struct S { sequence<string> v; string t[2]; map<string, string> m; \
         @optional string o; @external string e; };",
    )?;
    // clippy::borrow_deref_ref and clippy::explicit_auto_deref.
    for bad in ["&(*__", "&(**__", "&(*self."] {
        for call in ["__e.string(", "__e.wstring(", "__e.raw("] {
            assert!(!out.contains(&format!("{call}{bad}")), "{call}{bad}: {out}");
        }
    }
    assert!(out.contains("__e.string(__x"), "{out}");
    assert!(out.contains("__e.string(&self.e, 0)?;"), "{out}");
    Ok(())
}

#[test]
fn optional_member_decode_is_not_a_let_returned_from_a_block() -> TestResult<()> {
    let out = rust_of(
        "@final struct F { @optional string a; }; @appendable struct P { @optional string a; };",
    )?;
    // clippy::let_and_return: `let __f0 = ...; __f0 }`.
    assert!(
        !out.lines().any(|l| {
            let l = l.trim();
            l.starts_with("__f") && l.ends_with("};")
        }),
        "{out}"
    );
    assert!(
        out.contains("let __f0 = if __d.xcdr2 {"),
        "final optional is the conditional itself: {out}"
    );
    assert!(
        out.contains("{ None } else if __d.xcdr2 {"),
        "appendable optional continues the default branch: {out}"
    );
    Ok(())
}

#[test]
fn array_decode_without_dheader_is_the_expression_itself() -> TestResult<()> {
    let out = rust_of("@final struct A { octet raw[16]; long g[2][2]; };")?;
    // clippy::let_and_return: `let __a0 = ...; __a0`.
    assert!(!out.contains("let __a"), "{out}");
    Ok(())
}

#[test]
fn mutable_defaults_use_the_forms_clippy_asks_for() -> TestResult<()> {
    let out = rust_of(
        "enum E { A, B }; @final struct In { long x; };\n\
         @mutable struct M { long a; string s; sequence<long> v; E e; In i; double d; };",
    )?;
    // clippy::unnecessary_lazy_evaluations, redundant_closure, unwrap_or_default.
    assert!(out.contains("__f0.unwrap_or(0);"), "{out}");
    assert!(out.contains("__f1.unwrap_or_default();"), "{out}");
    assert!(out.contains("__f2.unwrap_or_default();"), "{out}");
    assert!(out.contains("__f3.unwrap_or_default();"), "{out}");
    assert!(
        out.contains("__f4.unwrap_or_else(In::__hdds_default);"),
        "{out}"
    );
    assert!(out.contains("__f5.unwrap_or(0.0);"), "{out}");
    assert!(!out.contains("unwrap_or_else(||"), "{out}");
    Ok(())
}

#[test]
fn a_64_bit_bitmask_is_not_cast_nor_converted_to_u64() -> TestResult<()> {
    let out = rust_of(
        "@bit_bound(64) bitmask Big { B0, B1 }; @bit_bound(16) bitmask Mid { M0, M1 };\n\
         @final struct S { Big b; Mid m; sequence<Big> v; };",
    )?;
    // clippy::unnecessary_cast and useless_conversion.
    assert!(!out.contains("as u64).to_le_bytes"), "{out}");
    assert!(!out.contains("u64::from(u64::"), "{out}");
    assert!(out.contains("u64::from(u16::from_le_bytes("), "{out}");
    assert!(out.contains("as u16).to_le_bytes"), "{out}");
    assert!(!out.contains("((("), "no unused parentheses: {out}");
    Ok(())
}

#[test]
fn names_dictated_by_the_idl_carry_a_targeted_allow() -> TestResult<()> {
    let out = rust_of(
        "module Outer { enum RGB { RED, GREEN }; @final struct S { @external string e; }; };",
    )?;
    assert!(
        out.contains(
            "#[allow(non_camel_case_types, clippy::upper_case_acronyms, clippy::enum_variant_names)]"
        ),
        "{out}"
    );
    assert!(
        out.contains("#[allow(non_snake_case, clippy::module_inception)]\npub mod Outer {"),
        "{out}"
    );
    assert!(
        out.contains("#[allow(clippy::box_collection)]\n    pub struct S {")
            || out.contains("#[allow(clippy::box_collection)]\npub struct S {"),
        "{out}"
    );
    Ok(())
}
