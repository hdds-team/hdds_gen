// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Helper utilities for pretty-printing.
//!
//! Indentation, type rendering, and annotation formatting.

use crate::types::{quote_idl_string, Annotation, ExtensibilityKind, IdlType, PrimitiveType};
use std::fmt::Write;

pub(super) fn indent(n: usize) -> String {
    "    ".repeat(n)
}

/// IDL 4.2 keywords (7.2.4, Table 7-6), plus the boolean literals and the
/// type aliases this parser also reads as keywords.
const IDL_KEYWORDS: &[&str] = &[
    "abstract",
    "any",
    "alias",
    "attribute",
    "bitfield",
    "bitmask",
    "bitset",
    "boolean",
    "case",
    "char",
    "component",
    "connector",
    "const",
    "consumes",
    "context",
    "custom",
    "default",
    "double",
    "exception",
    "emits",
    "enum",
    "eventtype",
    "factory",
    "false",
    "finder",
    "fixed",
    "float",
    "getraises",
    "getter",
    "home",
    "import",
    "in",
    "inout",
    "interface",
    "local",
    "long",
    "manages",
    "map",
    "mirrorport",
    "module",
    "multiple",
    "native",
    "object",
    "octet",
    "oneway",
    "out",
    "primarykey",
    "private",
    "port",
    "porttype",
    "provides",
    "public",
    "publishes",
    "raises",
    "readonly",
    "setraises",
    "setter",
    "sequence",
    "short",
    "string",
    "struct",
    "supports",
    "switch",
    "true",
    "truncatable",
    "typedef",
    "typeid",
    "typename",
    "typeprefix",
    "unsigned",
    "union",
    "uses",
    "valuebase",
    "valuetype",
    "void",
    "wchar",
    "wstring",
    "int8",
    "uint8",
    "int16",
    "int32",
    "int64",
    "uint16",
    "uint32",
    "uint64",
];

/// `name` as an IDL identifier: escaped with a leading underscore (IDL 4.2
/// 7.2.3.2) when it collides with a keyword (case-insensitively, as the
/// collision rule 7.2.3.1 reads) or itself begins with an underscore, so it parses
/// back to `name`.
pub(super) fn ident(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if name.starts_with('_')
        || IDL_KEYWORDS.contains(&lower.as_str())
        || crate::token::TokenKind::from_keyword(name).is_some()
    {
        format!("_{name}")
    } else {
        name.to_string()
    }
}

/// A scoped name (`a::b::T`, or absolute `::a::T`) with each segment
/// escaped as [`ident`] does.
pub(super) fn scoped(name: &str) -> String {
    let (prefix, rest) = name
        .strip_prefix("::")
        .map_or(("", name), |rest| ("::", rest));
    let parts: Vec<String> = rest.split("::").map(ident).collect();
    format!("{prefix}{}", parts.join("::"))
}

pub(super) fn type_to_idl(t: &IdlType) -> String {
    match t {
        IdlType::Primitive(PrimitiveType::Fixed { digits, scale }) => {
            format!("fixed<{digits}, {scale}>")
        }
        IdlType::Primitive(p) => p.to_idl_string().to_string(),
        IdlType::Named(name) => scoped(name),
        IdlType::BoundedString { .. } => t.to_idl_string(),
        IdlType::Sequence { inner, bound } => bound.map_or_else(
            || format!("sequence<{}>", type_to_idl(inner)),
            |b| format!("sequence<{}, {b}>", type_to_idl(inner)),
        ),
        IdlType::Map { key, value, bound } => bound.map_or_else(
            || format!("map<{}, {}>", type_to_idl(key), type_to_idl(value)),
            |b| format!("map<{}, {}, {b}>", type_to_idl(key), type_to_idl(value)),
        ),
        // An anonymous array only appears inside a template argument, which
        // IDL does not allow; split_array handles declarator arrays.
        IdlType::Array { inner, size } => format!("{}[{size}]", type_to_idl(inner)),
    }
}

pub(super) fn push_fmt(dst: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = dst.write_fmt(args);
}

pub(super) fn split_array(t: &IdlType) -> (&IdlType, Vec<u32>) {
    let mut dims = Vec::new();
    let mut cur = t;
    while let IdlType::Array { inner, size } = cur {
        dims.push(*size);
        cur = inner.as_ref();
    }
    (cur, dims)
}

pub(super) fn render_annotations_line(anns: &[Annotation]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for a in anns {
        parts.push(render_annotation(a));
    }
    parts.join(" ")
}

pub(super) fn render_annotation(a: &Annotation) -> String {
    match a {
        Annotation::Key => "@key".to_string(),
        Annotation::Optional => "@optional".to_string(),
        Annotation::Id(n) => format!("@id({n})"),
        Annotation::AutoId(kind) => match kind {
            crate::types::AutoIdKind::Sequential => "@autoid(SEQUENTIAL)".to_string(),
            crate::types::AutoIdKind::Hash => "@autoid(HASH)".to_string(),
        },
        Annotation::Position(n) => format!("@position({n})"),
        Annotation::BitBound(n) => format!("@bit_bound({n})"),
        Annotation::Unit(u) => format!("@unit({})", quote_idl_string(u)),
        Annotation::Min(v) => format!("@min({})", param_to_idl(v)),
        Annotation::Max(v) => format!("@max({})", param_to_idl(v)),
        Annotation::Range { min, max } => format!(
            "@range(min={}, max={})",
            param_to_idl(min),
            param_to_idl(max)
        ),
        Annotation::Extensibility(kind) => match kind {
            ExtensibilityKind::Final => "@extensibility(FINAL)".to_string(),
            ExtensibilityKind::Appendable => "@extensibility(APPENDABLE)".to_string(),
            ExtensibilityKind::Mutable => "@extensibility(MUTABLE)".to_string(),
        },
        Annotation::Final => "@final".to_string(),
        Annotation::Appendable => "@appendable".to_string(),
        Annotation::Mutable => "@mutable".to_string(),
        Annotation::MustUnderstand => "@must_understand".to_string(),
        Annotation::Default => "@default".to_string(),
        Annotation::Value(v) => format!("@default({})", param_to_idl(v)),
        Annotation::DefaultLiteral => "@default_literal".to_string(),
        Annotation::Service => "@service".to_string(),
        Annotation::Oneway => "@oneway".to_string(),
        Annotation::Ami => "@ami".to_string(),
        Annotation::Topic { name, platform } => {
            let params: Vec<String> = [("name", name), ("platform", platform)]
                .into_iter()
                .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", quote_idl_string(v))))
                .collect();
            if params.is_empty() {
                "@topic".to_string()
            } else {
                format!("@topic({})", params.join(", "))
            }
        }
        Annotation::Verbatim {
            language,
            placement,
            text,
        } => format!(
            "@verbatim(language={}, placement={}, text={})",
            quote_idl_string(language),
            literal_to_idl(placement),
            quote_idl_string(text)
        ),
        Annotation::External => "@external".to_string(),
        Annotation::Nested => "@nested".to_string(),
        Annotation::DataRepresentation(v) => format!("@data_representation({v})"),
        Annotation::NonSerialized => "@non_serialized".to_string(),
        Annotation::Custom { name, params } => {
            if params.is_empty() {
                format!("@{name}")
            } else {
                let inner: Vec<String> = params
                    .iter()
                    .map(|p| {
                        if p.name.is_empty() {
                            param_to_idl(p)
                        } else {
                            format!("{}={}", p.name, param_to_idl(p))
                        }
                    })
                    .collect();
                format!("@{name}({})", inner.join(", "))
            }
        }
    }
}

/// A builtin annotation value whose literal kind the AST does not keep
/// (the `@verbatim` placement, an annotation member default): a number or
/// a boolean is written as is, a (scoped) name escaped where it is a
/// keyword, anything else as a quoted string, so the output always parses
/// back to the same value.
pub(super) fn literal_to_idl(v: &str) -> String {
    let is_number = !v.is_empty()
        && v.trim_start_matches('-')
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'));
    let is_name = !v.is_empty()
        && v.trim_start_matches("::").split("::").all(|seg| {
            seg.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if is_number || matches!(v, "TRUE" | "FALSE" | "true" | "false") {
        v.to_string()
    } else if is_name {
        // A name that is a keyword is escaped (`_struct`), so it is read
        // back as the same name, never as a keyword or a string.
        scoped(v)
    } else {
        quote_idl_string(v)
    }
}

/// An annotation parameter value as an IDL literal of its kind; a name is
/// escaped where it collides with a keyword (`@default(_octet)`).
pub(super) fn param_to_idl(p: &crate::types::AnnotationParam) -> String {
    if p.kind == crate::types::AnnotationParamKind::Identifier {
        scoped(&p.value)
    } else {
        p.value_to_idl()
    }
}
