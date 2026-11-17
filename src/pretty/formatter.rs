// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Top-level formatting dispatcher.
//!
//! Routes AST definitions to their respective pretty-printers.

#[cfg(feature = "interfaces")]
use super::interfaces;
use super::{
    bitsets, enums,
    helpers::{ident, indent, push_fmt, type_to_idl},
    modules, structs, unions,
};
use crate::ast::{Definition, IdlFile};

pub(super) fn push_def(out: &mut String, def: &Definition, level: usize) {
    match def {
        Definition::Module(m) => modules::push_module(out, m, level),
        Definition::Struct(s) => structs::push_struct(out, s, level),
        Definition::Typedef(t) => modules::push_typedef(out, t, level),
        Definition::Enum(e) => enums::push_enum(out, e, level),
        Definition::Union(u) => unions::push_union(out, u, level),
        Definition::AnnotationDecl(ad) => modules::push_annotation_decl(out, ad, level),
        Definition::Const(c) => {
            push_fmt(
                out,
                format_args!(
                    "{}const {} {} = {};\n",
                    indent(level),
                    type_to_idl(&c.const_type),
                    ident(&c.name),
                    crate::codegen::const_literal::literal(
                        c,
                        crate::codegen::const_literal::Lang::Idl
                    )
                ),
            );
        }
        Definition::Bitset(b) => bitsets::push_bitset(out, b, level),
        Definition::Bitmask(m) => bitsets::push_bitmask(out, m, level),
        Definition::ForwardDecl(f) => {
            let kind = match f.kind {
                crate::ast::ForwardKind::Struct => "struct",
                crate::ast::ForwardKind::Union => "union",
            };
            push_fmt(
                out,
                format_args!("{}{} {};\n", indent(level), kind, ident(&f.name)),
            );
        }
        #[cfg(feature = "interfaces")]
        Definition::Interface(i) => interfaces::push_interface(out, i, level),
        #[cfg(feature = "interfaces")]
        Definition::Exception(e) => interfaces::push_exception(out, e, level),
    }
}

#[must_use]
/// Render an IDL file back into textual IDL form.
pub fn to_idl(file: &IdlFile) -> String {
    let mut out = String::new();
    for def in &file.definitions {
        push_def(&mut out, def, 0);
        out.push('\n');
    }
    out
}
