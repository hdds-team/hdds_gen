// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union code generation for Rust.
//!
//! Generates Rust enum representations of IDL discriminated unions
//! with `Cdr2Encode`/`Cdr2Decode` implementations.

use super::helpers::to_pascal_case;
use super::{push_fmt, RustGenerator};
use crate::ast::{Union, UnionLabel};
use crate::types::{Annotation, IdlType, PrimitiveType};

impl RustGenerator {
    pub(super) fn generate_union(&self, u: &Union) -> String {
        let mut output = String::new();
        let indent = self.indent();
        let serde_derive = self.serde_derive();
        let serde_rename = self.serde_rename_attr();
        push_fmt(
            &mut output,
            format_args!("{indent}#[derive(Debug, Clone, PartialEq{serde_derive})]\n"),
        );
        if !serde_rename.is_empty() {
            push_fmt(&mut output, format_args!("{indent}{serde_rename}"));
        }
        let name = &u.name;
        push_fmt(&mut output, format_args!("{indent}pub enum {name} {{\n"));

        let mut default_case: Option<(String, String)> = None;

        for (idx, case) in u.cases.iter().enumerate() {
            let variant_name = Self::union_variant_name(&case.field.name, idx);
            let field_type = Self::type_to_rust(&case.field.field_type);
            let idl_type = case.field.field_type.to_idl_string();
            let needs_comment = matches!(
                case.field.field_type,
                IdlType::Array { .. } | IdlType::Sequence { .. } | IdlType::Map { .. }
            );

            if default_case.is_none()
                && (case
                    .labels
                    .iter()
                    .any(|label| matches!(label, UnionLabel::Default))
                    || case
                        .field
                        .annotations
                        .iter()
                        .any(|ann| matches!(ann, Annotation::Default | Annotation::DefaultLiteral)))
            {
                if let Some(expr) = Self::union_case_default_expr(&case.field.field_type) {
                    default_case = Some((variant_name.clone(), expr));
                }
            }

            let field_name = &case.field.name;
            if needs_comment {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{indent}    {variant_name}({field_type}),  // was: {idl_type} {field_name}\n"
                    ),
                );
            } else {
                push_fmt(
                    &mut output,
                    format_args!("{indent}    {variant_name}({field_type}),\n"),
                );
            }
        }

        push_fmt(&mut output, format_args!("{indent}}}\n\n"));

        if let Some((variant, expr)) = default_case {
            push_fmt(
                &mut output,
                format_args!("{indent}impl Default for {name} {{\n"),
            );
            push_fmt(
                &mut output,
                format_args!("{indent}    fn default() -> Self {{\n"),
            );
            push_fmt(
                &mut output,
                format_args!("{indent}        Self::{variant}({expr})\n"),
            );
            push_fmt(&mut output, format_args!("{indent}    }}\n"));
            push_fmt(&mut output, format_args!("{indent}}}\n\n"));
        }

        // Dual emission for unions, same pattern as structs: inherent
        // `encode_xcdrN_le` / `decode_xcdrN_le` methods for each version in
        // `VERSIONS_TO_EMIT`, then a `Cdr2Encode` / `Cdr2Decode` trait
        // delegator pointing at the primary version selected by the
        // `@data_representation` annotation.
        let repr = super::helpers::data_representation_annotation(&u.annotations);
        let primary = super::helpers::primary_version(repr.as_deref());
        output.push_str(&super::key_hash::with_index(|idx| {
            let mut code = super::codec::emit_union_codec(idx, u);
            code.push_str(&super::codec::emit_union_default(idx, u));
            code
        }));
        output.push_str(&Self::emit_cdr_trait_delegator(&u.name, primary));
        output.push_str(&super::key_hash::emit_union_key_methods(u));

        output
    }

    pub(super) fn union_variant_name(field_name: &str, idx: usize) -> String {
        if !field_name.starts_with(|c: char| c.is_alphabetic()) {
            return format!("Case{idx}");
        }
        to_pascal_case(field_name)
    }

    fn union_case_default_expr(idl_type: &IdlType) -> Option<String> {
        match idl_type {
            IdlType::Primitive(primitive) => match primitive {
                PrimitiveType::Void => Some("()".to_string()),
                PrimitiveType::Boolean => Some("false".to_string()),
                PrimitiveType::Char | PrimitiveType::WChar => Some("'\\0'".to_string()),
                PrimitiveType::String | PrimitiveType::WString => Some("String::new()".to_string()),
                PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble => {
                    Some("0.0".to_string())
                }
                PrimitiveType::Fixed { digits, scale } => {
                    Some(format!("Fixed::<{digits}, {scale}>::from_raw(0)"))
                }
                _ => Some("0".to_string()),
            },
            IdlType::Sequence { .. } => Some("Vec::new()".to_string()),
            IdlType::BoundedString { .. } => Some("String::new()".to_string()),
            IdlType::Map { .. } => Some("std::collections::HashMap::new()".to_string()),
            IdlType::Named(name) if name == "String" => Some("String::new()".to_string()),
            _ => None,
        }
    }
}
