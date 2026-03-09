// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Basic usage example of the IDL parser
//!
//! Demonstrates parsing a simple IDL file and inspecting the AST.

use hddsgen::{Definition, Parser};
use std::fs;

#[allow(clippy::expect_used)]
fn main() {
    println!("=== IDL Parser Example ===\n");

    // Example 1: Parse inline IDL
    let inline_idl = r"
        module Example {
            struct Point {
                @key int32_t x;
                @key int32_t y;
            };
        };
    ";

    println!("Parsing inline IDL...");
    let mut parser = Parser::try_new(inline_idl).expect("Lexer error");
    match parser.parse() {
        Ok(ast) => {
            println!(
                "[OK] Successfully parsed {} definitions",
                ast.definitions.len()
            );

            for def in &ast.definitions {
                print_definition(def, 0);
            }
        }
        Err(e) => {
            eprintln!("[ERROR] Parse error: {e}");
        }
    }

    println!("\n{}", "=".repeat(50));

    // Example 2: Parse from file
    println!("\nParsing from file (examples/sample.idl)...");
    if let Ok(content) = fs::read_to_string("examples/sample.idl") {
        let mut parser = Parser::try_new(&content).expect("Lexer error");
        match parser.parse() {
            Ok(ast) => {
                println!(
                    "[OK] Successfully parsed {} definitions",
                    ast.definitions.len()
                );

                for def in &ast.definitions {
                    print_definition(def, 0);
                }
            }
            Err(e) => {
                eprintln!("[ERROR] Parse error: {e}");
            }
        }
    } else {
        println!("Note: examples/sample.idl not found");
    }
}

/// Pretty-print a definition with indentation
fn print_definition(def: &Definition, indent: usize) {
    let indent_str = "  ".repeat(indent);

    match def {
        Definition::Module(m) => {
            println!("{}Module: {}", indent_str, m.name);
            for inner_def in &m.definitions {
                print_definition(inner_def, indent + 1);
            }
        }
        Definition::AnnotationDecl(ad) => {
            println!(
                "{}@annotation {} ({} member(s))",
                indent_str,
                ad.name,
                ad.members.len()
            );
        }
        Definition::Struct(s) => {
            println!("{}Struct: {}", indent_str, s.name);

            let key_fields = s.key_fields();
            if !key_fields.is_empty() {
                println!(
                    "{}  Key fields: {}",
                    indent_str,
                    key_fields
                        .iter()
                        .map(|f| f.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }

            println!("{indent_str}  Fields:");
            for field in &s.fields {
                let key_marker = if field.is_key() { " [@key]" } else { "" };
                let optional_marker = if field.is_optional() {
                    " [@optional]"
                } else {
                    ""
                };
                println!(
                    "{}    - {}: {:?}{}{}",
                    indent_str, field.name, field.field_type, key_marker, optional_marker
                );
            }
        }
        Definition::Typedef(t) => {
            println!("{}Typedef: {} = {:?}", indent_str, t.name, t.base_type);
        }
        Definition::Enum(e) => {
            println!("{}Enum: {}", indent_str, e.name);
            for variant in &e.variants {
                if let Some(val) = variant.value {
                    println!("{}  - {} = {}", indent_str, variant.name, val);
                } else {
                    println!("{}  - {}", indent_str, variant.name);
                }
            }
        }
        Definition::Union(u) => {
            println!(
                "{}Union: {} (discriminator: {:?})",
                indent_str, u.name, u.discriminator
            );
        }
        Definition::Const(c) => {
            println!(
                "{}Const: {} ({:?}) = {}",
                indent_str, c.name, c.const_type, c.value
            );
        }
        Definition::Bitset(b) => {
            println!("{}Bitset: {}", indent_str, b.name);
            for f in &b.fields {
                println!("{}  - bitfield<{}> {}", indent_str, f.width, f.name);
            }
        }
        Definition::Bitmask(m) => {
            println!("{}Bitmask: {}", indent_str, m.name);
            for flag in &m.flags {
                println!("{}  - {}", indent_str, flag.name);
            }
        }
        Definition::ForwardDecl(fd) => {
            let kind = match fd.kind {
                hddsgen::ForwardKind::Struct => "struct",
                hddsgen::ForwardKind::Union => "union",
            };
            println!("{}Forward decl: {} {};", indent_str, kind, fd.name);
        }
        // interfaces variants (only when feature enabled)
        #[cfg(feature = "interfaces")]
        Definition::Interface(i) => {
            println!("{}Interface: {}", indent_str, i.name);
        }
        #[cfg(feature = "interfaces")]
        Definition::Exception(e) => {
            println!("{}Exception: {}", indent_str, e.name);
        }
    }
}
