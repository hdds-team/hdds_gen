// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Language keyword escaping for generated identifiers.
//!
//! Each target language has reserved words that cannot be used as identifiers.
//! When an IDL field name collides with a keyword, the generated code would be
//! invalid.  These helpers escape such names using language-specific conventions:
//!
//! * **Rust** -- raw identifier syntax `r#name`
//! * **Python** -- trailing underscore `name_` (PEP 8 convention)
//! * **C / C Micro** -- trailing underscore `name_`
//! * **C++** -- trailing underscore `name_`
//! * **TypeScript** -- trailing underscore `name_`

use std::borrow::Cow;

// ---------------------------------------------------------------------------
// Rust
// ---------------------------------------------------------------------------

/// Strict + reserved keywords from the Rust Reference.
const RUST_KEYWORDS: &[&str] = &[
    "Self", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
    "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Escape a field name for Rust using raw-identifier syntax (`r#name`).
#[must_use]
pub fn rust_ident(name: &str) -> Cow<'_, str> {
    if RUST_KEYWORDS.contains(&name) {
        Cow::Owned(format!("r#{name}"))
    } else {
        Cow::Borrowed(name)
    }
}

// ---------------------------------------------------------------------------
// Python
// ---------------------------------------------------------------------------

/// Hard keywords (Python 3.12+, includes soft keywords `match`, `case`, `type`).
const PYTHON_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "case", "class",
    "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if",
    "import", "in", "is", "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "type", "while", "with", "yield",
];

/// Escape a field name for Python by appending `_` (PEP 8).
#[must_use]
pub fn python_ident(name: &str) -> Cow<'_, str> {
    if PYTHON_KEYWORDS.contains(&name) {
        Cow::Owned(format!("{name}_"))
    } else {
        Cow::Borrowed(name)
    }
}

// ---------------------------------------------------------------------------
// C
// ---------------------------------------------------------------------------

/// C11 keywords that would be invalid as struct member names.
const C_KEYWORDS: &[&str] = &[
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Bool",
    "_Complex",
    "_Generic",
    "_Imaginary",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
];

/// Escape a field name for C by appending `_`.
#[must_use]
pub fn c_ident(name: &str) -> Cow<'_, str> {
    if C_KEYWORDS.contains(&name) {
        Cow::Owned(format!("{name}_"))
    } else {
        Cow::Borrowed(name)
    }
}

// ---------------------------------------------------------------------------
// C++
// ---------------------------------------------------------------------------

/// C++20 keywords (superset of C keywords).
const CPP_KEYWORDS: &[&str] = &[
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Bool",
    "_Complex",
    "_Generic",
    "_Imaginary",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "co_await",
    "co_return",
    "co_yield",
    "compl",
    "concept",
    "const",
    "const_cast",
    "consteval",
    "constexpr",
    "constinit",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "final",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "nullptr",
    "operator",
    "or",
    "or_eq",
    "override",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
    "xor_eq",
];

/// Escape a field name for C++ by appending `_`.
#[must_use]
pub fn cpp_ident(name: &str) -> Cow<'_, str> {
    if CPP_KEYWORDS.contains(&name) {
        Cow::Owned(format!("{name}_"))
    } else {
        Cow::Borrowed(name)
    }
}

// ---------------------------------------------------------------------------
// TypeScript
// ---------------------------------------------------------------------------

/// ECMAScript / TypeScript reserved words.
const TS_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Escape a field name for TypeScript by appending `_`.
#[must_use]
pub fn ts_ident(name: &str) -> Cow<'_, str> {
    if TS_KEYWORDS.contains(&name) {
        Cow::Owned(format!("{name}_"))
    } else {
        Cow::Borrowed(name)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_escapes_type() {
        assert_eq!(rust_ident("type"), "r#type");
        assert_eq!(rust_ident("match"), "r#match");
    }

    #[test]
    fn rust_passes_through_normal() {
        assert_eq!(rust_ident("x"), "x");
        assert_eq!(rust_ident("position"), "position");
    }

    #[test]
    fn python_escapes_class() {
        assert_eq!(python_ident("class"), "class_");
        assert_eq!(python_ident("type"), "type_");
    }

    #[test]
    fn python_passes_through_normal() {
        assert_eq!(python_ident("name"), "name");
    }

    #[test]
    fn c_escapes_register() {
        assert_eq!(c_ident("register"), "register_");
        assert_eq!(c_ident("default"), "default_");
    }

    #[test]
    fn c_does_not_escape_type() {
        // "type" is NOT a C keyword
        assert_eq!(c_ident("type"), "type");
    }

    #[test]
    fn cpp_escapes_class() {
        assert_eq!(cpp_ident("class"), "class_");
        assert_eq!(cpp_ident("virtual"), "virtual_");
    }

    #[test]
    fn ts_escapes_type() {
        assert_eq!(ts_ident("type"), "type_");
        assert_eq!(ts_ident("class"), "class_");
    }

    #[test]
    fn ts_passes_through_normal() {
        assert_eq!(ts_ident("value"), "value");
    }
}
