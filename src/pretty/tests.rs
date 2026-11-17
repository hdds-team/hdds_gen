// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Unit tests for the pretty-printer.

#![allow(clippy::pedantic)]

use super::to_idl;
use crate::ast::*;
use crate::parser::Parser;
use crate::types::{Annotation, IdlType, PrimitiveType};

fn build_msg_struct() -> Struct {
    use PrimitiveType::*;

    let mut s = Struct::new("Msg");
    s.annotations.push(Annotation::Appendable);
    s.add_field(Field::new("id", IdlType::Primitive(Int32)).with_annotation(Annotation::Key));

    let mut content = Field::new("content", IdlType::Primitive(String));
    content.annotations.push(Annotation::Optional);
    s.add_field(content);

    let str_field = Field::new(
        "name",
        IdlType::BoundedString {
            wide: false,
            bound: 16,
        },
    );
    s.add_field(str_field);

    let wide_field = Field::new(
        "wname",
        IdlType::BoundedString {
            wide: true,
            bound: 32,
        },
    );
    s.add_field(wide_field);

    let int_seq = Field::new(
        "small",
        IdlType::Sequence {
            inner: Box::new(IdlType::Primitive(Int32)),
            bound: Some(10),
        },
    );
    s.add_field(int_seq);
    s
}

fn build_color_enum() -> Enum {
    let mut e = Enum::new("Color");
    e.add_variant(EnumVariant::new("Red", Some(0)));
    e.add_variant(EnumVariant::new("Green", Some(1)));
    e.add_variant(EnumVariant::new("Blue", Some(2)));
    e
}

fn build_map_typedef() -> Typedef {
    use PrimitiveType::Int32;

    Typedef::new(
        "MapType",
        IdlType::Map {
            key: Box::new(IdlType::Primitive(PrimitiveType::String)),
            value: Box::new(IdlType::Primitive(Int32)),
            bound: Some(100),
        },
    )
}

fn build_flags_bitset() -> Bitset {
    let mut bs = Bitset::new("Flags");
    bs.add_field(BitfieldDecl::new(3, "mode"));
    let mut bf = BitfieldDecl::new(5, "value");
    bf.annotations.push(Annotation::Position(4));
    bs.add_field(bf);
    bs
}

fn build_perm_bitmask() -> Bitmask {
    let mut bm = Bitmask::new("Perm");
    bm.add_flag(BitmaskFlag::new("Read"));
    bm.add_flag(BitmaskFlag::new("Write"));
    bm
}

fn build_data_union() -> Union {
    use PrimitiveType::{Int32, Octet};

    let mut u = Union::new("Data", IdlType::Primitive(Int32));
    u.add_case(UnionCase {
        labels: vec![UnionLabel::Value("1".into())],
        field: Field::new("i", IdlType::Primitive(Int32)),
    });
    u.add_case(UnionCase {
        labels: vec![UnionLabel::Default],
        field: Field::new("raw", IdlType::Primitive(Octet)),
    });
    u
}

fn build_composite_file() -> IdlFile {
    let mut module = Module::new("Comp");
    module.add_definition(Definition::Struct(build_msg_struct()));
    module.add_definition(Definition::Enum(build_color_enum()));
    module.add_definition(Definition::Typedef(build_map_typedef()));
    module.add_definition(Definition::Bitset(build_flags_bitset()));
    module.add_definition(Definition::Bitmask(build_perm_bitmask()));
    module.add_definition(Definition::Union(build_data_union()));

    let mut file = IdlFile::new();
    file.add_definition(Definition::Module(module));
    file
}

fn assert_sequence_fields(idl: &str) {
    let mut has_str = false;
    let mut has_wstr = false;

    for line in idl.lines() {
        let trimmed = line.trim_start();
        if trimmed.contains("string<16>") && trimmed.contains("name;") {
            has_str = true;
        }
        if trimmed.contains("wstring<32>") && trimmed.contains("wname;") {
            has_wstr = true;
        }
    }

    assert!(has_str, "missing aligned string<16> name; in output\n{idl}");
    assert!(
        has_wstr,
        "missing aligned wstring<32> wname; in output\n{idl}"
    );
}

#[test]
fn pretty_basic_struct() {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Point");
    s.add_field(Field::new("x", IdlType::Primitive(PrimitiveType::Int32)));
    s.add_field(Field::new("y", IdlType::Primitive(PrimitiveType::Int32)));
    file.add_definition(Definition::Struct(s));

    let idl = to_idl(&file);
    assert!(idl.contains("struct Point {"));
    assert!(idl.contains("int32_t x;"));
    assert!(idl.contains("int32_t y;"));
    assert!(idl.contains("};"));
}

#[test]
fn pretty_roundtrip_composite() -> Result<(), String> {
    let file = build_composite_file();
    let idl = to_idl(&file);
    assert_sequence_fields(&idl);

    let mut parser = Parser::try_new(&idl).map_err(|e| e.to_string())?;
    let parsed = parser.parse().map_err(|e| e.to_string())?;
    assert!(parsed
        .definitions
        .iter()
        .any(|def| matches!(def, Definition::Module(m) if m.name == "Comp")));
    Ok(())
}

#[test]
fn pretty_union_default_via_annotation() -> Result<(), String> {
    let input = r#"
        union U switch(int32_t) {
            @default int32_t a;
        };
    "#;
    let mut parser = Parser::try_new(input).map_err(|e| e.to_string())?;
    let ast = parser.parse().map_err(|e| e.to_string())?;
    let idl = to_idl(&ast);
    assert!(idl.contains("default: int32_t a;"), "{idl}");
    Ok(())
}

#[test]
fn pretty_union_multilabel_inline() {
    let mut u = Union::new("U", IdlType::Primitive(PrimitiveType::Int32));
    u.add_case(UnionCase {
        labels: vec![UnionLabel::Value("1".into()), UnionLabel::Value("2".into())],
        field: Field::new("v", IdlType::Primitive(PrimitiveType::Int32)),
    });
    let mut file = IdlFile::new();
    file.add_definition(Definition::Union(u));
    let idl = to_idl(&file);
    assert!(idl.contains("case 1: case 2: int32_t v;"));
}

#[test]
fn pretty_enum_alignment_columns() {
    let mut e = Enum::new("E");
    e.add_variant(EnumVariant::new("A", Some(1)));
    e.add_variant(EnumVariant::new("LongName", Some(42)));
    e.add_variant(EnumVariant::new("Z", None));
    let file = IdlFile {
        definitions: vec![Definition::Enum(e)],
    };
    let s = to_idl(&file);
    let cols: Vec<usize> = s.lines().filter_map(|line| line.find('=')).collect();
    assert!(cols.len() >= 2, "expected aligned enum values\n{s}");
    assert!(cols.windows(2).all(|w| w[0] == w[1]));
}

#[test]
fn pretty_struct_field_alignment_columns() {
    let mut sdef = Struct::new("S");
    sdef.add_field(Field::new("a", IdlType::Primitive(PrimitiveType::UInt32)));
    sdef.add_field(Field::new("b", IdlType::Primitive(PrimitiveType::String)));
    sdef.add_field(Field::new("c", IdlType::Primitive(PrimitiveType::WString)));
    let file = IdlFile {
        definitions: vec![Definition::Struct(sdef)],
    };
    let rendered = to_idl(&file);
    let cols: Vec<usize> = rendered
        .lines()
        .filter(|line| line.trim().ends_with(';'))
        .filter_map(|line| {
            line.find('a')
                .or_else(|| line.find('b'))
                .or_else(|| line.find('c'))
        })
        .collect();
    assert!(cols.len() >= 3, "expected three fields aligned\n{rendered}");
    assert!(cols.iter().all(|&c| c == cols[0]));
}

#[test]
fn union_key_discriminator_round_trips() -> Result<(), String> {
    let idl = "enum K { A, B };\nunion C switch (@key K) { case A: long a; case B: double b; };\n";
    let mut p = Parser::try_new(idl).map_err(|e| e.to_string())?;
    let ast = p.parse().map_err(|e| e.to_string())?;
    let keyed = ast
        .definitions
        .iter()
        .any(|d| matches!(d, Definition::Union(u) if u.discriminator_is_key()));
    assert!(keyed, "the discriminator is a key");
    let printed = to_idl(&ast);
    assert!(printed.contains("switch(@key K)"), "{printed}");
    Ok(())
}
