// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Struct code generation for C++.
//!
//! Generates C++ struct definitions with member variables and codecs.

use super::super::keywords::cpp_ident;
use super::codec;
use super::helpers::{cpp_default_init, push_fmt, type_to_cpp};
use super::index::DefinitionIndex;
use super::CppGenerator;
use crate::ast::Struct;
use crate::types::IdlType;

#[allow(clippy::too_many_lines)] // Full struct + codec generation
pub(super) fn generate_struct(
    generator: &CppGenerator,
    s: &Struct,
    idx: &DefinitionIndex,
) -> String {
    let mut output = String::new();
    let indent = generator.indent();
    let member_indent = format!("{indent}    ");

    let name = &s.name;
    if let Some(base) = &s.base_struct {
        push_fmt(
            &mut output,
            format_args!("{indent}struct {name} : public {base} {{\n"),
        );
    } else {
        push_fmt(&mut output, format_args!("{indent}struct {name} {{\n"));
    }

    if generator.fastdds_compat {
        // FastDDS-compatible mode: private members with m_ prefix, public getter/setter methods
        push_fmt(&mut output, format_args!("{indent}public:\n"));

        // Generate getter/setter methods
        for field in &s.fields {
            let base_type = type_to_cpp(&field.field_type);
            let field_type = if field.is_optional() {
                format!("std::optional<{base_type}>")
            } else {
                base_type
            };
            let is_primitive =
                matches!(field.field_type, IdlType::Primitive(_)) && !field.is_optional();

            let fname = cpp_ident(&field.name);
            // Getter - return by value for primitives, const ref for complex types
            if is_primitive {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{member_indent}{field_type} {fname}() const {{ return m_{fname}; }}\n"
                    ),
                );
            } else {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{member_indent}const {field_type}& {fname}() const {{ return m_{fname}; }}\n"
                    ),
                );
                // Non-const getter for complex types
                push_fmt(
                    &mut output,
                    format_args!(
                        "{member_indent}{field_type}& {fname}() {{ return m_{fname}; }}\n"
                    ),
                );
            }

            // Setter
            if is_primitive {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{member_indent}void {fname}({field_type} val) {{ m_{fname} = val; }}\n"
                    ),
                );
            } else {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{member_indent}void {fname}(const {field_type}& val) {{ m_{fname} = val; }}\n"
                    ),
                );
            }
            output.push('\n');
        }

        output.push('\n');

        // Generate CDR2 encode/decode methods (uses m_ prefixed fields)
        output.push_str(&codec::generate_struct_codec_fastdds(
            s,
            idx,
            &member_indent,
        ));

        // Private section with actual member storage
        push_fmt(&mut output, format_args!("\n{indent}private:\n"));
        for field in &s.fields {
            let base_type = type_to_cpp(&field.field_type);
            let field_type = if field.is_optional() {
                format!("std::optional<{base_type}>")
            } else {
                base_type
            };
            let fname = cpp_ident(&field.name);
            push_fmt(
                &mut output,
                format_args!("{member_indent}{field_type} m_{fname};\n"),
            );
        }
    } else {
        // Standard mode: public member fields (direct access) with default initializers
        for field in &s.fields {
            let base_type = type_to_cpp(&field.field_type);
            let field_type = if field.is_optional() {
                format!("std::optional<{base_type}>")
            } else {
                base_type
            };
            let idl_type = field.field_type.to_idl_string();
            let fname = cpp_ident(&field.name);
            let default_init = if field.is_optional() {
                ""
            } else {
                cpp_default_init(&field.field_type)
            };
            let needs_comment = matches!(
                field.field_type,
                IdlType::Array { .. } | IdlType::Sequence { .. } | IdlType::Map { .. }
            );

            if needs_comment {
                push_fmt(
                    &mut output,
                    format_args!(
                        "{member_indent}{field_type} {fname}{default_init};  // was: {idl_type} {}\n",
                        field.name
                    ),
                );
            } else {
                push_fmt(
                    &mut output,
                    format_args!("{member_indent}{field_type} {fname}{default_init};\n"),
                );
            }
        }

        // Generate default + convenience constructors
        if !s.fields.is_empty() {
            output.push('\n');
            // Default constructor (needed because custom ctor suppresses it)
            push_fmt(
                &mut output,
                format_args!("{member_indent}{name}() = default;\n"),
            );
            // Convenience constructor with all fields
            let params: Vec<String> = s
                .fields
                .iter()
                .map(|f| {
                    let base_type = type_to_cpp(&f.field_type);
                    let ft = if f.is_optional() {
                        format!("std::optional<{base_type}>")
                    } else {
                        base_type
                    };
                    let fname = cpp_ident(&f.name);
                    format!("{ft} {fname}")
                })
                .collect();
            let init_list: Vec<String> = s
                .fields
                .iter()
                .map(|f| {
                    let fname = cpp_ident(&f.name);
                    format!("{fname}(std::move({fname}))")
                })
                .collect();

            push_fmt(
                &mut output,
                format_args!(
                    "{member_indent}{name}({params})\n{member_indent}    : {init_list} {{}}\n",
                    params = params.join(", "),
                    init_list = init_list.join(", "),
                ),
            );
        }

        output.push('\n');

        // Generate CDR2 encode/decode methods
        output.push_str(&codec::generate_struct_codec(s, idx, &member_indent));
    }

    push_fmt(&mut output, format_args!("{indent}}};\n\n"));

    // Generate PubSubType class for the struct
    output.push_str(&codec::pubsub_types::generate_pubsub_type_header(s));

    output
}
