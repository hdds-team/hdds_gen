// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Unit tests for the C code generator.

#![allow(clippy::expect_used)]

use super::CGenerator;
use crate::ast::{Definition, Enum, EnumVariant, Field, IdlFile, Struct};
use crate::codegen::CodeGenerator;
use crate::types::{IdlType, PrimitiveType};
use std::error::Error;

type TestResult<T> = std::result::Result<T, Box<dyn Error>>;

#[test]
fn c_generates_struct_and_enum() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut e = Enum::new("Color");
    e.add_variant(EnumVariant::new("Red", Some(0)));
    e.add_variant(EnumVariant::new("Green", Some(1)));
    e.add_variant(EnumVariant::new("Blue", Some(2)));
    file.add_definition(Definition::Enum(e));

    let mut s = Struct::new("Point");
    s.add_field(Field::new("x", IdlType::Primitive(PrimitiveType::Int32)));
    s.add_field(Field::new("y", IdlType::Primitive(PrimitiveType::Int32)));
    file.add_definition(Definition::Struct(s));

    let cg = CGenerator::new();
    let code = cg.generate(&file)?;
    assert!(code.contains("#include <stdint.h>"));
    assert!(code.contains("typedef enum"));
    assert!(code.contains("COLOR_RED"));
    assert!(code.contains("typedef struct Point"));
    assert!(code.contains("int32_t x;"));
    assert!(code.contains("int32_t y;"));
    assert!(code.contains("static inline int point_encode_cdr2_le_at("));
    assert!(code.contains("static inline int point_encode_cdr2_le("));
    assert!(code.contains("static inline int point_decode_cdr2_le_at("));
    assert!(code.contains("static inline int point_decode_cdr2_le("));
    assert!(code.contains("static inline size_t point_max_cdr2_size("));
    Ok(())
}

#[test]
fn c_generates_fixed_point_support() -> TestResult<()> {
    let mut file = IdlFile::new();
    let mut s = Struct::new("Money");
    s.add_field(Field::new(
        "amount",
        IdlType::Primitive(PrimitiveType::Fixed {
            digits: 10,
            scale: 2,
        }),
    ));
    file.add_definition(Definition::Struct(s));

    let cg = CGenerator::new();
    let code = cg.generate(&file)?;

    assert!(
        code.contains("typedef struct {\n    int64_t high;\n    uint64_t low;\n} cdr_fixed128_t;"),
        "missing fixed-point helper type in header prelude"
    );
    assert!(
        code.contains("cdr_fixed128_t /* fixed<10, 2> */ amount;"),
        "fixed field did not map to raw storage"
    );
    assert!(
        code.contains("cdr_need_write(len, offset, CDR_SIZE_FIXED128)"),
        "encode path did not reserve 16 bytes for fixed field"
    );
    assert!(
        code.contains("cdr_need_read(len, offset, CDR_SIZE_FIXED128)"),
        "decode path did not reserve 16 bytes for fixed field"
    );
    Ok(())
}
