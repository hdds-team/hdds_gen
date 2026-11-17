// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Module, typedef, const, and annotation pretty-printing.
//!
//! Renders top-level IDL constructs back to source format.

use super::formatter::push_def;
use super::helpers::{
    ident, indent, literal_to_idl, push_fmt, render_annotation, render_annotations_line,
    split_array, type_to_idl,
};
use crate::ast::{AnnotationDecl, Definition, Module, Typedef};
use crate::types::Annotation;
use std::fmt::Write;

pub(super) fn push_module(out: &mut String, m: &Module, level: usize) {
    for ann in &m.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }
    push_fmt(
        out,
        format_args!("{}module {} {{\n", indent(level), ident(&m.name)),
    );
    let mut i = 0usize;
    while i < m.definitions.len() {
        if matches!(m.definitions[i], Definition::Typedef(_)) {
            let start = i;
            let mut max_width = 0usize;
            while i < m.definitions.len() {
                if let Definition::Typedef(td) = &m.definitions[i] {
                    let (base, _dims) = split_array(&td.base_type);
                    let type_str = type_to_idl(base);
                    max_width = max_width.max(type_str.len());
                    i += 1;
                } else {
                    break;
                }
            }
            for idx in start..i {
                if let Definition::Typedef(td) = &m.definitions[idx] {
                    push_typedef_with_pad(out, td, level + 1, Some(max_width));
                }
            }
            continue;
        }

        push_def(out, &m.definitions[i], level + 1);
        i += 1;
    }
    push_fmt(out, format_args!("{}}};\n", indent(level)));
}

pub(super) fn push_typedef(out: &mut String, t: &Typedef, level: usize) {
    push_typedef_with_pad(out, t, level, None);
}

pub(super) fn push_typedef_with_pad(
    out: &mut String,
    t: &Typedef,
    level: usize,
    align_to: Option<usize>,
) {
    for ann in &t.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }

    let (base, dims) = split_array(&t.base_type);
    let base_str = type_to_idl(base);
    let pad = align_to
        .map(|width| {
            if width > base_str.len() {
                " ".repeat(width - base_str.len())
            } else {
                String::new()
            }
        })
        .unwrap_or_default();

    if dims.is_empty() {
        push_fmt(
            out,
            format_args!(
                "{}typedef {}{} {};\n",
                indent(level),
                base_str,
                pad,
                ident(&t.name)
            ),
        );
    } else {
        let mut name_with_dims = ident(&t.name);
        for d in dims {
            let _ = write!(&mut name_with_dims, "[{d}]");
        }
        push_fmt(
            out,
            format_args!(
                "{}typedef {}{} {};\n",
                indent(level),
                base_str,
                pad,
                name_with_dims
            ),
        );
    }
}

pub(super) fn push_annotation_decl(out: &mut String, ad: &AnnotationDecl, level: usize) {
    push_fmt(
        out,
        format_args!("{}@annotation {} {{\n", indent(level), ident(&ad.name)),
    );
    for member in &ad.members {
        if let Some(default) = &member.default {
            push_fmt(
                out,
                format_args!(
                    "{}{} {} default {};\n",
                    indent(level + 1),
                    member.ty,
                    ident(&member.name),
                    literal_to_idl(default)
                ),
            );
        } else {
            push_fmt(
                out,
                format_args!(
                    "{}{} {};\n",
                    indent(level + 1),
                    member.ty,
                    ident(&member.name)
                ),
            );
        }
    }
    push_fmt(out, format_args!("{}}};\n", indent(level)));
}

pub(super) fn render_field_annotations(anns: &[Annotation]) -> Option<String> {
    let line = render_annotations_line(anns);
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}
