// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Helper functions for validation rules.
//!
//! Common validation logic shared across rule implementations.

use super::super::collect::resolve_typedef;
use super::super::diagnostics::{Level, ValidationDiag};
use crate::ast::{AnnotationDecl, Field};
use crate::types::{Annotation, IdlType, PrimitiveType};
use std::collections::{HashMap, HashSet};

pub(super) fn enforce_extensibility_conflicts(
    anns: &[Annotation],
    who: &str,
    diags: &mut Vec<ValidationDiag>,
) {
    use crate::types::ExtensibilityKind as EK;

    let mut kinds: HashSet<EK> = HashSet::new();
    for a in anns {
        match a {
            Annotation::Extensibility(k) => {
                kinds.insert(*k);
            }
            Annotation::Final => {
                kinds.insert(EK::Final);
            }
            Annotation::Appendable => {
                kinds.insert(EK::Appendable);
            }
            Annotation::Mutable => {
                kinds.insert(EK::Mutable);
            }
            Annotation::Custom { name, params } if name == "extensibility" => {
                if let Some(v) = params.first().map(|p| &p.value) {
                    match v.to_ascii_uppercase().as_str() {
                        "FINAL" => {
                            kinds.insert(EK::Final);
                        }
                        "APPENDABLE" => {
                            kinds.insert(EK::Appendable);
                        }
                        "MUTABLE" => {
                            kinds.insert(EK::Mutable);
                        }
                        _ => {}
                    }
                }
            }
            Annotation::DataRepresentation(val) => {
                let check = val.to_ascii_uppercase();
                let allowed = ["XCDR1", "XCDR2", "PLAIN_CDR", "PLAIN_CDR2"];
                if !allowed.contains(&check.as_str()) {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "{who}: @data_representation({val}) is not one of {allowed:?}"
                        ),
                    });
                }
            }
            Annotation::NonSerialized => {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!(
                        "{who}: @non_serialized is invalid at type level (use on members)"
                    ),
                });
            }
            _ => {}
        }
    }
    if kinds.len() > 1 {
        diags.push(ValidationDiag {
            level: Level::Error,
            message: format!("{who}: conflicting extensibility annotations (found: {kinds:?})"),
        });
    }

    // `@mutable` types are wire-encoded as PL_CDR2 in XCDR v2. The matching
    // XCDR v1 wire (PL_CDR v1: per-member PID uint16 + length uint16 header,
    // sentinel-terminated parameter list) is explicitly out of scope of the
    // current XCDR1 WIP, so pinning a `@mutable` type to XCDR1 via
    // `@data_representation("XCDR1")` or the equivalent `"PLAIN_CDR"` asks
    // the codegen for a wire format it cannot produce. Reject at parse time
    // so the user sees the error before they hit codegen.
    if kinds.contains(&EK::Mutable) {
        for a in anns {
            if let Annotation::DataRepresentation(val) = a {
                let check = val.to_ascii_uppercase();
                if check == "XCDR1" || check == "PLAIN_CDR" {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "{who}: @data_representation({val}) is not supported on @mutable types \
                             (PL_CDR v1 is out of scope; use XCDR2 or PLAIN_CDR2)"
                        ),
                    });
                }
            }
        }
    }
}

pub(super) fn validate_type_usage(
    ty: &IdlType,
    context: &str,
    diags: &mut Vec<ValidationDiag>,
    typedefs: &HashMap<String, IdlType>,
) {
    type_usage(ty, context, diags, typedefs, 0);
}

/// Typedef chains deeper than this are cyclic (the reference validation
/// reports the cycle); the walk stops there instead of overflowing.
const MAX_ALIAS_DEPTH: usize = 64;

fn type_usage(
    ty: &IdlType,
    context: &str,
    diags: &mut Vec<ValidationDiag>,
    typedefs: &HashMap<String, IdlType>,
    depth: usize,
) {
    if depth > MAX_ALIAS_DEPTH {
        return;
    }
    let validate_type_usage =
        |t: &IdlType, c: &str, d: &mut Vec<ValidationDiag>, m: &HashMap<String, IdlType>| {
            type_usage(t, c, d, m, depth + 1);
        };
    match ty {
        IdlType::Map { key, value, .. } => {
            if matches!(key.as_ref(), IdlType::Primitive(p) if matches!(p, PrimitiveType::Float | PrimitiveType::Double | PrimitiveType::LongDouble))
            {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!(
                        "{context}: map has invalid key type (floating-point not allowed)"
                    ),
                });
            }
            validate_type_usage(key, context, diags, typedefs);
            validate_type_usage(value, context, diags, typedefs);
        }
        IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => {
            validate_type_usage(inner, context, diags, typedefs);
        }
        IdlType::Named(name) => {
            if let Some(base) = resolve_typedef(name, typedefs) {
                validate_type_usage(&base, context, diags, typedefs);
            }
        }
        IdlType::Primitive(_) | IdlType::BoundedString { .. } => {}
    }
}

const fn is_numeric(t: &IdlType) -> bool {
    matches!(
        t,
        IdlType::Primitive(
            PrimitiveType::Short
                | PrimitiveType::UnsignedShort
                | PrimitiveType::Long
                | PrimitiveType::UnsignedLong
                | PrimitiveType::LongLong
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::Int8
                | PrimitiveType::Int16
                | PrimitiveType::Int32
                | PrimitiveType::Int64
                | PrimitiveType::UInt8
                | PrimitiveType::UInt16
                | PrimitiveType::UInt32
                | PrimitiveType::UInt64
                | PrimitiveType::Float
                | PrimitiveType::Double
                | PrimitiveType::LongDouble
                | PrimitiveType::Fixed { .. }
        )
    )
}

fn parse_num(s: &str) -> Option<f64> {
    s.parse::<f64>().ok()
}

#[allow(clippy::too_many_lines)]
pub(super) fn validate_field_annotations(
    struct_name: &str,
    f: &Field,
    diags: &mut Vec<ValidationDiag>,
) {
    let numeric = is_numeric(&f.field_type);
    let field_name = &f.name;
    let mut min_v: Option<f64> = None;
    let mut max_v: Option<f64> = None;
    let mut range_seen = false;
    let mut unit_seen: Option<String> = None;

    for ann in &f.annotations {
        match ann {
            Annotation::Min(v) => {
                if !numeric {
                    diags.push(ValidationDiag {
                        level: Level::Warning,
                        message: format!("{}.{}: @min on non-numeric field", struct_name, f.name),
                    });
                }
                if let Some(n) = parse_num(&v.value) {
                    min_v = Some(n);
                } else {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "{}.{}: @min value '{}' is not a number",
                            struct_name, f.name, v
                        ),
                    });
                }
            }
            Annotation::Max(v) => {
                if !numeric {
                    diags.push(ValidationDiag {
                        level: Level::Warning,
                        message: format!("{}.{}: @max on non-numeric field", struct_name, f.name),
                    });
                }
                if let Some(n) = parse_num(&v.value) {
                    max_v = Some(n);
                } else {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "{}.{}: @max value '{}' is not a number",
                            struct_name, f.name, v
                        ),
                    });
                }
            }
            Annotation::Range { min, max } => {
                range_seen = true;
                if !numeric {
                    diags.push(ValidationDiag {
                        level: Level::Warning,
                        message: format!("{}.{}: @range on non-numeric field", struct_name, f.name),
                    });
                }
                match (parse_num(&min.value), parse_num(&max.value)) {
                    (Some(a), Some(b)) => {
                        min_v = Some(a);
                        max_v = Some(b);
                    }
                    _ => diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "{}.{}: @range values '{}'..'{}' are not numbers",
                            struct_name, f.name, min, max
                        ),
                    }),
                }
            }
            Annotation::Unit(u) => {
                if !numeric {
                    diags.push(ValidationDiag {
                        level: Level::Warning,
                        message: format!("{struct_name}.{field_name}: @unit on non-numeric field"),
                    });
                }
                if unit_seen.is_some() {
                    diags.push(ValidationDiag {
                        level: Level::Warning,
                        message: format!("{struct_name}.{field_name}: multiple @unit annotations"),
                    });
                }
                unit_seen = Some(u.clone());
            }
            Annotation::DataRepresentation(val) => {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!("{struct_name}.{field_name}: @data_representation({val}) is invalid on a member (type-level only)"),
                });
            }
            _ => {}
        }
    }

    if let (Some(a), Some(b)) = (min_v, max_v) {
        if a > b {
            let src = if range_seen { "@range" } else { "@min/@max" };
            diags.push(ValidationDiag {
                level: Level::Error,
                message: format!("{struct_name}.{field_name}: {src} has min>max ({a} > {b})"),
            });
        }
    }

    if let Some((tmin, tmax)) = numeric_limits(&f.field_type) {
        if let Some(a) = min_v {
            if a < tmin {
                diags.push(ValidationDiag {
                    level: Level::Warning,
                    message: format!(
                        "{struct_name}.{field_name}: @min {a} below type minimum {tmin}"
                    ),
                });
            }
        }
        if let Some(b) = max_v {
            if b > tmax {
                diags.push(ValidationDiag {
                    level: Level::Warning,
                    message: format!(
                        "{struct_name}.{field_name}: @max {b} above type maximum {tmax}"
                    ),
                });
            }
        }
    } else if let Some((fmin, fmax)) = numeric_limits_float(&f.field_type) {
        if let Some(a) = min_v {
            if a < fmin {
                diags.push(ValidationDiag {
                    level: Level::Warning,
                    message: format!(
                        "{struct_name}.{field_name}: @min {a} below float type minimum {fmin}"
                    ),
                });
            }
        }
        if let Some(b) = max_v {
            if b > fmax {
                diags.push(ValidationDiag {
                    level: Level::Warning,
                    message: format!(
                        "{struct_name}.{field_name}: @max {b} above float type maximum {fmax}"
                    ),
                });
            }
        }
    }
}

pub(super) fn validate_custom_annotations(
    anns: &[Annotation],
    index: &HashMap<String, AnnotationDecl>,
    context: &str,
    diags: &mut Vec<ValidationDiag>,
) {
    for a in anns {
        if let Annotation::Custom { name, params } = a {
            if !index.contains_key(name) && validate_builtin(name, params, anns, context, diags) {
                continue;
            }
            if let Some(decl) = index.get(name) {
                let order: Vec<String> = decl.members.iter().map(|m| m.name.clone()).collect();
                let mut required: HashSet<String> = decl
                    .members
                    .iter()
                    .filter(|m| m.default.is_none())
                    .map(|m| m.name.clone())
                    .collect();

                let mut seen: HashSet<String> = HashSet::new();
                let mut pos_idx = 0usize;
                for crate::types::AnnotationParam { name: k, .. } in params {
                    let key = if k.is_empty() {
                        if pos_idx >= order.len() {
                            diags.push(ValidationDiag {
                                level: Level::Error,
                                message: format!(
                                    "{context}: @{name} has too many positional arguments ({actual} > {expected})",
                                    actual = params.len(),
                                    expected = order.len()
                                ),
                            });
                            break;
                        }
                        let nm = order[pos_idx].clone();
                        pos_idx += 1;
                        nm
                    } else {
                        k.clone()
                    };
                    if !order.iter().any(|n| n == &key) {
                        diags.push(ValidationDiag {
                            level: Level::Error,
                            message: format!("{context}: @{name} unknown parameter '{key}'"),
                        });
                    }
                    if !seen.insert(key.clone()) {
                        diags.push(ValidationDiag {
                            level: Level::Error,
                            message: format!("{context}: @{name} duplicate parameter '{key}'"),
                        });
                    }
                    required.remove(&key);
                }
                if !required.is_empty() {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "{context}: @{name} missing required parameter(s): {missing}",
                            missing = join_names(&required)
                        ),
                    });
                }
            } else {
                diags.push(ValidationDiag {
                    level: Level::Warning,
                    message: format!("{context}: unknown annotation '@{name}' (no declaration)"),
                });
            }
        }
    }
}

fn join_names(set: &HashSet<String>) -> String {
    let mut v: Vec<_> = set.iter().cloned().collect();
    v.sort();
    v.join(", ")
}

#[allow(clippy::cast_precision_loss)]
fn numeric_limits(t: &IdlType) -> Option<(f64, f64)> {
    match t {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Int8 => Some((f64::from(i8::MIN), f64::from(i8::MAX))),
            PrimitiveType::UInt8 | PrimitiveType::Octet => Some((0.0, f64::from(u8::MAX))),
            PrimitiveType::Int16 | PrimitiveType::Short => {
                Some((f64::from(i16::MIN), f64::from(i16::MAX)))
            }
            PrimitiveType::UInt16 | PrimitiveType::UnsignedShort => {
                Some((0.0, f64::from(u16::MAX)))
            }
            PrimitiveType::Int32 | PrimitiveType::Long => {
                Some((f64::from(i32::MIN), f64::from(i32::MAX)))
            }
            PrimitiveType::UInt32 | PrimitiveType::UnsignedLong => Some((0.0, f64::from(u32::MAX))),
            PrimitiveType::Int64 | PrimitiveType::LongLong => {
                Some((i64::MIN as f64, i64::MAX as f64))
            }
            PrimitiveType::UInt64 | PrimitiveType::UnsignedLongLong => Some((0.0, u64::MAX as f64)),
            _ => None,
        },
        _ => None,
    }
}

fn numeric_limits_float(t: &IdlType) -> Option<(f64, f64)> {
    match t {
        IdlType::Primitive(p) => match p {
            PrimitiveType::Float => Some((-f64::from(f32::MAX), f64::from(f32::MAX))),
            PrimitiveType::Double | PrimitiveType::LongDouble => Some((-(f64::MAX), f64::MAX)),
            _ => None,
        },
        _ => None,
    }
}

/// Member names declared twice in one scope: an error (IDL 4.2 7.5.2, "An
/// identifier can only be defined once in a scope"), case-insensitively,
/// since identifiers differing only in case collide (7.2.3.1). `_a` and `a`
/// are the same identifier once the escape is removed.
pub(super) fn validate_unique_member_names<'a>(
    context: &str,
    names: impl Iterator<Item = &'a str>,
    diags: &mut Vec<ValidationDiag>,
) {
    let mut seen: HashMap<String, &str> = HashMap::new();
    for name in names {
        if let Some(first) = seen.insert(name.to_ascii_lowercase(), name) {
            diags.push(ValidationDiag {
                level: Level::Error,
                message: format!("{context}: member '{name}' collides with member '{first}'"),
            });
        }
    }
}

/// The standard annotations the parser keeps as custom ones (DDS-XTypes 1.3
/// 7.3.1.2.1, IDL 4.2 8.3): checked here, never "unknown". `false` for any
/// other name.
fn validate_builtin(
    name: &str,
    params: &[crate::types::AnnotationParam],
    anns: &[Annotation],
    context: &str,
    diags: &mut Vec<ValidationDiag>,
) -> bool {
    use crate::types::AnnotationParamKind;
    match name {
        // `@hashid` / `@hashid("name")` (XTypes 1.3 7.3.1.2.1.2): the member
        // id is the hash of the member name, or of the given string.
        "hashid" => {
            let ok = match params {
                [] => true,
                [p] => {
                    (p.name.is_empty() || p.name == "value")
                        && p.kind == AnnotationParamKind::String
                }
                _ => false,
            };
            if !ok {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!(
                        "{context}: @hashid takes no parameter or one string (the name hashed)"
                    ),
                });
            }
            if anns.iter().any(|a| matches!(a, Annotation::Id(_))) {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!("{context}: @id and @hashid both set the member id; keep one"),
                });
            }
            true
        }
        // Checked where they apply (data model) or without parameters.
        "try_construct" | "shared" | "ignore_literal_names" | "default_nested" => true,
        _ => false,
    }
}
