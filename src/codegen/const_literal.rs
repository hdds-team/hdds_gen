// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The value of an IDL constant as a literal of each target language.
//!
//! `Const::value` holds the evaluated value as text: a string or a
//! character unquoted and unescaped, a boolean `true` / `false`, a number
//! in Rust's notation. The declared type says how to write it back.

use crate::ast::Const;
use crate::types::{quote_idl_char, quote_idl_string, IdlType, PrimitiveType};

/// Target language of a constant literal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// IDL itself (the pretty-printer).
    Idl,
    /// C (C89 included: a boolean is `1` / `0`).
    C,
    /// C++.
    Cpp,
    /// Rust.
    Rust,
    /// Python.
    Python,
    /// TypeScript.
    TypeScript,
}

enum Category {
    Str { wide: bool },
    Char { wide: bool },
    Bool,
    Float,
    Other,
}

const fn category(t: &IdlType) -> Category {
    match t {
        IdlType::Primitive(PrimitiveType::String) => Category::Str { wide: false },
        IdlType::Primitive(PrimitiveType::WString) => Category::Str { wide: true },
        IdlType::BoundedString { wide, .. } => Category::Str { wide: *wide },
        IdlType::Primitive(PrimitiveType::Char) => Category::Char { wide: false },
        IdlType::Primitive(PrimitiveType::WChar) => Category::Char { wide: true },
        IdlType::Primitive(PrimitiveType::Boolean) => Category::Bool,
        IdlType::Primitive(
            PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble,
        ) => Category::Float,
        _ => Category::Other,
    }
}

/// The value of `c` as a literal of `lang`.
#[must_use]
pub fn literal(c: &Const, lang: Lang) -> String {
    let v = c.value.as_str();
    let wide_prefix = |wide: bool| {
        if wide && matches!(lang, Lang::Idl | Lang::C | Lang::Cpp) {
            "L"
        } else {
            ""
        }
    };
    match category(&c.const_type) {
        Category::Str { wide } => format!("{}{}", wide_prefix(wide), quote_idl_string(v)),
        Category::Char { wide } => match lang {
            // No character type: a one-character string.
            Lang::Python | Lang::TypeScript => quote_idl_string(v),
            _ => format!("{}{}", wide_prefix(wide), quote_idl_char(v)),
        },
        Category::Bool => {
            let b = v == "true";
            match (lang, b) {
                (Lang::Idl, true) => "TRUE".into(),
                (Lang::Idl, false) => "FALSE".into(),
                (Lang::Python, true) => "True".into(),
                (Lang::Python, false) => "False".into(),
                (Lang::C, true) => "1".into(),
                (Lang::C, false) => "0".into(),
                (_, true) => "true".into(),
                (_, false) => "false".into(),
            }
        }
        Category::Float => {
            if v.contains(['.', 'e', 'E', 'n', 'i']) {
                v.to_string()
            } else {
                format!("{v}.0")
            }
        }
        Category::Other => v.to_string(),
    }
}
