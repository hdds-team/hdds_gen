// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Helper utilities for C++ code generation.
//!
//! Name manipulation, type mapping, and formatting helpers.

use crate::types::{IdlType, PrimitiveType};
use std::fmt::Write;

pub(super) fn push_fmt(dst: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = dst.write_fmt(args);
}

/// Returns the last identifier in a qualified name (e.g., `Foo::Bar::Baz` -> `Baz`)
pub(super) fn last_ident(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

pub(super) fn type_to_cpp(idl_type: &IdlType) -> String {
    match idl_type {
        // `string<N>` / `wstring<N>`: N characters, NUL-terminated when
        // shorter.
        IdlType::BoundedString { wide, bound } => {
            let c = if *wide { "wchar_t" } else { "char" };
            format!("std::array<{c}, {bound}>")
        }
        IdlType::Primitive(p) => match p {
            PrimitiveType::Fixed { digits, scale } => format!("Fixed<{digits}, {scale}>"),
            other => other.to_cpp_name().to_string(),
        },
        IdlType::Named(name) => name.clone(),
        // A sequence (`sequence<char>` included) holds 0 to N elements; the
        // codec enforces the bound on both sides.
        IdlType::Sequence { inner, .. } => format!("std::vector<{}>", type_to_cpp(inner)),
        IdlType::Map { key, value, .. } => format!(
            "std::map<{k}, {v}>",
            k = type_to_cpp(key),
            v = type_to_cpp(value)
        ),
        IdlType::Array { inner, size } => {
            format!("std::array<{inner}, {size}>", inner = type_to_cpp(inner))
        }
    }
}

/// Returns a C++ default initializer suffix for a primitive type.
/// e.g., `= 0` for integers, `= 0.0` for floats, `= false` for bool.
/// Returns empty string for types with implicit default (string, vector, etc.).
pub(super) const fn cpp_default_init(idl_type: &IdlType) -> &'static str {
    match idl_type {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => " = false",
            PrimitiveType::Char
            | PrimitiveType::WChar
            | PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Short
            | PrimitiveType::Int16
            | PrimitiveType::UnsignedShort
            | PrimitiveType::UInt16
            | PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32
            | PrimitiveType::LongLong
            | PrimitiveType::Int64
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::UInt64
            | PrimitiveType::Fixed { .. }
            | PrimitiveType::Void => " = 0",
            PrimitiveType::Float => " = 0.0f",
            PrimitiveType::Double | PrimitiveType::LongDouble => " = 0.0",
            // String types have implicit default (empty string)
            PrimitiveType::String | PrimitiveType::WString => "",
        },
        // Named types, sequences, arrays, maps all have implicit default constructors
        _ => "",
    }
}

pub(super) fn escape_string_literal(value: &str, wide: bool) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\0' => escaped.push_str("\\0"),
            c if c.is_ascii() && !c.is_control() => escaped.push(c),
            c => {
                let code = u32::from(c);
                if wide {
                    if code <= 0xFFFF {
                        push_fmt(&mut escaped, format_args!("\\u{code:04X}"));
                    } else {
                        push_fmt(&mut escaped, format_args!("\\U{code:08X}"));
                    }
                } else if code <= 0xFF {
                    push_fmt(&mut escaped, format_args!("\\x{code:02X}"));
                } else {
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    for byte in encoded.as_bytes() {
                        push_fmt(&mut escaped, format_args!("\\x{byte:02X}"));
                    }
                }
            }
        }
    }
    escaped
}
