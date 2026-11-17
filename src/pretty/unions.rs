// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union pretty-printing.
//!
//! Renders IDL discriminated union definitions back to source format.

use super::helpers::{
    ident, indent, push_fmt, render_annotation, render_annotations_line, scoped, split_array,
    type_to_idl,
};
use crate::ast::{Union, UnionCase, UnionLabel};
use crate::types::Annotation;
use std::fmt::Write;

pub(super) fn push_union(out: &mut String, u: &Union, level: usize) {
    for ann in &u.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }
    let mut disc_annotations = String::new();
    for ann in &u.discriminator_annotations {
        disc_annotations.push_str(&render_annotation(ann));
        disc_annotations.push(' ');
    }
    push_fmt(
        out,
        format_args!(
            "{}union {} switch({}{}) {{\n",
            indent(level),
            ident(&u.name),
            disc_annotations,
            type_to_idl(&u.discriminator)
        ),
    );

    for case in &u.cases {
        push_union_case(out, case, level + 1);
    }

    push_fmt(out, format_args!("{}}};\n", indent(level)));
}

fn push_union_case(out: &mut String, case: &UnionCase, level: usize) {
    let mut first = true;
    for lbl in &case.labels {
        match lbl {
            UnionLabel::Value(v) => {
                let v = label_to_idl(v);
                if first {
                    push_fmt(out, format_args!("{}case {v}: ", indent(level)));
                    first = false;
                } else {
                    push_fmt(out, format_args!("case {v}: "));
                }
            }
            UnionLabel::Default => {
                if first {
                    push_fmt(out, format_args!("{}default: ", indent(level)));
                    first = false;
                } else {
                    out.push_str("default: ");
                }
            }
        }
    }

    push_union_case_field(out, case, level, first);
}

pub(super) fn push_union_case_field(out: &mut String, case: &UnionCase, level: usize, first: bool) {
    if first
        && case
            .field
            .annotations
            .iter()
            .any(|a| matches!(a, Annotation::Default | Annotation::DefaultLiteral))
    {
        push_fmt(out, format_args!("{}default: ", indent(level)));
    }

    let (base, dims) = split_array(&case.field.field_type);
    let mut name_with_dims = ident(&case.field.name);
    for d in dims {
        let _ = write!(&mut name_with_dims, "[{d}]");
    }

    let annotations = render_annotations_line(&case.field.annotations);
    let sep = if annotations.is_empty() { "" } else { " " };
    push_fmt(
        out,
        format_args!(
            "{annotations}{sep}{} {};\n",
            type_to_idl(base),
            name_with_dims
        ),
    );
}

/// A case label: a (scoped) enumerator or constant name is escaped like any
/// identifier; a number, a character and `TRUE` / `FALSE` are written as is.
fn label_to_idl(v: &str) -> String {
    let named = v.starts_with("::")
        || v.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if named && v != "TRUE" && v != "FALSE" {
        scoped(v)
    } else {
        v.to_string()
    }
}
