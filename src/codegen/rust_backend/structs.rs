// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Struct code generation for Rust.
//!
//! Generates Rust struct definitions from IDL struct types.

use super::super::keywords::rust_ident;
use super::{push_fmt, RustGenerator};
use crate::ast::{Field, Struct};
use crate::types::{Annotation, IdlType, PrimitiveType};

impl RustGenerator {
    pub(super) fn generate_struct(&self, s: &Struct) -> String {
        self.generate_struct_with_module(s, None)
    }

    pub(super) fn generate_struct_with_module(
        &self,
        s: &Struct,
        module_path: Option<&str>,
    ) -> String {
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
        let name = &s.name;
        push_fmt(&mut output, format_args!("{indent}pub struct {name} {{\n"));

        if let Some(base) = &s.base_struct {
            push_fmt(&mut output, format_args!("{indent}    pub base: {base},\n"));
        }

        for field in &s.fields {
            let mut field_type = Self::type_to_rust(&field.field_type);
            if field.is_external() {
                field_type = format!("Box<{field_type}>");
            }
            if field.is_optional() {
                field_type = format!("Option<{field_type}>");
            }
            let idl_type = field.field_type.to_idl_string();
            let needs_comment = matches!(
                field.field_type,
                IdlType::Array { .. } | IdlType::Sequence { .. } | IdlType::Map { .. }
            );

            let field_name = rust_ident(&field.name);
            if needs_comment {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{indent}    pub {field_name}: {field_type},  // was: {idl_type} {}\n",
                        field.name,
                    ),
                );
            } else {
                push_fmt(
                    &mut output,
                    format_args!("{indent}    pub {field_name}: {field_type},\n"),
                );
            }
        }

        push_fmt(&mut output, format_args!("{indent}}}\n\n"));
        output.push_str(&Self::emit_cdr2_encode_impl(s));
        output.push_str(&Self::emit_cdr2_decode_impl(s));
        let is_nested = s
            .annotations
            .iter()
            .any(|a| matches!(a, Annotation::Nested));
        if !is_nested {
            output.push_str(&Self::emit_dds_trait_impl(s, module_path));
        }
        output.push_str(&Self::emit_builder_impl(s));
        output
    }

    /// Generate builder pattern for a struct
    fn emit_builder_impl(s: &Struct) -> String {
        let struct_name = &s.name;
        let builder_name = format!("{struct_name}Builder");

        let mut output = String::new();

        // impl StructName { fn builder() }
        push_fmt(&mut output, format_args!("impl {struct_name} {{\n"));
        push_fmt(
            &mut output,
            format_args!("    /// Create a builder for {struct_name}\n"),
        );
        push_fmt(
            &mut output,
            format_args!("    #[must_use]\n    pub fn builder() -> {builder_name} {{\n"),
        );
        push_fmt(
            &mut output,
            format_args!("        {builder_name}::default()\n    }}\n}}\n\n"),
        );

        // Builder struct with all Optional fields
        push_fmt(
            &mut output,
            format_args!("/// Builder for `{struct_name}`\n"),
        );
        push_fmt(
            &mut output,
            format_args!("#[derive(Default)]\npub struct {builder_name} {{\n"),
        );

        for field in &s.fields {
            let field_type = Self::type_to_rust(&field.field_type);
            let fname = rust_ident(&field.name);
            // All builder fields are Option<T>, even if already optional in struct
            push_fmt(
                &mut output,
                format_args!("    {fname}: Option<{field_type}>,\n"),
            );
        }
        output.push_str("}\n\n");

        // Builder impl with setter methods
        push_fmt(&mut output, format_args!("impl {builder_name} {{\n"));

        for field in &s.fields {
            let field_name = rust_ident(&field.name);

            // Use Into<T> for String types for ergonomics
            let (param_type, conversion) = Self::builder_param_type(&field.field_type);

            push_fmt(
                &mut output,
                format_args!("    /// Set the `{field_name}` field\n"),
            );
            push_fmt(
                &mut output,
                format_args!(
                    "    #[must_use]\n    pub fn {field_name}(mut self, value: {param_type}) -> Self {{\n"
                ),
            );
            push_fmt(
                &mut output,
                format_args!("        self.{field_name} = Some({conversion});\n"),
            );
            output.push_str("        self\n    }\n\n");
        }

        // build() method
        output.push_str(
            "    /// Build the struct, returning an error if required fields are missing\n",
        );
        output.push_str("    pub fn build(self) -> Result<");
        output.push_str(struct_name);
        output.push_str(", &'static str> {\n");
        output.push_str("        Ok(");
        output.push_str(struct_name);
        output.push_str(" {\n");

        for field in &s.fields {
            let field_name = rust_ident(&field.name);
            let unwrap_expr = Self::builder_unwrap_expr(field);
            push_fmt(
                &mut output,
                format_args!("            {field_name}: {unwrap_expr},\n"),
            );
        }

        output.push_str("        })\n    }\n}\n\n");

        output
    }

    /// Get the parameter type and conversion expression for builder setter
    fn builder_param_type(ty: &IdlType) -> (String, String) {
        match ty {
            IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => {
                ("impl Into<String>".to_string(), "value.into()".to_string())
            }
            IdlType::Sequence { inner, .. }
                if matches!(
                    inner.as_ref(),
                    IdlType::Primitive(PrimitiveType::Char | PrimitiveType::WChar)
                ) =>
            {
                // Bounded string (string<N>)
                ("impl Into<String>".to_string(), "value.into()".to_string())
            }
            _ => {
                let rust_type = Self::type_to_rust(ty);
                (rust_type, "value".to_string())
            }
        }
    }

    /// Get the unwrap expression for a field in `build()`
    fn builder_unwrap_expr(field: &Field) -> String {
        let field_name = rust_ident(&field.name);
        let raw_name = &field.name;
        let box_wrap = field.is_external();

        // Check for @default annotation
        if let Some(default_val) = field.get_default() {
            let rust_default = Self::default_to_rust(default_val, &field.field_type);
            let expr = format!("self.{field_name}.unwrap_or({rust_default})");
            return if box_wrap {
                format!("Box::new({expr})")
            } else {
                expr
            };
        }

        // @optional + @external -> Option<Box<T>>
        if field.is_optional() && box_wrap {
            return format!("self.{field_name}.map(Box::new)");
        }

        // @optional fields can be None
        if field.is_optional() {
            return format!("self.{field_name}");
        }

        // @external required field
        if box_wrap {
            return format!("Box::new(self.{field_name}.ok_or(\"{raw_name} is required\")?)");
        }

        // Required field
        format!("self.{field_name}.ok_or(\"{raw_name} is required\")?")
    }

    /// Convert IDL default value to Rust syntax
    fn default_to_rust(value: &str, ty: &IdlType) -> String {
        match ty {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => match value.to_ascii_lowercase().as_str() {
                    "true" => "true".to_string(),
                    "false" => "false".to_string(),
                    _ => value.to_string(),
                },
                PrimitiveType::String | PrimitiveType::WString => {
                    if value.starts_with('"') && value.ends_with('"') {
                        format!("{value}.to_string()")
                    } else {
                        format!("\"{value}\".to_string()")
                    }
                }
                PrimitiveType::Float => format!("{}f32", value.trim_end_matches('f')),
                PrimitiveType::Double | PrimitiveType::LongDouble => {
                    format!("{}f64", value.trim_end_matches('d'))
                }
                _ => value.to_string(),
            },
            _ => value.to_string(),
        }
    }
}
