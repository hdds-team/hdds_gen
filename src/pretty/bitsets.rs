// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Bitset and bitmask pretty-printing.
//!
//! Renders IDL bitset and bitmask definitions back to source format.

use super::helpers::{ident, indent, push_fmt, render_annotation};
use crate::ast::{Bitmask, Bitset};
use std::fmt::Write;

pub(super) fn push_bitset(out: &mut String, b: &Bitset, level: usize) {
    for ann in &b.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }
    push_fmt(
        out,
        format_args!("{}bitset {} {{\n", indent(level), ident(&b.name)),
    );
    for field in &b.fields {
        if field.annotations.is_empty() {
            push_fmt(
                out,
                format_args!(
                    "{}bitfield<{}> {};\n",
                    indent(level + 1),
                    field.width,
                    ident(&field.name)
                ),
            );
        } else {
            let mut extras = String::new();
            for ann in &field.annotations {
                let _ = write!(&mut extras, ", {}", render_annotation(ann));
            }
            push_fmt(
                out,
                format_args!(
                    "{}bitfield<{}> {}{};\n",
                    indent(level + 1),
                    field.width,
                    ident(&field.name),
                    extras
                ),
            );
        }
    }
    push_fmt(out, format_args!("{}}};\n", indent(level)));
}

pub(super) fn push_bitmask(out: &mut String, m: &Bitmask, level: usize) {
    for ann in &m.annotations {
        push_fmt(
            out,
            format_args!("{}{}\n", indent(level), render_annotation(ann)),
        );
    }
    push_fmt(
        out,
        format_args!("{}bitmask {} {{\n", indent(level), ident(&m.name)),
    );
    for (idx, flag) in m.flags.iter().enumerate() {
        let sep = if idx + 1 == m.flags.len() { "" } else { "," };
        if flag.annotations.is_empty() {
            push_fmt(
                out,
                format_args!("{}{}{}\n", indent(level + 1), ident(&flag.name), sep),
            );
        } else {
            let mut annot_line = String::new();
            for (idx_ann, ann) in flag.annotations.iter().enumerate() {
                if idx_ann > 0 {
                    annot_line.push(' ');
                }
                annot_line.push_str(&render_annotation(ann));
            }
            push_fmt(
                out,
                format_args!(
                    "{}{}\n{}{}{}\n",
                    indent(level + 1),
                    annot_line,
                    indent(level + 1),
                    ident(&flag.name),
                    sep
                ),
            );
        }
    }
    push_fmt(out, format_args!("{}}};\n", indent(level)));
}
