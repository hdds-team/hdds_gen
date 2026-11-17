// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! `TypeDescriptor` generation for C
//!
//! Generates metadata structures for DDS types, including:
//! - Type name
//! - Type kind (struct, enum, etc.)
//! - Field descriptions (name, type, `@key` status)
//! - Type hash computation

use super::super::key_holder::TypeIndex;
use super::super::keywords::c_ident;
use crate::ast::Struct;
use crate::types::{Annotation, IdlType, PrimitiveType};
use std::fmt::Write;

/// Convert type name to lowercase `snake_case` for C function names
fn to_snake_case(name: &str) -> String {
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
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

/// Generate `TypeDescriptor` struct and functions for a struct
#[must_use]
pub fn generate_type_descriptor(s: &Struct) -> String {
    let mut file = crate::ast::IdlFile::new();
    file.add_definition(crate::ast::Definition::Struct(s.clone()));
    generate_type_descriptor_in(s, &TypeIndex::new(&file))
}

/// [`generate_type_descriptor`] with the type index of the whole file.
///
/// The `KeyHolder` of a key member of a named type needs that index. Alone,
/// a struct only knows its own members: a named key type is then refused
/// and its `compute_key()` returns -2.
#[must_use]
pub fn generate_type_descriptor_in(s: &Struct, keys: &TypeIndex) -> String {
    generate_type_descriptor_scoped(s, keys, &std::collections::HashMap::new())
}

/// [`generate_type_descriptor_in`] for a module-scoped AST.
///
/// `idl_names` maps the C name of each type (`codegen::c::scoping`) to its
/// IDL qualified name (`a::inner::T`): the name the descriptor reports and
/// hashes, for the structure and for its member types.
#[must_use]
pub(crate) fn generate_type_descriptor_scoped(
    s: &Struct,
    keys: &TypeIndex,
    idl_names: &std::collections::HashMap<String, String>,
) -> String {
    let type_name = idl_names.get(&s.name).unwrap_or(&s.name);
    // Function prefix: each scope segment in snake case (`a::MyType`:
    // `a_my_type`).
    let snake_name = type_name
        .split("::")
        .map(to_snake_case)
        .collect::<Vec<_>>()
        .join("_");

    // Collect @key fields
    let key_field_count = s
        .fields
        .iter()
        .filter(|f| f.annotations.iter().any(|a| matches!(a, Annotation::Key)))
        .count();

    let mut out = String::new();

    // Header comment
    let _ = writeln!(out, "/* TypeDescriptor for {type_name} */\n");

    // Field descriptor struct (only if not already defined)
    out.push_str("#ifndef CDR_FIELD_DESCRIPTOR_DEFINED\n");
    out.push_str("#define CDR_FIELD_DESCRIPTOR_DEFINED\n");
    out.push_str("typedef struct cdr_field_descriptor {\n");
    out.push_str("    const char* name;      /* Field name */\n");
    out.push_str("    const char* type_name; /* Field type as string */\n");
    out.push_str("    uint8_t is_key;        /* 1 if @key annotated */\n");
    out.push_str("    uint8_t type_kind;     /* Type kind enum */\n");
    out.push_str("    uint32_t offset;       /* Offset in struct (for simple types) */\n");
    out.push_str("} cdr_field_descriptor_t;\n");
    out.push_str("#endif /* CDR_FIELD_DESCRIPTOR_DEFINED */\n\n");

    // Type kind enum (only if not already defined)
    out.push_str("#ifndef CDR_TYPE_KIND_DEFINED\n");
    out.push_str("#define CDR_TYPE_KIND_DEFINED\n");
    out.push_str("typedef enum cdr_type_kind {\n");
    out.push_str("    CDR_TYPE_PRIMITIVE = 0,\n");
    out.push_str("    CDR_TYPE_STRING = 1,\n");
    out.push_str("    CDR_TYPE_SEQUENCE = 2,\n");
    out.push_str("    CDR_TYPE_ARRAY = 3,\n");
    out.push_str("    CDR_TYPE_STRUCT = 4,\n");
    out.push_str("    CDR_TYPE_ENUM = 5,\n");
    out.push_str("    CDR_TYPE_UNION = 6,\n");
    out.push_str("    CDR_TYPE_MAP = 7\n");
    out.push_str("} cdr_type_kind_t;\n");
    out.push_str("#endif /* CDR_TYPE_KIND_DEFINED */\n\n");

    // TypeDescriptor struct (only if not already defined)
    out.push_str("#ifndef CDR_TYPE_DESCRIPTOR_DEFINED\n");
    out.push_str("#define CDR_TYPE_DESCRIPTOR_DEFINED\n");
    out.push_str("typedef struct cdr_type_descriptor {\n");
    out.push_str("    const char* name;                  /* Type name */\n");
    out.push_str("    cdr_type_kind_t kind;              /* Type kind */\n");
    out.push_str("    uint32_t field_count;              /* Number of fields */\n");
    out.push_str("    uint32_t key_field_count;          /* Number of @key fields */\n");
    out.push_str("    const cdr_field_descriptor_t* fields; /* Field descriptors */\n");
    out.push_str("    uint8_t type_hash[16];             /* Type identifier hash */\n");
    out.push_str("} cdr_type_descriptor_t;\n");
    out.push_str("#endif /* CDR_TYPE_DESCRIPTOR_DEFINED */\n\n");

    // Field descriptors array
    let _ = writeln!(
        out,
        "static const cdr_field_descriptor_t {snake_name}_fields[] = {{"
    );
    for field in &s.fields {
        let is_key = i32::from(
            field
                .annotations
                .iter()
                .any(|a| matches!(a, Annotation::Key)),
        );
        let type_kind = get_type_kind(&field.field_type);
        let type_name_str = super::scoping::idl_string(&field.field_type, idl_names);
        let _ = writeln!(
            out,
            "    {{ \"{}\", \"{}\", {}, {}, 0 }},",
            field.name, type_name_str, is_key, type_kind
        );
    }
    if s.fields.is_empty() {
        // An empty initializer list is not ISO C.
        out.push_str("    { NULL, NULL, 0, 0, 0 }\n");
    }
    out.push_str("};\n\n");

    // Type hash computation (FNV-1a based on type structure)
    let type_hash = compute_type_hash(s, type_name, idl_names);
    let _ = writeln!(
        out,
        "/* Type hash for {} (computed from type structure) */",
        type_name
    );
    let _ = write!(
        out,
        "static const uint8_t {snake_name}_type_hash[16] = {{\n    "
    );
    for (i, byte) in type_hash.iter().enumerate() {
        if i > 0 && i % 8 == 0 {
            out.push_str("\n    ");
        }
        let _ = write!(out, "0x{:02x}", byte);
        if i < 15 {
            out.push_str(", ");
        }
    }
    out.push_str("\n};\n\n");

    // TypeDescriptor instance
    let _ = writeln!(
        out,
        "static const cdr_type_descriptor_t {snake_name}_type_descriptor = {{"
    );
    let _ = writeln!(out, "    \"{type_name}\",");
    out.push_str("    CDR_TYPE_STRUCT,\n");
    let _ = writeln!(out, "    {},", s.fields.len());
    let _ = writeln!(out, "    {key_field_count},");
    let _ = writeln!(out, "    {snake_name}_fields,");
    out.push_str("    { 0 } /* type_hash: see the _type_hash array */\n");
    out.push_str("};\n\n");

    // Getter function
    let _ = writeln!(
        out,
        "static inline const cdr_type_descriptor_t* {snake_name}_get_type_descriptor(void) {{"
    );
    let _ = writeln!(out, "    return &{snake_name}_type_descriptor;");
    out.push_str("}\n\n");

    // Type name getter
    let _ = writeln!(
        out,
        "static inline const char* {snake_name}_type_name(void) {{"
    );
    let _ = writeln!(out, "    return \"{type_name}\";");
    out.push_str("}\n\n");

    // Has key function (a key member of a base type counts)
    let _ = writeln!(out, "static inline int {snake_name}_has_key(void) {{");
    let _ = writeln!(out, "    return {};", i32::from(keys.is_keyed(s)));
    out.push_str("}\n\n");

    out.push_str(&compute_key_function(s, keys, &snake_name));
    if keys.is_keyed(s) {
        out.push_str(&writer_operations(s, &snake_name));
    }
    out
}

/// The C types of this backend: a base type is the `base` member, bounded
/// strings are `char[N+1]` / `wchar_t[N+1]`, sequences are `{ data, len }`
/// structs, a union keeps its discriminator in `_d` and its member in `_u`.
struct CTypes<'a> {
    idx: &'a TypeIndex,
}

impl super::key_hash::CFlavor for CTypes<'_> {
    fn member(&self, parent: &str, base_depth: usize, name: &str) -> Option<String> {
        let mut place = parent.to_string();
        for _ in 0..base_depth {
            place.push_str(".base");
        }
        Some(format!("{place}.{}", c_ident(name)))
    }

    fn string(&self, place: &str, bounded: bool, wide: bool) -> Option<String> {
        Some(match (bounded, wide) {
            (false, false) => format!("hdds_kh_string(w, {place});"),
            (false, true) => format!("hdds_kh_wstring(w, {place});"),
            (true, false) => format!(
                "hdds_kh_string_n(w, {place}, hdds_kh_strnlen({place}, sizeof({place})));"
            ),
            (true, true) => format!(
                "hdds_kh_wstring_n(w, {place}, hdds_kh_wcsnlen({place}, sizeof({place}) / sizeof(({place})[0])));"
            ),
        })
    }

    fn sequence(&self, place: &str, _bounded: bool, i: &str) -> (String, String) {
        (format!("{place}.len"), format!("{place}.data[{i}]"))
    }

    fn union_disc(&self, place: &str) -> String {
        format!("{place}._d")
    }

    fn union_member(&self, place: &str, _union_name: &str, name: &str) -> String {
        format!("{place}._u.{}", c_ident(name))
    }

    fn label(&self, disc: &IdlType, label: &str) -> String {
        super::xcdr2::label_value(self.idx, disc, label)
            .map_or_else(|| label.to_string(), |v| v.to_string())
    }
}

/// `<snake>_compute_key()`: the instance `KeyHash` of RTPS 2.5 9.6.4.8.
fn compute_key_function(s: &Struct, keys: &TypeIndex, snake_name: &str) -> String {
    compute_key_mode(s, keys, snake_name, false) + &compute_key_mode(s, keys, snake_name, true)
}

fn compute_key_mode(s: &Struct, keys: &TypeIndex, snake_name: &str, protected: bool) -> String {
    let function = if protected {
        "compute_protected_key"
    } else {
        "compute_key"
    };
    let type_name = &s.name;
    let mut out = String::new();
    if protected {
        out.push_str("/* Protected KeyHash (DDS-Security 1.1 7.3.4): always MD5 of CDR big-endian\n * key fields, with natural alignment. Return codes are those of compute_key. */\n");
    } else {
        let _ = writeln!(
            out,
            "/* Instance KeyHash of {type_name} (RTPS 2.5 9.6.4.8): the KeyHolder\n \
         * serialized as PLAIN_CDR2 big endian, zero-padded to 16 bytes when its\n \
         * maximum size is at most 16, its MD5 otherwise. Returns 1 (keyed), 0 (no\n \
         * key: all zeros), -1 (NULL argument), -2 (a key hddsgen's KeyHash model\n \
         * does not cover: all zeros). */"
        );
    }
    let _ = writeln!(
        out,
        "static inline int {snake_name}_{function}(const {type_name}* value, uint8_t key_hash[16]) {{"
    );
    let holder = keys.key_holder(s);
    let emitted = match &holder {
        Ok(Some(kh)) => {
            let flavor = CTypes { idx: keys };
            let mut e = super::key_hash::Emit::new(&flavor);
            e.structure(1, kh, "(*value)").map(|()| {
                (
                    e.declarations(),
                    e.statements().to_string(),
                    kh.is_padded_form(),
                )
            })
        }
        Ok(None) => Err(String::new()),
        Err(reason) => Err(reason.clone()),
    };
    match (&holder, emitted) {
        (_, Ok((decls, body, padded))) => {
            out.push_str("    hdds_key_holder kh;\n    hdds_key_holder* w = &kh;\n");
            if !decls.is_empty() {
                let _ = writeln!(out, "    {decls}");
            }
            out.push_str("    if (!value || !key_hash) return -1;\n    hdds_kh_init(w);\n");
            if protected {
                out.push_str("    w->max_align = 8u;\n");
            }
            out.push_str(&body);
            let _ = writeln!(
                out,
                "    hdds_kh_finish(w, {}, key_hash);",
                i32::from(padded && !protected)
            );
            out.push_str("    return 1;\n");
        }
        (Ok(None), Err(_)) => {
            out.push_str("    if (!value || !key_hash) return -1;\n");
            out.push_str("    memset(key_hash, 0, 16);\n    return 0;\n");
        }
        (_, Err(reason)) => {
            let reason = reason.replace(['\n', '\r'], " ").replace("*/", "* /");
            let _ = writeln!(
                out,
                "    /* No KeyHolder generated: the key of this type is not covered by\n     \
                 * hddsgen's KeyHash model ({reason}). */"
            );
            out.push_str("    if (!value || !key_hash) return -1;\n");
            out.push_str("    memset(key_hash, 0, 16);\n    return -2;\n");
        }
    }
    out.push_str("}\n\n");
    out
}

/// Get the type kind enum value as a string
const fn get_type_kind(t: &IdlType) -> &'static str {
    match t {
        IdlType::BoundedString { .. }
        | IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString) => "CDR_TYPE_STRING",
        IdlType::Primitive(_) => "CDR_TYPE_PRIMITIVE",
        IdlType::Sequence { .. } => "CDR_TYPE_SEQUENCE",
        IdlType::Array { .. } => "CDR_TYPE_ARRAY",
        IdlType::Map { .. } => "CDR_TYPE_MAP",
        IdlType::Named(_) => "CDR_TYPE_STRUCT", // Could be struct or enum
    }
}

/// Compute a type hash based on type structure (FNV-1a)
fn compute_type_hash(
    s: &Struct,
    type_name: &str,
    idl_names: &std::collections::HashMap<String, String>,
) -> [u8; 16] {
    let mut hash: u64 = 14_695_981_039_346_656_037;

    // Hash the type name
    for byte in type_name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }

    // Hash each field
    for field in &s.fields {
        // Field name
        for byte in field.name.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
        // Field type
        for byte in super::scoping::idl_string(&field.field_type, idl_names).as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(1_099_511_628_211);
        }
        // @key annotation
        let is_key = field
            .annotations
            .iter()
            .any(|a| matches!(a, Annotation::Key));
        hash ^= u64::from(is_key);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }

    // Expand to 16 bytes
    let mut result = [0u8; 16];
    result[0..8].copy_from_slice(&hash.to_le_bytes());
    hash = hash.wrapping_mul(1_099_511_628_211);
    result[8..16].copy_from_slice(&hash.to_le_bytes());
    result
}

/// Typed operations use the codec's key payload and both specified hashes.
fn writer_operations(s: &Struct, hash_prefix: &str) -> String {
    let ty = &s.name;
    let f = super::helpers::c_name(ty);
    let mut out = format!(
        r"#ifdef HDDS_H
#include <stdlib.h>
/* Include hdds.h before this header to enable the typed writer operations. */
static inline HddsError {f}__instance_operation(struct HddsDataWriter* writer, const {ty}* value,
                                               unsigned operation, uint64_t timestamp, HddsInstanceHandle* handle) {{
    size_t capacity;
    uint8_t *data, *key_bytes;
    int len, key_len;
    HddsError result;
    HddsSerializedKey key;
    if (!writer || !value) return HDDS_INVALID_ARGUMENT;
    capacity = operation <= 1 ? {f}_max_cdr2_size(value) : {f}_max_key_cdr2_size(value);
    if (capacity == 0) capacity = 1;
    data = operation <= 1 ? (uint8_t*)malloc(capacity) : NULL;
    key_bytes = (uint8_t*)malloc(capacity);
    if ((operation <= 1 && !data) || !key_bytes) {{ free(data); free(key_bytes); return HDDS_OUT_OF_MEMORY; }}
    len = operation <= 1 ? {f}_encode_cdr2_le(value, data, capacity) : 0;
    key_len = {f}_encode_key_cdr2_le(value, key_bytes, capacity);
    if (len < 0 || key_len < 0 || {hash_prefix}_compute_key(value, key.HASH) != 1 ||
        {hash_prefix}_compute_protected_key(value, key.PROTECTED_HASH) != 1) {{
        free(data); free(key_bytes); return HDDS_INVALID_ARGUMENT;
    }}
    key.PAYLOAD = key_bytes; key.PAYLOAD_LEN = (size_t)key_len; key.REPRESENTATION = 2;
    switch (operation) {{
"
    );
    for (i, op) in [
        "write",
        "write_w_timestamp",
        "dispose",
        "dispose_w_timestamp",
        "unregister_instance",
        "unregister_instance_w_timestamp",
        "register_instance",
        "register_instance_w_timestamp",
    ]
    .iter()
    .enumerate()
    {
        let ts = if op.ends_with("_w_timestamp") {
            ", timestamp"
        } else {
            ""
        };
        let handle = if op.starts_with("register_instance") {
            ", handle"
        } else {
            ""
        };
        let _ = writeln!(out, "    case {i}: result = hdds_writer_{op}_keyed(writer, data, (size_t)len, &key{ts}{handle}); break;");
    }
    out.push_str("    default: result = HDDS_INVALID_ARGUMENT; break;\n    }\n    free(data); free(key_bytes); return result;\n}\n");
    for (i, op) in [
        "write",
        "write_w_timestamp",
        "dispose",
        "dispose_w_timestamp",
        "unregister_instance",
        "unregister_instance_w_timestamp",
        "register_instance",
        "register_instance_w_timestamp",
    ]
    .iter()
    .enumerate()
    {
        let ts = op.ends_with("_w_timestamp");
        let reg = op.starts_with("register_instance");
        let ts_param = if ts { ", uint64_t timestamp" } else { "" };
        let handle_param = if reg {
            ", HddsInstanceHandle* handle"
        } else {
            ""
        };
        let timestamp = if ts { "timestamp" } else { "0" };
        let handle = if reg { "handle" } else { "NULL" };
        let _ = writeln!(out, "static inline HddsError {f}_{op}(struct HddsDataWriter* writer, const {ty}* value{ts_param}{handle_param}) {{\n    return {f}__instance_operation(writer, value, {i}, {timestamp}, {handle});\n}}");
    }
    out.push_str("#endif\n\n");
    out
}

/// Union instance operations use its keyed discriminator, not its selected case.
pub(super) fn union_key_operations(u: &crate::ast::Union, keys: &TypeIndex) -> String {
    if !u.discriminator_is_key() {
        return String::new();
    }
    let holder = crate::codegen::data_model::discriminator_holder(u, "_d");
    let prefix = super::helpers::c_name(&u.name);
    compute_key_function(&holder, keys, &prefix) + &writer_operations(&holder, &prefix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Field;

    #[test]
    fn test_type_descriptor_generation() {
        let s = Struct {
            name: "TestMessage".to_string(),
            fields: vec![
                Field {
                    name: "id".to_string(),
                    field_type: IdlType::Primitive(PrimitiveType::UInt32),
                    annotations: vec![Annotation::Key],
                },
                Field {
                    name: "data".to_string(),
                    field_type: IdlType::Primitive(PrimitiveType::String),
                    annotations: vec![],
                },
            ],
            annotations: vec![],
            base_struct: None,
            extensibility: None,
        };

        let output = generate_type_descriptor(&s);
        assert!(output.contains("cdr_type_descriptor_t"));
        assert!(output.contains("test_message_fields"));
        assert!(output.contains("test_message_compute_key"));
    }

    #[test]
    fn test_snake_case() {
        assert_eq!(to_snake_case("HelloWorld"), "hello_world");
        assert_eq!(to_snake_case("XMLParser"), "x_m_l_parser");
        assert_eq!(to_snake_case("simple"), "simple");
    }
}
