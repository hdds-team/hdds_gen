// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Type definition generation for C.
//!
//! Note: `uninlined_format_args` allowed here due to extensive `format_args!` usage
//! in code generation that would require significant refactoring.
//!
//! Generates C struct, enum, union, typedef, bitset, and bitmask definitions,
//! and the public codec entry points of structures and unions.

#![allow(clippy::uninlined_format_args)]

use super::super::keywords::c_ident;
use super::{
    helpers::{c_name, push_fmt, to_upper_ascii},
    index::DefinitionIndex,
    xcdr2::{self, Flavor, Names},
    CGenerator,
};
use crate::ast::{Bitmask, Bitset, Definition, Enum, Struct, Typedef, Union};
use crate::codegen::data_model::{self, bitmask_width, member_chain, Ext};
use crate::codegen::key_holder::{Named, TypeIndex};
use crate::types::{Annotation, IdlType, PrimitiveType};

impl CGenerator {
    pub(super) fn emit_struct(&self, s: &Struct) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!("{}typedef struct {} {{\n", self.indent(), s.name),
        );

        if let Some(base) = &s.base_struct {
            push_fmt(
                &mut out,
                format_args!("{}    {} base;\n", self.indent(), base),
            );
        }

        for f in &s.fields {
            let escaped = c_ident(&f.name);
            if f.is_optional() {
                push_fmt(
                    &mut out,
                    format_args!("{}    uint8_t has_{};\n", self.indent(), escaped),
                );
            }
            push_fmt(
                &mut out,
                format_args!(
                    "{}    {};\n",
                    self.indent(),
                    Self::decl(&f.field_type, &escaped)
                ),
            );
        }

        // ISO C has no empty structure (C99 6.7.2.1: `-pedantic-errors`
        // refuses one): a one-byte placeholder the codec never reads nor
        // writes (an empty FINAL structure is zero bytes on the wire,
        // DDS-XTypes 1.3 7.4.3.5.3).
        if s.fields.is_empty() && s.base_struct.is_none() {
            push_fmt(
                &mut out,
                format_args!(
                    "{}    uint8_t hdds_empty_; /* ISO C has no empty struct; not serialized */\n",
                    self.indent()
                ),
            );
        }

        push_fmt(
            &mut out,
            format_args!("{}}} {} ;\n\n", self.indent(), s.name),
        );
        out
    }

    pub(super) fn emit_enum(&self, e: &Enum) -> String {
        let mut out = String::new();
        push_fmt(&mut out, format_args!("{}typedef enum {{\n", self.indent()));
        let ename_up = to_upper_ascii(&e.name);
        for (i, v) in e.variants.iter().enumerate() {
            let vname_up = to_upper_ascii(&v.name);
            let comma = if i + 1 == e.variants.len() { "" } else { "," };
            if let Some(val) = v.value {
                push_fmt(
                    &mut out,
                    format_args!(
                        "{}    {}_{} = {}{}\n",
                        self.indent(),
                        ename_up,
                        vname_up,
                        val,
                        comma
                    ),
                );
            } else {
                push_fmt(
                    &mut out,
                    format_args!("{}    {}_{}{}\n", self.indent(), ename_up, vname_up, comma),
                );
            }
        }
        push_fmt(
            &mut out,
            format_args!("{}}} {} ;\n\n", self.indent(), e.name),
        );
        out
    }

    pub(super) fn emit_typedef(&self, t: &Typedef) -> String {
        format!(
            "{}typedef {} ;\n\n",
            self.indent(),
            Self::decl(&t.base_type, &t.name)
        )
    }

    pub(super) fn emit_bitset(&self, b: &Bitset) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!(
                "{}typedef struct {} {{ uint64_t bits; }} {} ;\n",
                self.indent(),
                b.name,
                b.name
            ),
        );
        let mut next_pos: u32 = 0;
        for f in &b.fields {
            let mut pos = None;
            for ann in &f.annotations {
                if let Annotation::Position(p) = ann {
                    pos = Some(*p);
                    break;
                }
            }
            let start = pos.unwrap_or_else(|| {
                let p = next_pos;
                next_pos += f.width;
                p
            });
            let escaped = c_ident(&f.name);
            push_fmt(
                &mut out,
                format_args!(
                    "{}/* {}: width {} at bit {} */\n",
                    self.indent(),
                    f.name,
                    f.width,
                    start
                ),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}static inline uint64_t {n}_get_{f}(const {n}* s) {{ return (s->bits >> {start}) & ((((uint64_t)1) << {w}) - 1u); }}\n",
                    self.indent(),
                    n = b.name,
                    f = escaped,
                    start = start,
                    w = f.width
                ),
            );
            push_fmt(
                &mut out,
                format_args!(
                    "{}static inline void {n}_set_{f}({n}* s, uint64_t v) {{ uint64_t mask = ((((uint64_t)1) << {w}) - 1u) << {start}; s->bits = (s->bits & ~mask) | (((v) & ((((uint64_t)1) << {w}) - 1u)) << {start}); }}\n",
                    self.indent(),
                    n = b.name,
                    f = escaped,
                    w = f.width,
                    start = start
                ),
            );
        }
        out.push('\n');
        out
    }

    pub(super) fn emit_bitmask(&self, m: &Bitmask) -> String {
        let mut out = String::new();
        // The holder is the serialized width (@bit_bound, XTypes 1.3
        // 7.3.1.2.1.6 / 7.4.3.5.3).
        let width = bitmask_width(m.annotations.iter().find_map(|a| match a {
            Annotation::BitBound(n) => Some(*n),
            _ => None,
        }));
        let holder = match width {
            1 => "uint8_t",
            2 => "uint16_t",
            4 => "uint32_t",
            _ => "uint64_t",
        };
        push_fmt(
            &mut out,
            format_args!("{}typedef {} {} ;\n", self.indent(), holder, m.name),
        );
        let mut next_pos: u32 = 0;
        for flag in &m.flags {
            let mut pos = None;
            for ann in &flag.annotations {
                if let Annotation::Position(p) = ann {
                    pos = Some(*p);
                    break;
                }
            }
            let bit = pos.unwrap_or_else(|| {
                let p = next_pos;
                next_pos += 1;
                p
            });
            push_fmt(
                &mut out,
                format_args!(
                    "{}#define {tn}_{fn} (({ty})1 << {bit})\n",
                    self.indent(),
                    tn = m.name.to_uppercase(),
                    fn = flag.name.to_uppercase(),
                    ty = m.name,
                    bit = bit
                ),
            );
        }
        out.push('\n');
        out
    }

    pub(super) fn emit_union(&self, u: &Union) -> String {
        let mut out = String::new();
        push_fmt(
            &mut out,
            format_args!(
                "{}/* Union: {} (discriminator: {}) */\n",
                self.indent(),
                u.name,
                match &u.discriminator {
                    IdlType::Primitive(p) => format!("{:?}", p),
                    IdlType::Named(n) => n.clone(),
                    other => other.to_idl_string(),
                }
            ),
        );
        push_fmt(
            &mut out,
            format_args!("{}typedef struct {} {{\n", self.indent(), u.name),
        );
        push_fmt(
            &mut out,
            format_args!(
                "{}    {} _d; /* discriminator */\n",
                self.indent(),
                Self::type_to_c(&u.discriminator)
            ),
        );
        push_fmt(&mut out, format_args!("{}    union {{\n", self.indent()));
        for case in &u.cases {
            if matches!(
                case.field.field_type,
                IdlType::Primitive(crate::types::PrimitiveType::Void)
            ) {
                continue;
            }
            let escaped = c_ident(&case.field.name);
            push_fmt(
                &mut out,
                format_args!(
                    "{}        {};\n",
                    self.indent(),
                    Self::decl(&case.field.field_type, &escaped)
                ),
            );
        }
        push_fmt(&mut out, format_args!("{}    }} _u;\n", self.indent()));
        push_fmt(
            &mut out,
            format_args!("{}}} {} ;\n\n", self.indent(), u.name),
        );
        out
    }

    pub(super) fn emit_type_definitions(&self, defs: &[&Definition]) -> String {
        let mut out = String::new();
        for d in defs {
            match d {
                Definition::Struct(s) => out.push_str(&self.emit_struct(s)),
                Definition::Enum(e) => out.push_str(&self.emit_enum(e)),
                Definition::Typedef(t) => out.push_str(&self.emit_typedef(t)),
                Definition::Bitset(b) => out.push_str(&self.emit_bitset(b)),
                Definition::Bitmask(m) => out.push_str(&self.emit_bitmask(m)),
                Definition::Union(u) => out.push_str(&self.emit_union(u)),
                Definition::Const(c) => push_fmt(
                    &mut out,
                    format_args!(
                        "#define {} ({})\n\n",
                        c.name,
                        crate::codegen::const_literal::literal(
                            c,
                            crate::codegen::const_literal::Lang::C
                        )
                    ),
                ),
                _ => {}
            }
        }
        out
    }

    /// Prototypes of every encoder and decoder (a structure may hold a
    /// union declared after its own functions).
    pub(super) fn emit_codec_prototypes(defs: &[&Definition]) -> String {
        let mut out = String::new();
        for def in defs {
            let name = match def {
                Definition::Struct(s) => &s.name,
                Definition::Union(u) => &u.name,
                _ => continue,
            };
            let (enc, dec) = Flavor::C.fn_names(name);
            let names = Names {
                ctype: name,
                enc: &enc,
                dec: &dec,
            };
            out.push_str(&xcdr2::prototypes(Flavor::C, "inline", &names));
            out.push_str(&xcdr2::prototype_dec1("inline", &names));
        }
        out.push('\n');
        out
    }

    pub(super) fn emit_struct_helpers_section(
        defs: &[&Definition],
        idx: &DefinitionIndex,
        idl_names: &std::collections::HashMap<String, String>,
    ) -> String {
        let mut out = String::new();
        for def in defs {
            if let Definition::Struct(sdef) = def {
                let (enc, dec) = Flavor::C.fn_names(&sdef.name);
                let names = Names {
                    ctype: &sdef.name,
                    enc: &enc,
                    dec: &dec,
                };
                out.push_str(&Self::codec_or_error(
                    &sdef.name,
                    idl_names
                        .get(&sdef.name)
                        .map_or(sdef.name.as_str(), String::as_str),
                    "struct",
                    data_model::struct_ext(sdef),
                    idx.keys.is_keyed(sdef),
                    struct_needs_allocator(&idx.keys, sdef),
                    xcdr2::struct_codec(&idx.keys, Flavor::C, "inline", sdef, &names).and_then(
                        |body| {
                            xcdr2::struct_dec1(&idx.keys, "inline", sdef, &names)
                                .map(|dec1| body + &dec1)
                        },
                    ),
                ));
                out.push_str(&super::type_descriptor::generate_type_descriptor_scoped(
                    sdef, &idx.keys, idl_names,
                ));
            }
        }
        out
    }

    pub(super) fn emit_union_helpers_section(
        defs: &[&Definition],
        idx: &DefinitionIndex,
        idl_names: &std::collections::HashMap<String, String>,
    ) -> String {
        let mut out = String::new();
        for def in defs {
            if let Definition::Union(udef) = def {
                let (enc, dec) = Flavor::C.fn_names(&udef.name);
                let names = Names {
                    ctype: &udef.name,
                    enc: &enc,
                    dec: &dec,
                };
                out.push_str(&Self::codec_or_error(
                    &udef.name,
                    idl_names
                        .get(&udef.name)
                        .map_or(udef.name.as_str(), String::as_str),
                    "union",
                    data_model::union_ext(udef),
                    udef.discriminator_is_key(),
                    union_needs_allocator(&idx.keys, udef),
                    xcdr2::union_codec(&idx.keys, Flavor::C, "inline", udef, &names).and_then(
                        |body| {
                            xcdr2::union_dec1(&idx.keys, "inline", udef, &names)
                                .map(|dec1| body + &dec1)
                        },
                    ),
                ));
                out.push_str(&super::type_descriptor::union_key_operations(
                    udef, &idx.keys,
                ));
            }
        }
        out
    }

    /// The codec functions and the public entry points of `name`, or an
    /// `#error` naming why this backend cannot serialize it.
    fn codec_or_error(
        name: &str,
        idl_name: &str,
        what: &str,
        ext: Ext,
        keyed: bool,
        needs_alloc: bool,
        codec: Result<String, String>,
    ) -> String {
        match codec {
            Ok(body) => {
                let mut out = body;
                out.push_str(&entry_points(name, needs_alloc));
                out.push_str(&extensibility_entry(name, ext));
                out.push_str(&writer_create_helpers(name, idl_name, keyed));
                out
            }
            Err(reason) => format!(
                "#error \"hddsgen: {what} {name}: {}\"\n\n",
                reason.replace(['"', '\\'], "'")
            ),
        }
    }
}

/// The extensibility of the structure or union `name` (DDS-XTypes 1.3
/// 7.2.2.4.4), as the macro `<NAME>_EXTENSIBILITY` and the function
/// `<name>_extensibility()`: the value of `HDDS_EXTENSIBILITY_FINAL` (0),
/// `_APPENDABLE` (1) or `_MUTABLE` (2) of the hdds C API, the argument of
/// `hdds_writer_create_with_extensibility`. A writer given it labels every
/// sample with the encapsulation identifier its body needs (Table 60:
/// `CDR2_LE`, `D_CDR2_LE` for a body that starts with a DHEADER,
/// `PL_CDR2_LE`).
fn extensibility_entry(name: &str, ext: Ext) -> String {
    let f = c_name(name);
    let upper = to_upper_ascii(&f);
    let code = ext.hdds_code();
    let kw = to_upper_ascii(ext.keyword());
    format!(
        "/* Extensibility of {name} (DDS-XTypes 1.3 7.2.2.4.4): HDDS_EXTENSIBILITY_{kw}
 * of the hdds C API. Create its writers with
 * hdds_writer_create_with_extensibility(..., {f}_extensibility()) so every
 * sample's encapsulation identifier names it (Table 60). */
#define {upper}_EXTENSIBILITY {code}u

static inline uint32_t {f}_extensibility(void) {{
    return {upper}_EXTENSIBILITY;
}}

"
    )
}

/// `<name>_create_writer` and `<name>_create_writer_from_publisher`: the
/// natural way to create a `DataWriter` of `name` with the hdds C ABI. The
/// older `hdds_writer_create*`/`hdds_publisher_create_writer*` functions
/// label every sample FINAL (`CDR2_LE`, 0x0007) regardless of the type's
/// real extensibility; these two call `hdds_writer_create_with_extensibility`
/// / `hdds_publisher_create_writer_with_type` with the IDL name and
/// `{f}_extensibility()` (DDS-XTypes 1.3 Table 60), so every sample of
/// `name` is labelled right by construction. `HddsParticipant`,
/// `HddsPublisher`, `HddsQoS` and `HddsDataWriter` are forward-declared
/// (once per file, see `writer_create_prelude`): this header still needs
/// no `#include` of the hdds C ABI header, only a program that also
/// includes it and links `libhdds_c` to call these two functions.
fn writer_create_helpers(name: &str, idl_name: &str, keyed: bool) -> String {
    let mut out = writer_create_helper(name, idl_name, false);
    if keyed {
        out.push_str(&writer_create_helper(name, idl_name, true));
    }
    out
}

fn writer_create_helper(name: &str, idl_name: &str, keyed: bool) -> String {
    let helper = if keyed {
        "create_keyed_writer"
    } else {
        "create_writer"
    };
    let create = if keyed {
        "hdds_writer_create_keyed"
    } else {
        "hdds_writer_create_with_extensibility"
    };
    let pub_create = if keyed {
        "hdds_publisher_create_writer_keyed"
    } else {
        "hdds_publisher_create_writer_with_type"
    };
    let f = c_name(name);
    // The IDL qualified name, the one the other backends announce: a NULL
    // name would make the writer adopt whatever name its topic announces
    // (DDS-XTypes 1.3 Sec.7.6.3.4.2 Step 2 compares names).
    let type_arg = format!("\"{idl_name}\"");
    let pub_type_arg = format!("\"{idl_name}\", ");
    format!(
        "/* The natural way to create a DataWriter of {name}: every sample is\n\
 * labelled with {f}_extensibility() (Table 60), unlike hdds_writer_create()\n\
 * and the other older constructors, which always label FINAL. `type_name`\n\
 * is the IDL qualified name, as every other backend announces it. */\n\
static inline struct HddsDataWriter *{f}_{helper}(struct HddsParticipant *aParticipant,\n\
                                                        const char *aTopicName,\n\
                                                        const struct HddsQoS *aQos) {{\n    \
    return {create}(aParticipant, aTopicName, {type_arg}, aQos, {f}_extensibility());\n\
}}\n\n\
/* Same as {f}_{helper}, from a Publisher (DDS v1.4 Sec.2.2.2.4.1). */\n\
static inline struct HddsDataWriter *{f}_{helper}_from_publisher(struct HddsPublisher *aPublisher,\n\
                                                                      const char *aTopicName,\n\
                                                                      const struct HddsQoS *aQos) {{\n    \
    return {pub_create}(aPublisher, aTopicName, {pub_type_arg}aQos, {f}_extensibility());\n\
}}\n\n"
    )
}

/// Forward declarations of the hdds C ABI entities `<name>_create_writer`
/// and `<name>_create_writer_from_publisher` need (`HddsParticipant`,
/// `HddsPublisher`, `HddsQoS`, `HddsDataWriter`, and the two functions
/// they call), emitted once per generated file right before the first use.
/// A program that also `#include`s the hdds C ABI header (`hdds.h`) sees
/// the same opaque structure tags and identical function prototypes twice,
/// which ISO C allows (6.7/C99, 6.2.7p2 compatible types): this generated
/// header keeps compiling stand-alone (codec-only, no hdds runtime linked)
/// as long as these two writer-creation functions are never called.
pub(super) fn writer_create_prelude() -> String {
    let declarations = "/* Forward declarations of the hdds C ABI (see hdds.h): only needed to\n\
 * call <name>_create_writer/<name>_create_writer_from_publisher below, and\n\
 * only resolved at link time against libhdds_c. */\n\
struct HddsParticipant;\n\
struct HddsPublisher;\n\
struct HddsQoS;\n\
struct HddsDataWriter;\n\n\
struct HddsDataWriter *hdds_writer_create_with_extensibility(struct HddsParticipant *aParticipant,\n\
                                                              const char *aTopicName,\n\
                                                              const char *aTypeName,\n\
                                                              const struct HddsQoS *aQos,\n\
                                                              uint32_t aExtensibility);\n\n\
struct HddsDataWriter *hdds_publisher_create_writer_with_type(struct HddsPublisher *aPublisher,\n\
                                                              const char *aTopicName,\n\
                                                              const char *aTypeName,\n\
                                                              const struct HddsQoS *aQos,\n\
                                                              uint32_t aExtensibility);\n\n"
        .to_string();
    declarations + "struct HddsDataWriter *hdds_writer_create_keyed(struct HddsParticipant *aParticipant, const char *aTopicName, const char *aTypeName, const struct HddsQoS *aQos, uint32_t aExtensibility);\n\nstruct HddsDataWriter *hdds_publisher_create_writer_keyed(struct HddsPublisher *aPublisher, const char *aTopicName, const char *aTypeName, const struct HddsQoS *aQos, uint32_t aExtensibility);\n\n"
}

/// Whether the structure `s` holds storage the C decoder takes from an
/// allocator (see `entry_points`): one of its members does, through
/// typedefs, arrays, structures and unions.
fn struct_needs_allocator(idx: &TypeIndex, s: &Struct) -> bool {
    let mut seen = vec![s.name.clone()];
    member_chain(idx, s).is_ok_and(|chain| {
        chain
            .iter()
            .any(|m| holds_storage(idx, &m.field.field_type, &mut seen))
    })
}

/// Whether the union `u` holds storage the C decoder takes from an
/// allocator (see `struct_needs_allocator`).
fn union_needs_allocator(idx: &TypeIndex, u: &Union) -> bool {
    let mut seen = vec![u.name.clone()];
    u.cases
        .iter()
        .any(|c| holds_storage(idx, &c.field.field_type, &mut seen))
}

/// Whether a value of `ty` holds storage the C decoder obtains from the
/// reader's allocator: a string or wstring (unbounded: a pointer), any
/// sequence or map (every one is a pointer holder in C, bounded or not).
/// `seen` holds the named aggregates on the walk, against cycles.
fn holds_storage(idx: &TypeIndex, ty: &IdlType, seen: &mut Vec<String>) -> bool {
    match idx.resolve(ty) {
        IdlType::Primitive(PrimitiveType::String | PrimitiveType::WString)
        | IdlType::Sequence { .. }
        | IdlType::Map { .. } => true,
        IdlType::Array { inner, .. } => holds_storage(idx, &inner, seen),
        IdlType::Named(n) => {
            if seen.contains(&n) {
                return false;
            }
            seen.push(n.clone());
            match idx.named(&n) {
                Some(Named::Struct(s)) => member_chain(idx, s).is_ok_and(|chain| {
                    chain
                        .iter()
                        .any(|m| holds_storage(idx, &m.field.field_type, seen))
                }),
                Some(Named::Union(u)) => u
                    .cases
                    .iter()
                    .any(|c| holds_storage(idx, &c.field.field_type, seen)),
                _ => false,
            }
        }
        _ => false,
    }
}

/// The public entry points of the structure or union `name` (their names
/// and signatures are those of earlier hddsgen releases): `_encode_cdr2_le`
/// (returns the byte count or a negative error), `_encode_cdr2_le_at`
/// (aligns from the start of `dst`), the decoders, and `_max_cdr2_size`,
/// the exact XCDR2 size of the value (0 when it cannot be encoded).
///
/// The XCDR1 decoders (`_decode_cdr1_le[_at][_alloc]`, the body of a
/// `CDR_LE` or `PL_CDR_LE` sample) have the same signatures, and
/// `_decode_by_encapsulation_alloc` picks the decoder from the
/// encapsulation identifier of the sample (DDS-XTypes 1.3 7.6.3.1.2, Table
/// 60: `CDR_LE` 0x0001, `PL_CDR_LE` 0x0003, `CDR2_LE` 0x0007, `D_CDR2_LE`
/// 0x0009, `PL_CDR2_LE` 0x000b; a big-endian one is refused with
/// `-CDR_INVALID_DATA`). `HDDSGEN_C_DECODES_XCDR1` is defined.
///
/// The `_alloc` decoders take the allocator the decoder obtains the
/// storage of every pointer holder from (an unbounded string or wstring,
/// the elements of a sequence or map), the size computed from bytes it has
/// already bounded: the decoder never writes through a pointer the caller
/// set. Without an allocator (the decoders of earlier releases' names), a
/// value holding such a member is refused with `-CDR_OUT_OF_MEMORY`, an
/// empty sequence or map excepted.
fn entry_points(name: &str, needs_alloc: bool) -> String {
    let f = c_name(name);
    let note = if needs_alloc {
        format!(
            "/* This type needs an allocator: {f}_decode_cdr2_le fails with\n\
             * CDR_OUT_OF_MEMORY whenever it needs storage for a sequence, a map\n\
             * or an unbounded string. Use {f}_decode_cdr2_le_alloc with an\n\
             * hdds_x2c_alloc_fn instead. */\n"
        )
    } else {
        String::new()
    };
    format!(
        "static inline int {f}_encode_cdr2_le_at(const {ty}* value, uint8_t* dst, size_t len, size_t* offset_io) {{
    hdds_x2c_w c;
    int err;
    if (value == NULL || dst == NULL || offset_io == NULL || *offset_io > len) {{ return -CDR_INVALID_DATA; }}
    c.buf = dst;
    c.size = len;
    c.pos = *offset_io;
    err = {f}__xenc(value, &c);
    if (err) {{ return err; }}
    *offset_io = c.pos;
    return CDR_OK;
}}

static inline int {f}_encode_cdr2_le(const {ty}* value, uint8_t* dst, size_t len) {{
    size_t offset = 0;
    int err = {f}_encode_cdr2_le_at(value, dst, len, &offset);
    if (err) {{ return err; }}
    return (int)offset;
}}

static inline int {f}_encode_key_cdr2_le(const {ty}* value, uint8_t* dst, size_t len) {{
    hdds_x2c_w c;
    int err;
    if (value == NULL || dst == NULL) {{ return -CDR_INVALID_DATA; }}
    c.buf = dst; c.size = len; c.pos = 0;
    err = {f}__xkey(value, &c);
    return err ? err : (int)c.pos;
}}

static inline int {f}_decode_cdr2_le_at_alloc({ty}* out, const uint8_t* src, size_t len, size_t* offset_io, hdds_x2c_alloc_fn alloc, void* actx) {{
    hdds_x2c_r c;
    int err;
    if (out == NULL || src == NULL || offset_io == NULL || *offset_io > len) {{ return -CDR_INVALID_DATA; }}
    hdds_x2c_rinit(&c, src, len, *offset_io, alloc, actx);
    err = {f}__xdec(out, &c);
    if (err) {{ return err; }}
    *offset_io = c.pos;
    return CDR_OK;
}}

static inline int {f}_decode_cdr2_le_at({ty}* out, const uint8_t* src, size_t len, size_t* offset_io) {{
    return {f}_decode_cdr2_le_at_alloc(out, src, len, offset_io, NULL, NULL);
}}

static inline int {f}_decode_cdr2_le_alloc({ty}* out, const uint8_t* src, size_t len, hdds_x2c_alloc_fn alloc, void* actx) {{
    size_t offset = 0;
    int err = {f}_decode_cdr2_le_at_alloc(out, src, len, &offset, alloc, actx);
    if (err) {{ return err; }}
    return (int)offset;
}}

{note}static inline int {f}_decode_cdr2_le({ty}* out, const uint8_t* src, size_t len) {{
    return {f}_decode_cdr2_le_alloc(out, src, len, NULL, NULL);
}}

static inline int {f}_decode_cdr1_le_at_alloc({ty}* out, const uint8_t* src, size_t len, size_t* offset_io, hdds_x2c_alloc_fn alloc, void* actx) {{
    hdds_x2c_r c;
    int err;
    if (out == NULL || src == NULL || offset_io == NULL || *offset_io > len) {{ return -CDR_INVALID_DATA; }}
    hdds_x2c_rinit(&c, src, len, *offset_io, alloc, actx);
    err = {f}__xdec1(out, &c, 0);
    if (err) {{ return err; }}
    *offset_io = c.pos;
    return CDR_OK;
}}

static inline int {f}_decode_cdr1_le_at({ty}* out, const uint8_t* src, size_t len, size_t* offset_io) {{
    return {f}_decode_cdr1_le_at_alloc(out, src, len, offset_io, NULL, NULL);
}}

static inline int {f}_decode_cdr1_le_alloc({ty}* out, const uint8_t* src, size_t len, hdds_x2c_alloc_fn alloc, void* actx) {{
    size_t offset = 0;
    int err = {f}_decode_cdr1_le_at_alloc(out, src, len, &offset, alloc, actx);
    if (err) {{ return err; }}
    return (int)offset;
}}

static inline int {f}_decode_cdr1_le({ty}* out, const uint8_t* src, size_t len) {{
    return {f}_decode_cdr1_le_alloc(out, src, len, NULL, NULL);
}}

static inline int {f}_decode_by_encapsulation_alloc({ty}* out, uint16_t encapsulation_id, const uint8_t* src, size_t len, hdds_x2c_alloc_fn alloc, void* actx) {{
    switch (encapsulation_id) {{
    case 0x0001u:
    case 0x0003u:
        return {f}_decode_cdr1_le_alloc(out, src, len, alloc, actx);
    case 0x0007u:
    case 0x0009u:
    case 0x000bu:
        return {f}_decode_cdr2_le_alloc(out, src, len, alloc, actx);
    default:
        return -CDR_INVALID_DATA;
    }}
}}

static inline size_t {f}_max_key_cdr2_size(const {ty}* value) {{
    hdds_x2c_w c;
    c.buf = NULL; c.size = (size_t)-1; c.pos = 0;
    if (value == NULL || {f}__xkey(value, &c) != 0) {{ return 0; }}
    return c.pos;
}}

static inline size_t {f}_max_cdr2_size(const {ty}* value) {{
    hdds_x2c_w c;
    c.buf = NULL;
    c.size = (size_t)-1;
    c.pos = 0;
    if (value == NULL || {f}__xenc(value, &c) != 0) {{ return 0; }}
    return c.pos;
}}

",
        ty = name,
        note = note
    )
}
