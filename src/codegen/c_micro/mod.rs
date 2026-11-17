// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! C Micro code generator for embedded MCUs
//!
//! Generates header-only C99 code (no allocation) for the `hdds_micro_cdr.h`
//! runtime of `sdk/c-micro/include` (its `hdds_cdr_t` cursor and error
//! codes). Target: STM32, AVR, PIC, ESP32, any MCU with a C compiler.
//!
//! The data path is XCDR2 little endian (DDS-`XTypes` 1.3 7.4.3), through
//! the codec shared with the C backend (`codegen::c::xcdr2`): FINAL,
//! APPENDABLE (DHEADER) and MUTABLE (DHEADER, `EMHEADER1`) structures,
//! optional members, unions, enumerations and bitmasks at their
//! `@bit_bound` width, 8-byte primitives aligned at 4 from the start of the
//! buffer. The codec emits its own cursor functions (`hdds_x2m_*`) and uses
//! only the SDK's cursor type, `hdds_cdr_init` and error codes; the SDK's
//! hand-coding helpers follow the same XCDR2 rules.
//!
//! # Supported IDL Constructs
//!
//! | Construct | Holder | Notes |
//! |-----------|--------|-------|
//! | `struct` | `struct` | Base members in a `base` member; `@optional` adds `has_<m>` |
//! | `enum` | `enum` | Serialized at its `@bit_bound` width (1, 2 or 4 bytes) |
//! | `bitmask` | `uint8_t` .. `uint64_t` | `@bit_bound` width |
//! | `union` | `struct { _d; union _u; }` | FINAL, APPENDABLE or MUTABLE |
//! | `typedef` | `typedef` | |
//! | `module` | name prefix | The full module path: `a_inner_T`; references resolved by IDL scoping |
//! | `sequence<T, N>` | `{ T data[N]; uint32_t count; }` | Unbounded: `max_sequence_len`; `sequence<char>` too (a sequence, not a string) |
//! | `string<N>` | `char[N+1]` | Unbounded: `char[max_string_len]` |
//! | `wstring<N>` | `uint16_t[N+1]` (UTF-16) | Unbounded: `uint16_t[max_string_len]` |
//! | `wchar` | `uint16_t` (UTF-16 unit) | |
//! | `@key` | `compute_key()` | RTPS 2.5 9.6.4.8 `KeyHash` |
//!
//! # Refused Constructs
//!
//! A type using one of these makes [`CMicroGenerator::generate`] fail with
//! the type and the reason, rather than emit code that would serialize
//! something else: `map<K, V>` (no dynamic storage), `bitset`, `fixed`, a
//! `@mutable` union case with member id 0 (the discriminator's id).

#![allow(clippy::uninlined_format_args)]

use std::fmt::Write as _;

use crate::ast::{Bitmask, Definition, Enum, IdlFile, Struct, Typedef, Union};
use crate::codegen::c::xcdr2::{self, Flavor, Names};
use crate::codegen::data_model::{bitmask_width, enum_width};
use crate::codegen::key_holder::TypeIndex;
use crate::codegen::CodeGenerator;
use crate::error::{ErrorKind, ParseError, Position, Result};
use crate::types::{Annotation, IdlType, PrimitiveType};

use super::keywords::c_ident;

mod key_hash;

/// Configuration for C micro code generation
#[derive(Debug, Clone)]
pub struct CMicroConfig {
    /// Bytes of the `char[]` holding an unbounded string, NUL included
    /// (also the `uint16_t[]` units of an unbounded wstring)
    pub max_string_len: usize,
    /// Capacity of an unbounded sequence
    pub max_sequence_len: usize,
    /// Function prefix (e.g., "hdds_" or "")
    pub prefix: String,
    /// Include guard prefix
    pub guard_prefix: String,
}

impl Default for CMicroConfig {
    fn default() -> Self {
        Self {
            max_string_len: 64,
            max_sequence_len: 32,
            prefix: String::new(),
            guard_prefix: String::new(),
        }
    }
}

/// Generates header-only C code for embedded MCUs
pub struct CMicroGenerator {
    config: CMicroConfig,
}

/// A definition and the C name it is declared under (module prefix).
struct Item<'a> {
    def: &'a Definition,
    cname: String,
}

fn bit_bound(annotations: &[Annotation]) -> Option<u32> {
    annotations.iter().find_map(|a| match a {
        Annotation::BitBound(n) => Some(*n),
        _ => None,
    })
}

fn refuse(what: &str, name: &str, reason: &str) -> ParseError {
    ParseError::new(
        ErrorKind::Other,
        Position::new(0, 0),
        format!("c-micro backend: {what} `{name}` refused: {reason}"),
    )
}

impl CMicroGenerator {
    /// Creates a new C micro generator with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: CMicroConfig::default(),
        }
    }

    /// Creates a new C micro generator with custom configuration.
    #[must_use]
    pub const fn with_config(config: CMicroConfig) -> Self {
        Self { config }
    }

    fn flavor(&self) -> Flavor {
        Flavor::Micro {
            max_string: u32::try_from(self.config.max_string_len).unwrap_or(u32::MAX),
            max_seq: u32::try_from(self.config.max_sequence_len).unwrap_or(u32::MAX),
        }
    }

    /// The C declaration of `d` of IDL type `t` (a declarator: arrays nest).
    fn decl(&self, t: &IdlType, d: &str) -> std::result::Result<String, String> {
        let s = self.config.max_string_len;
        Ok(match t {
            IdlType::BoundedString { wide: false, bound } => {
                format!("char {d}[{}]", u64::from(*bound) + 1)
            }
            IdlType::BoundedString { wide: true, bound } => {
                format!("uint16_t {d}[{}]", u64::from(*bound) + 1)
            }
            IdlType::Primitive(PrimitiveType::String) => format!("char {d}[{s}]"),
            IdlType::Primitive(PrimitiveType::WString) => format!("uint16_t {d}[{s}]"),
            IdlType::Primitive(PrimitiveType::Fixed { .. }) => {
                return Err("`fixed` is not supported by this backend".into())
            }
            IdlType::Primitive(p) => format!("{} {d}", self.flavor().prim(p)),
            // `sequence<char>` / `sequence<wchar>` included: a sequence on
            // the wire, not a string.
            IdlType::Sequence { inner, bound } => {
                let cap = bound.map_or(self.config.max_sequence_len as u64, u64::from);
                format!(
                    "struct {{ {}; uint32_t count; }} {d}",
                    self.decl(inner, &format!("data[{cap}]"))?
                )
            }
            IdlType::Array { inner, size } => self.decl(inner, &format!("{d}[{size}]"))?,
            IdlType::Map { .. } => return Err("a map is not supported by this backend".into()),
            IdlType::Named(n) => format!("{} {d}", xcdr2::micro_type_name(n)),
        })
    }

    /// The definitions of `defs` in order (their names already carry their
    /// full module path, `codegen::c::scoping`).
    fn collect<'a>(defs: &'a [Definition], out: &mut Vec<Item<'a>>) {
        for def in defs {
            let name = match def {
                Definition::Module(m) => {
                    Self::collect(&m.definitions, out);
                    continue;
                }
                Definition::Struct(s) => &s.name,
                Definition::Union(u) => &u.name,
                Definition::Enum(e) => &e.name,
                Definition::Typedef(t) => &t.name,
                Definition::Bitmask(b) => &b.name,
                _ => continue,
            };
            out.push(Item {
                def,
                cname: xcdr2::micro_type_name(name),
            });
        }
    }
}

impl Default for CMicroGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGenerator for CMicroGenerator {
    fn generate(&self, ast: &IdlFile) -> Result<String> {
        // Types named by their full module path, references resolved.
        let scoped = crate::codegen::c::scoping::scope(ast).map_err(|e| {
            ParseError::new(
                ErrorKind::Other,
                Position::new(0, 0),
                format!("c-micro backend: {e}"),
            )
        })?;
        let ast = &scoped.ast;
        let idx = TypeIndex::new(ast);
        let fl = self.flavor();
        let mut items = Vec::new();
        Self::collect(&ast.definitions, &mut items);

        let mut output = String::new();
        output.push_str("/**\n");
        output.push_str(" * @file generated_types.h\n");
        output.push_str(" * @brief Auto-generated by hddsgen gen c-micro\n");
        output.push_str(" * @note DO NOT EDIT\n");
        output.push_str(" */\n\n");
        output.push_str("#ifndef GENERATED_TYPES_H\n");
        output.push_str("#define GENERATED_TYPES_H\n\n");
        output.push_str("#include <stdint.h>\n");
        output.push_str("#include <stddef.h>\n");
        output.push_str("#include <string.h>\n");
        output.push_str("#include \"hdds_micro_cdr.h\"\n\n");
        output.push_str("#ifdef __cplusplus\n");
        output.push_str("extern \"C\" {\n");
        output.push_str("#endif\n\n");

        // The KeyHash runtime, when a struct has a key (RTPS 2.5 9.6.4.8).
        if let Some(runtime) = key_hash::begin_file(ast) {
            output.push_str(&runtime);
        }
        output.push_str(&xcdr2::runtime::runtime(fl, "inline"));
        output.push('\n');

        // Forward declarations and codec prototypes.
        output.push_str("/* Forward declarations */\n");
        for it in &items {
            if matches!(it.def, Definition::Struct(_) | Definition::Union(_)) {
                let _ = writeln!(output, "typedef struct {0} {0};", it.cname);
            }
        }
        output.push('\n');
        let mut protos = String::new();
        for it in &items {
            if matches!(it.def, Definition::Struct(_) | Definition::Union(_)) {
                let (enc, dec) = fl.fn_names(&it.cname);
                protos.push_str(&xcdr2::prototypes(
                    fl,
                    "inline",
                    &Names {
                        ctype: &it.cname,
                        enc: &enc,
                        dec: &dec,
                    },
                ));
            }
        }
        output.push_str(&protos);
        output.push('\n');

        for it in &items {
            let text = match it.def {
                Definition::Struct(s) => self.generate_struct(&idx, s, &it.cname)?,
                Definition::Union(u) => self.generate_union(&idx, u, &it.cname)?,
                Definition::Enum(e) => Self::generate_enum(e, &it.cname),
                Definition::Typedef(t) => self.generate_typedef(t, &it.cname)?,
                Definition::Bitmask(b) => Self::generate_bitmask(b, &it.cname),
                _ => String::new(),
            };
            output.push_str(&text);
        }

        output.push_str("#ifdef __cplusplus\n");
        output.push_str("}\n");
        output.push_str("#endif\n\n");
        output.push_str("#endif /* GENERATED_TYPES_H */\n");

        Ok(output)
    }
}

impl CMicroGenerator {
    fn generate_enum(e: &Enum, cname: &str) -> String {
        let mut output = String::new();
        let func_prefix = xcdr2::snake(cname);
        let width = enum_width(bit_bound(&e.annotations));
        let (uint, sint, bits) = match width {
            1 => ("uint8_t", "int8_t", 8),
            2 => ("uint16_t", "int16_t", 16),
            _ => ("uint32_t", "int32_t", 32),
        };
        let _ = writeln!(output, "/* Enum: {cname} ({width} byte(s) on the wire) */");
        let _ = writeln!(output, "enum {cname} {{");
        let mut values = Vec::new();
        for (i, (variant, val)) in e.variants.iter().zip(e.resolved_values()).enumerate() {
            values.push(val);
            let comma = if i + 1 == e.variants.len() { "" } else { "," };
            let _ = writeln!(
                output,
                "    {}_{} = {val}{comma}",
                cname.to_uppercase(),
                variant.name.to_uppercase()
            );
        }
        output.push_str("};\n");
        let _ = writeln!(output, "typedef enum {cname} {cname};\n");
        values.sort_unstable();
        values.dedup();
        let cases: Vec<String> = values.iter().map(|v| format!("case {v}:")).collect();

        let _ = write!(
            output,
            "static inline int32_t {func_prefix}_encode(const {cname}* self, hdds_cdr_t* cdr) {{\n\
             \x20   if (self == NULL || cdr == NULL) {{ return HDDS_CDR_ERR_NULL; }}\n\
             \x20   return hdds_x2m_w{bits}(cdr, ({uint})({sint})*self);\n\
             }}\n\n\
             static inline int32_t {func_prefix}_decode({cname}* self, hdds_cdr_t* cdr) {{\n\
             \x20   {uint} t;\n\
             \x20   int32_t v;\n\
             \x20   int rc;\n\
             \x20   if (self == NULL || cdr == NULL) {{ return HDDS_CDR_ERR_NULL; }}\n\
             \x20   rc = hdds_x2m_r{bits}(cdr, &t);\n\
             \x20   if (rc != 0) {{ return rc; }}\n\
             \x20   v = (int32_t)({sint})t;\n\
             \x20   switch (v) {{ {} break; default: return HDDS_CDR_ERR_INVALID; }}\n\
             \x20   *self = ({cname})v;\n\
             \x20   return HDDS_CDR_OK;\n\
             }}\n\n",
            cases.join(" ")
        );
        output
    }

    fn generate_typedef(&self, t: &Typedef, cname: &str) -> Result<String> {
        let d = self
            .decl(&t.base_type, cname)
            .map_err(|r| refuse("typedef", &t.name, &r))?;
        Ok(format!("typedef {d};\n\n"))
    }

    fn generate_bitmask(m: &Bitmask, cname: &str) -> String {
        let holder = match bitmask_width(bit_bound(&m.annotations)) {
            1 => "uint8_t",
            2 => "uint16_t",
            4 => "uint32_t",
            _ => "uint64_t",
        };
        let mut out = format!("typedef {holder} {cname};\n");
        let mut next_pos: u32 = 0;
        for flag in &m.flags {
            let pos = flag.annotations.iter().find_map(|a| match a {
                Annotation::Position(p) => Some(*p),
                _ => None,
            });
            let bit = pos.unwrap_or_else(|| {
                let p = next_pos;
                next_pos += 1;
                p
            });
            let _ = writeln!(
                out,
                "#define {}_{} (({cname})1 << {bit})",
                cname.to_uppercase(),
                flag.name.to_uppercase()
            );
        }
        out.push('\n');
        out
    }

    fn generate_struct(&self, idx: &TypeIndex, s: &Struct, cname: &str) -> Result<String> {
        let fl = self.flavor();
        let mut output = String::new();
        let func_prefix = xcdr2::snake(cname);
        let refused = |r: String| refuse("struct", &s.name, &r);

        let _ = writeln!(output, "/* Struct: {cname} */");
        let _ = writeln!(output, "struct {cname} {{");
        if let Some(base) = &s.base_struct {
            let _ = writeln!(output, "    {} base;", xcdr2::micro_type_name(base));
        }
        for field in &s.fields {
            let escaped = c_ident(&field.name);
            if field.is_optional() {
                let _ = writeln!(output, "    uint8_t has_{escaped};");
            }
            let d = self.decl(&field.field_type, &escaped).map_err(refused)?;
            let _ = writeln!(output, "    {d};");
        }
        if s.fields.is_empty() && s.base_struct.is_none() {
            output.push_str("    uint8_t hdds_empty_; /* ISO C has no empty struct */\n");
        }
        output.push_str("};\n\n");

        if let Some(max) = xcdr2::struct_bound(idx, fl, s, 0) {
            let _ = writeln!(
                output,
                "/* Upper bound of the XCDR2 size of a {cname} */\n#define {}_ENCODED_SIZE_MAX {max}u\n",
                cname.to_uppercase()
            );
        }

        let (enc, dec) = fl.fn_names(cname);
        let names = Names {
            ctype: cname,
            enc: &enc,
            dec: &dec,
        };
        output.push_str(&xcdr2::struct_codec(idx, fl, "inline", s, &names).map_err(refused)?);

        // Compute key function: instance KeyHash, RTPS 2.5 9.6.4.8
        output.push_str(&key_hash::compute_key_function(s, &func_prefix, cname));
        Ok(output)
    }

    fn generate_union(&self, idx: &TypeIndex, u: &Union, cname: &str) -> Result<String> {
        let fl = self.flavor();
        let refused = |r: String| refuse("union", &u.name, &r);
        let mut output = String::new();
        let disc = self.decl(&u.discriminator, "_d").map_err(refused)?;
        let _ = writeln!(output, "/* Union: {cname} */");
        let _ = writeln!(output, "struct {cname} {{");
        let _ = writeln!(output, "    {disc};");
        output.push_str("    union {\n");
        for case in &u.cases {
            if matches!(
                case.field.field_type,
                IdlType::Primitive(PrimitiveType::Void)
            ) {
                continue;
            }
            let d = self
                .decl(&case.field.field_type, &c_ident(&case.field.name))
                .map_err(refused)?;
            let _ = writeln!(output, "        {d};");
        }
        output.push_str("    } _u;\n");
        output.push_str("};\n\n");
        let (enc, dec) = fl.fn_names(cname);
        let names = Names {
            ctype: cname,
            enc: &enc,
            dec: &dec,
        };
        output.push_str(&xcdr2::union_codec(idx, fl, "inline", u, &names).map_err(refused)?);
        Ok(output)
    }
}
