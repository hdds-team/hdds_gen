// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Enum pretty-printing.
//!
//! Renders IDL enum definitions back to source format.

use super::helpers::{ident, indent, push_fmt, render_annotation};
use crate::ast::Enum;

pub(super) fn push_enum(out: &mut String, e: &Enum, level: usize) {
    for ann in &e.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }
    push_fmt(
        out,
        format_args!("{}enum {} {{\n", indent(level), ident(&e.name)),
    );

    let max_name_len = e
        .variants
        .iter()
        .filter(|v| v.value.is_some())
        .map(|v| ident(&v.name).len())
        .max()
        .unwrap_or(0);

    for (idx, v) in e.variants.iter().enumerate() {
        let sep = if idx + 1 == e.variants.len() { "" } else { "," };
        let name = ident(&v.name);
        match v.value {
            Some(val) => {
                let pad = if max_name_len > name.len() {
                    " ".repeat(max_name_len - name.len())
                } else {
                    String::new()
                };
                push_fmt(
                    out,
                    format_args!("{}{}{} = {}{}\n", indent(level + 1), name, pad, val, sep),
                );
            }
            None => push_fmt(out, format_args!("{}{}{}\n", indent(level + 1), name, sep)),
        }
    }

    push_fmt(out, format_args!("{}}};\n", indent(level)));
}
