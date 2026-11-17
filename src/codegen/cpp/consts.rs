// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Const code generation for C++.
//!
//! Generates C++ constexpr constants from IDL const declarations.

use super::helpers::{escape_string_literal, push_fmt, type_to_cpp};
use super::CppGenerator;
use crate::ast::Const;
use crate::types::{IdlType, PrimitiveType};

pub(super) fn generate_const(generator: &CppGenerator, c: &Const) -> String {
    let mut output = String::new();
    let indent = generator.indent();
    // A constant named like a C++ keyword (`_delete`) gets the backend's
    // usual escape (`delete_`).
    let name = crate::codegen::keywords::cpp_ident(&c.name);
    match &c.const_type {
        IdlType::Primitive(PrimitiveType::String) | IdlType::BoundedString { wide: false, .. } => {
            let escaped = escape_string_literal(&c.value, false);
            push_fmt(
                &mut output,
                format_args!("{indent}inline constexpr auto {name} = \"{escaped}\";\n\n"),
            );
        }
        IdlType::Primitive(PrimitiveType::WString) | IdlType::BoundedString { wide: true, .. } => {
            let escaped = escape_string_literal(&c.value, true);
            push_fmt(
                &mut output,
                format_args!("{indent}inline constexpr auto {name} = L\"{escaped}\";\n\n"),
            );
        }
        _ => {
            let ty = type_to_cpp(&c.const_type);
            let value =
                crate::codegen::const_literal::literal(c, crate::codegen::const_literal::Lang::Cpp);
            push_fmt(
                &mut output,
                format_args!("{indent}inline constexpr {ty} {name} = {value};\n\n"),
            );
        }
    }
    output
}
