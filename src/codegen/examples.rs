// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Example code generation for each language backend.
//!
//! Generates a `main()` function demonstrating serialization/deserialization
//! of the first struct found in the IDL.
//!
//! Note: uninlined_format_args allowed here due to extensive format!() usage
//! in code generation that would require significant refactoring.

#![allow(clippy::uninlined_format_args)]
#![allow(clippy::format_push_string)]

use crate::ast::{Definition, IdlFile, Struct};

/// Find the first struct in the AST, returning (`struct`, `module_path`)
fn find_first_struct_with_path(
    defs: &[Definition],
    path: Vec<String>,
) -> Option<(&Struct, Vec<String>)> {
    for def in defs {
        match def {
            Definition::Struct(s) => return Some((s, path)),
            Definition::Module(m) => {
                let mut new_path = path.clone();
                new_path.push(m.name.clone());
                if let Some(result) = find_first_struct_with_path(&m.definitions, new_path) {
                    return Some(result);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the first struct in the AST
fn find_first_struct(defs: &[Definition]) -> Option<&Struct> {
    find_first_struct_with_path(defs, Vec::new()).map(|(s, _)| s)
}

/// Generate example code for Rust
#[must_use]
pub fn generate_rust_example(ast: &IdlFile) -> String {
    let Some(s) = find_first_struct(&ast.definitions) else {
        return String::from("\n// No struct found to generate example\n");
    };

    let name = &s.name;
    let mut field_inits = String::new();
    for f in &s.fields {
        // Use @default annotation value if present, otherwise use type default
        let init = f.get_default().map_or_else(
            || default_value_rust(&f.field_type),
            |v| convert_default_value_rust(v, &f.field_type),
        );
        field_inits.push_str(&format!("        {}: {},\n", f.name, init));
    }

    format!(
        r#"

// ============================================
// Example: CDR2 serialization / deserialization
// ============================================

fn main() {{
    // Create an instance
    let original = {name} {{
{field_inits}    }};
    println!("Original: {{:?}}", original);

    // Encode to CDR2
    let mut buffer = vec![0u8; 4096];
    let encoded_len = original.encode_xcdr2(&mut buffer).expect("encode failed");
    buffer.truncate(encoded_len);
    println!("Encoded ({{}} bytes): {{:02x?}}", buffer.len(), &buffer[..buffer.len().min(32)]);

    // Decode back
    let decoded = {name}::decode_xcdr2(&buffer).expect("decode failed");
    println!("Decoded: {{:?}}", decoded);

    // Verify roundtrip
    assert_eq!(original, decoded, "Roundtrip failed!");
    println!("Roundtrip successful!");
}}
"#,
        name = name,
        field_inits = field_inits
    )
}

/// Convert an IDL @default value to Rust syntax
fn convert_default_value_rust(value: &str, ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => {
                // Handle TRUE/FALSE (IDL) -> true/false (Rust)
                match value.to_ascii_lowercase().as_str() {
                    "true" => "true".to_string(),
                    "false" => "false".to_string(),
                    _ => value.to_string(),
                }
            }
            PrimitiveType::Char | PrimitiveType::WChar => {
                // Ensure proper char literal
                if value.starts_with('\'') {
                    value.to_string()
                } else if value.len() == 1 {
                    format!("'{}'", value)
                } else {
                    format!("'{}'", value.chars().next().unwrap_or('?'))
                }
            }
            PrimitiveType::String | PrimitiveType::WString => {
                // Handle string literals
                if value.starts_with('"') && value.ends_with('"') {
                    format!("{}.to_string()", value)
                } else {
                    format!("\"{}\".to_string()", value)
                }
            }
            PrimitiveType::Float => format!("{}f32", value.trim_end_matches('f')),
            PrimitiveType::Double | PrimitiveType::LongDouble => {
                format!("{}f64", value.trim_end_matches('d'))
            }
            PrimitiveType::Octet | PrimitiveType::UInt8 => format!("{}u8", value),
            PrimitiveType::Int8 => format!("{}i8", value),
            PrimitiveType::Short | PrimitiveType::Int16 => format!("{}i16", value),
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => format!("{}u16", value),
            PrimitiveType::Long | PrimitiveType::Int32 => format!("{}i32", value),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => format!("{}u32", value),
            PrimitiveType::LongLong | PrimitiveType::Int64 => format!("{}i64", value),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => format!("{}u64", value),
            _ => value.to_string(),
        },
        IdlType::Named(_) => {
            // For enums/named types, use qualified name or try to parse as enum variant
            value.to_string()
        }
        _ => value.to_string(),
    }
}

fn default_value_rust(ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::BoundedString { .. } => "String::new()".to_string(),
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => "false".to_string(),
            PrimitiveType::Char | PrimitiveType::WChar => "'A'".to_string(),
            PrimitiveType::Octet | PrimitiveType::UInt8 => "0u8".to_string(),
            PrimitiveType::Int8 => "0i8".to_string(),
            PrimitiveType::Short | PrimitiveType::Int16 => "0i16".to_string(),
            PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => "0u16".to_string(),
            PrimitiveType::Long | PrimitiveType::Int32 => "42i32".to_string(),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => "42u32".to_string(),
            PrimitiveType::LongLong | PrimitiveType::Int64 => "0i64".to_string(),
            PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => "0u64".to_string(),
            PrimitiveType::Float => "3.14f32".to_string(),
            PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159f64".to_string(),
            PrimitiveType::String | PrimitiveType::WString => "\"hello\".to_string()".to_string(),
            PrimitiveType::Fixed { .. } => "0.0".to_string(),
            PrimitiveType::Void => "()".to_string(),
        },
        IdlType::Named(n) => format!("{}::default()", last_ident(n)),
        IdlType::Sequence { .. } => "vec![]".to_string(),
        IdlType::Array { inner, size } => {
            let elem = default_value_rust(inner);
            format!("[{}; {}]", elem, size)
        }
        IdlType::Map { .. } => "Default::default()".to_string(),
    }
}

/// Generate example code for C++
#[must_use]
pub fn generate_cpp_example(ast: &IdlFile) -> String {
    let Some((s, path)) = find_first_struct_with_path(&ast.definitions, Vec::new()) else {
        return String::from("\n// No struct found to generate example\n");
    };

    // Build fully qualified type name with namespace
    let full_name = if path.is_empty() {
        s.name.clone()
    } else {
        format!("{}::{}", path.join("::"), s.name)
    };

    let mut field_inits = String::new();
    for f in &s.fields {
        // Use @default annotation value if present, otherwise use type default
        let init = f.get_default().map_or_else(
            || default_value_cpp(&f.field_type),
            |v| convert_default_value_cpp(v, &f.field_type),
        );
        field_inits.push_str(&format!("    obj.{} = {};\n", f.name, init));
    }

    format!(
        r#"

// ============================================
// Example: CDR2 serialization / deserialization
// ============================================

#include <iostream>
#include <iomanip>
#include <cstring>

int main() {{
    // Create an instance
    {full_name} obj;
{field_inits}
    std::cout << "Original created" << std::endl;

    // Encode to CDR2
    std::uint8_t buffer[4096];
    int encoded_len = obj.encode_cdr2_le(buffer, sizeof(buffer));
    if (encoded_len < 0) {{
        std::cerr << "Encode failed!" << std::endl;
        return 1;
    }}
    std::cout << "Encoded (" << encoded_len << " bytes): ";
    for (int i = 0; i < std::min(encoded_len, 32); ++i) {{
        std::cout << std::hex << std::setw(2) << std::setfill('0') << static_cast<int>(buffer[i]);
    }}
    std::cout << std::endl;

    // Decode back
    {full_name} decoded;
    int decoded_len = decoded.decode_cdr2_le(buffer, static_cast<std::size_t>(encoded_len));
    if (decoded_len < 0) {{
        std::cerr << "Decode failed!" << std::endl;
        return 1;
    }}
    std::cout << "Decoded successfully (" << decoded_len << " bytes read)" << std::endl;

    std::cout << "Roundtrip successful!" << std::endl;
    return 0;
}}
"#,
        full_name = full_name,
        field_inits = field_inits
    )
}

/// Convert an IDL @default value to C++ syntax
fn convert_default_value_cpp(value: &str, ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => match value.to_ascii_lowercase().as_str() {
                "true" => "true".to_string(),
                "false" => "false".to_string(),
                _ => value.to_string(),
            },
            PrimitiveType::Char => {
                if value.starts_with('\'') {
                    value.to_string()
                } else if value.len() == 1 {
                    format!("'{}'", value)
                } else {
                    format!("'{}'", value.chars().next().unwrap_or('?'))
                }
            }
            PrimitiveType::WChar => {
                if value.starts_with("L'") {
                    value.to_string()
                } else if value.starts_with('\'') {
                    format!("L{}", value)
                } else if value.len() == 1 {
                    format!("L'{}'", value)
                } else {
                    format!("L'{}'", value.chars().next().unwrap_or('?'))
                }
            }
            PrimitiveType::String => {
                if value.starts_with('"') {
                    value.to_string()
                } else {
                    format!("\"{}\"", value)
                }
            }
            PrimitiveType::WString => {
                if value.starts_with("L\"") {
                    value.to_string()
                } else if value.starts_with('"') {
                    format!("L{}", value)
                } else {
                    format!("L\"{}\"", value)
                }
            }
            PrimitiveType::Float => format!("{}f", value.trim_end_matches('f')),
            _ => value.to_string(),
        },
        _ => value.to_string(),
    }
}

fn default_value_cpp(ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => "false".to_string(),
            PrimitiveType::Char => "'A'".to_string(),
            PrimitiveType::WChar => "L'A'".to_string(),
            PrimitiveType::Octet
            | PrimitiveType::UInt8
            | PrimitiveType::Int8
            | PrimitiveType::Short
            | PrimitiveType::Int16
            | PrimitiveType::UnsignedShort
            | PrimitiveType::UInt16
            | PrimitiveType::LongLong
            | PrimitiveType::Int64
            | PrimitiveType::UnsignedLongLong
            | PrimitiveType::UInt64 => "0".to_string(),
            PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32 => "42".to_string(),
            PrimitiveType::Float => "3.14f".to_string(),
            PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159".to_string(),
            PrimitiveType::String => "\"hello\"".to_string(),
            PrimitiveType::WString => "L\"hello\"".to_string(),
            PrimitiveType::Fixed { .. } => "0.0".to_string(),
            PrimitiveType::Void => String::new(),
        },
        IdlType::Named(n) => format!("{}()", last_ident(n)),
        IdlType::Sequence { .. }
        | IdlType::Array { .. }
        | IdlType::Map { .. }
        | IdlType::BoundedString { .. } => "{}".to_string(),
    }
}

/// Generate example code for C
#[must_use]
pub fn generate_c_example(ast: &IdlFile) -> String {
    let Some(s) = find_first_struct(&ast.definitions) else {
        return String::from("\n// No struct found to generate example\n");
    };

    let name = &s.name;
    // C generator uses snake_case function names
    let name_lower = to_snake_case(name);
    let mut field_inits = String::new();
    for f in &s.fields {
        let init = default_value_c(&f.field_type, &f.name);
        if !init.is_empty() {
            field_inits.push_str(&format!("    {};\n", init));
        }
    }

    format!(
        r#"

/* ============================================
 * Example: CDR2 serialization / deserialization
 * ============================================ */

#include <stdio.h>
#include <string.h>

int main(void) {{
    /* Create an instance */
    {name} obj;
    memset(&obj, 0, sizeof(obj));
{field_inits}
    printf("Original created\n");

    /* Encode to CDR2 */
    uint8_t buffer[4096];
    int encoded_len = {name_lower}_encode_cdr2_le(&obj, buffer, sizeof(buffer));
    if (encoded_len < 0) {{
        fprintf(stderr, "Encode failed!\n");
        return 1;
    }}
    printf("Encoded (%d bytes): ", encoded_len);
    for (int i = 0; i < (encoded_len < 32 ? encoded_len : 32); ++i) {{
        printf("%02x", buffer[i]);
    }}
    printf("\n");

    /* Decode back */
    {name} decoded;
    memset(&decoded, 0, sizeof(decoded));
    int decoded_len = {name_lower}_decode_cdr2_le(&decoded, buffer, (size_t)encoded_len);
    if (decoded_len < 0) {{
        fprintf(stderr, "Decode failed!\n");
        return 1;
    }}
    printf("Decoded successfully (%d bytes read)\n", decoded_len);

    printf("Roundtrip successful!\n");
    return 0;
}}
"#,
        name = name,
        name_lower = name_lower,
        field_inits = field_inits
    )
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

fn default_value_c(ty: &crate::types::IdlType, field_name: &str) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => format!("obj.{} = false", field_name),
            PrimitiveType::Char | PrimitiveType::WChar => format!("obj.{} = 'A'", field_name),
            PrimitiveType::Long | PrimitiveType::Int32 => format!("obj.{} = 42", field_name),
            PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => {
                format!("obj.{} = 42", field_name)
            }
            PrimitiveType::Float => format!("obj.{} = 3.14f", field_name),
            PrimitiveType::Double | PrimitiveType::LongDouble => {
                format!("obj.{} = 3.14159", field_name)
            }
            PrimitiveType::String => {
                format!(
                    "strncpy(obj.{}, \"hello\", sizeof(obj.{}))",
                    field_name, field_name
                )
            }
            _ => format!("obj.{} = 0", field_name),
        },
        _ => String::new(),
    }
}

/// Generate example code for Python
#[must_use]
pub fn generate_python_example(ast: &IdlFile) -> String {
    let Some(s) = find_first_struct(&ast.definitions) else {
        return String::from("\n# No struct found to generate example\n");
    };

    let name = &s.name;
    let mut field_args = String::new();
    for (i, f) in s.fields.iter().enumerate() {
        if i > 0 {
            field_args.push_str(", ");
        }
        // Use @default annotation value if present, otherwise use type default
        let init = f.get_default().map_or_else(
            || default_value_python(&f.field_type),
            |v| convert_default_value_python(v, &f.field_type),
        );
        field_args.push_str(&format!("{}={}", f.name, init));
    }

    format!(
        r#"

# ============================================
# Example: CDR2 serialization / deserialization
# ============================================

if __name__ == "__main__":
    # Create an instance
    original = {name}({field_args})
    print(f"Original: {{original}}")

    # Encode to CDR2
    encoded = original.encode_cdr2_le()
    print(f"Encoded ({{len(encoded)}} bytes): {{encoded.hex()[:64]}}")

    # Decode back
    decoded, bytes_read = {name}.decode_cdr2_le(encoded)
    print(f"Decoded: {{decoded}} ({{bytes_read}} bytes read)")

    # Verify roundtrip
    assert original == decoded, f"Roundtrip failed! {{original}} != {{decoded}}"
    print("Roundtrip successful!")
"#,
        name = name,
        field_args = field_args
    )
}

/// Convert an IDL @default value to Python syntax
fn convert_default_value_python(value: &str, ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => {
                // Convert IDL TRUE/FALSE to Python True/False
                match value.to_ascii_lowercase().as_str() {
                    "true" => "True".to_string(),
                    "false" => "False".to_string(),
                    _ => value.to_string(),
                }
            }
            PrimitiveType::Char | PrimitiveType::WChar => {
                // Python uses strings for chars
                if value.starts_with('\'') || value.starts_with('"') {
                    value.to_string()
                } else if value.len() == 1 {
                    format!("\"{}\"", value)
                } else {
                    format!("\"{}\"", value.chars().next().unwrap_or('?'))
                }
            }
            PrimitiveType::String | PrimitiveType::WString => {
                if value.starts_with('"') || value.starts_with('\'') {
                    value.to_string()
                } else {
                    format!("\"{}\"", value)
                }
            }
            // Python handles numeric types directly
            _ => value.to_string(),
        },
        _ => value.to_string(),
    }
}

fn default_value_python(ty: &crate::types::IdlType) -> String {
    use crate::types::{IdlType, PrimitiveType};
    match ty {
        IdlType::BoundedString { .. } => "\"\"".to_string(),
        IdlType::Primitive(p) => match p {
            PrimitiveType::Boolean => "False".to_string(),
            PrimitiveType::Char | PrimitiveType::WChar => "\"A\"".to_string(),
            PrimitiveType::Long
            | PrimitiveType::Int32
            | PrimitiveType::UnsignedLong
            | PrimitiveType::UInt32 => "42".to_string(),
            PrimitiveType::Float => "3.14".to_string(),
            PrimitiveType::Double | PrimitiveType::LongDouble => "3.14159".to_string(),
            PrimitiveType::String | PrimitiveType::WString => "\"hello\"".to_string(),
            _ => "0".to_string(),
        },
        IdlType::Named(n) => format!("{}()", last_ident(n)),
        IdlType::Sequence { .. } => "[]".to_string(),
        IdlType::Array { inner, size } => {
            let elem = default_value_python(inner);
            format!("[{}] * {}", elem, size)
        }
        IdlType::Map { .. } => "{}".to_string(),
    }
}

fn last_ident(name: &str) -> String {
    name.rfind("::").map_or_else(
        || {
            name.rfind('.')
                .map_or_else(|| name.to_string(), |pos| name[pos + 1..].to_string())
        },
        |pos| name[pos + 2..].to_string(),
    )
}
