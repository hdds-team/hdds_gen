// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Struct pretty-printing.
//!
//! Renders IDL struct definitions back to source format.

use super::helpers::{
    ident, indent, push_fmt, render_annotation, scoped, split_array, type_to_idl,
};
use crate::ast::Struct;
use std::fmt::Write;

pub(super) fn push_struct(out: &mut String, s: &Struct, level: usize) {
    for ann in &s.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }

    if let Some(base) = &s.base_struct {
        push_fmt(
            out,
            format_args!(
                "{}struct {} : {} {{\n",
                indent(level),
                ident(&s.name),
                scoped(base)
            ),
        );
    } else {
        push_fmt(
            out,
            format_args!("{}struct {} {{\n", indent(level), ident(&s.name)),
        );
    }

    let mut type_width = 0usize;
    for f in &s.fields {
        let (base, _dims) = split_array(&f.field_type);
        type_width = type_width.max(type_to_idl(base).len());
    }

    for f in &s.fields {
        if let Some(line) = super::modules::render_field_annotations(&f.annotations) {
            push_fmt(out, format_args!("{}{}\n", indent(level + 1), line));
        }

        let (base, dims) = split_array(&f.field_type);
        let mut name_with_dims = ident(&f.name);
        for d in dims {
            let _ = write!(&mut name_with_dims, "[{d}]");
        }

        let base_str = type_to_idl(base);
        let pad = if type_width > base_str.len() {
            " ".repeat(type_width - base_str.len())
        } else {
            String::new()
        };

        push_fmt(
            out,
            format_args!(
                "{}{}{} {};\n",
                indent(level + 1),
                base_str,
                pad,
                name_with_dims
            ),
        );
    }

    push_fmt(out, format_args!("{}}};\n", indent(level)));
}
