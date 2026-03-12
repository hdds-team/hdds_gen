// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Example: `@key` field code generation
//!
//! Shows how hddsgen generates `compute_key()` for various field types:
//! - Numeric primitives  - `to_le_bytes()`
//! - bool                - cast to u8 first
//! - char                - cast to u32 (Unicode scalar)
//! - string / typedef'd string - CDR2 encode
//! - enum                - cast to u32
//! - nested struct       - CDR2 encode
//! - `Fixed<D,S>`        - CDR2 encode
//!
//! Run with: `cargo run --example key_fields`

use hddsgen::{Backend, Parser};

#[allow(clippy::expect_used)]
fn main() {
    let cases: &[(&str, &str)] = &[
        (
            "Numeric primitives (int, float)",
            r"
                @extensibility(APPENDABLE)
                struct SensorReading {
                    @key uint32 sensor_id;
                    @key float value;
                    string label;
                };
            ",
        ),
        (
            "bool and char keys",
            r"
                @extensibility(APPENDABLE)
                struct Toggle {
                    @key boolean enabled;
                    @key char channel;
                };
            ",
        ),
        (
            "typedef string key",
            r"
                typedef string topic_name;
                @extensibility(APPENDABLE)
                struct Message {
                    @key topic_name topic;
                    uint64 seq;
                };
            ",
        ),
        (
            "enum key",
            r"
                enum Priority { LOW, NORMAL, HIGH, CRITICAL };
                @extensibility(APPENDABLE)
                struct Task {
                    @key uint32 task_id;
                    @key Priority priority;
                    string payload;
                };
            ",
        ),
        (
            "nested struct key",
            r"
                struct DeviceId {
                    uint32 vendor;
                    uint32 product;
                };
                @extensibility(APPENDABLE)
                struct DeviceStatus {
                    @key DeviceId device;
                    boolean online;
                };
            ",
        ),
        (
            "fixed-point key",
            r"
                @extensibility(APPENDABLE)
                struct Order {
                    @key uint64 order_id;
                    @key fixed<10,2> price;
                };
            ",
        ),
    ];

    let gen = Backend::Rust.generator();

    for (label, idl) in cases {
        println!("--- {label} ---");

        let mut parser = Parser::try_new(idl).expect("lexer error");
        let ast = parser.parse().expect("parse error");
        let code = gen.generate(&ast).expect("codegen error");

        // Extract and print the last compute_key() method (the top-level struct, not nested ones)
        if let Some(start) = code.rfind("fn compute_key(") {
            let snippet = &code[start..];
            let end = find_method_end(snippet);
            println!("{}", &snippet[..end].trim_end());
        } else {
            println!("(no compute_key generated -- no @key fields)");
        }

        println!();
    }
}

/// Find the closing brace of the first method in the snippet.
/// Counts brace depth to handle nested blocks correctly.
fn find_method_end(s: &str) -> usize {
    let mut depth: i32 = 0;
    let mut in_method = false;
    for (i, c) in s.char_indices() {
        match c {
            '{' => {
                depth += 1;
                in_method = true;
            }
            '}' => {
                depth -= 1;
                if in_method && depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    s.len()
}
