// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Enum validation rules.
//!
//! Validates enum definitions for duplicate variants and annotation usage.

#![allow(clippy::redundant_pub_crate)]

use super::super::diagnostics::{Level, ValidationDiag};
use super::helpers::validate_custom_annotations;
use crate::ast::{AnnotationDecl, Enum};
use crate::codegen::data_model::enum_width;
use crate::types::Annotation;
use std::collections::{HashMap, HashSet};

pub(crate) fn validate_enum(
    e: &Enum,
    ann_index: &HashMap<String, AnnotationDecl>,
    diags: &mut Vec<ValidationDiag>,
) {
    validate_custom_annotations(
        &e.annotations,
        ann_index,
        &format!("enum {}", e.name),
        diags,
    );

    // Validate annotation placement: @external and @non_serialized are invalid on enums
    for ann in &e.annotations {
        match ann {
            Annotation::External => {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!(
                        "enum {}: @external is invalid on enums (valid on struct/union types and members)",
                        e.name
                    ),
                });
            }
            Annotation::NonSerialized => {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!(
                        "enum {}: @non_serialized is invalid on enums (valid on members only)",
                        e.name
                    ),
                });
            }
            _ => {}
        }
    }

    if e.variants.is_empty() {
        diags.push(ValidationDiag {
            level: Level::Error,
            message: format!("enum {}: must declare at least one enumerator", e.name),
        });
        return;
    }

    let mut seen: HashSet<String> = HashSet::new();
    for v in &e.variants {
        if !seen.insert(v.name.clone()) {
            diags.push(ValidationDiag {
                level: Level::Error,
                message: format!("enum {}: duplicate enumerator '{}'", e.name, v.name),
            });
        }
    }
    validate_values(e, diags);
}

/// The enumerator values (DDS-XTypes 1.3, 7.2.2.4.1.1 Table 4: each
/// enumerator has an Int32 value, unique within the type; 7.4.1.1.3.1
/// Table 32: the value is serialized as the Int8, Int16 or Int32 that
/// `@bit_bound` selects).
///
/// An enumerator with no value takes the previous one plus one, so a value
/// that moves the counter can make a later enumerator collide with an
/// earlier one, or run past the range.
fn validate_values(e: &Enum, diags: &mut Vec<ValidationDiag>) {
    let bit_bound = e.annotations.iter().find_map(|a| match a {
        Annotation::BitBound(n) => Some(*n),
        _ => None,
    });
    let (holder, min, max): (&str, i64, i64) = match enum_width(bit_bound) {
        1 => ("Int8", i64::from(i8::MIN), i64::from(i8::MAX)),
        2 => ("Int16", i64::from(i16::MIN), i64::from(i16::MAX)),
        _ => ("Int32", i64::from(i32::MIN), i64::from(i32::MAX)),
    };
    let mut seen: HashMap<i64, &str> = HashMap::new();
    for (v, value) in e.variants.iter().zip(e.resolved_values()) {
        if value < min || value > max {
            let bound = bit_bound.map_or_else(String::new, |n| format!(" (@bit_bound({n}))"));
            diags.push(ValidationDiag {
                level: Level::Error,
                message: format!(
                    "enum {}: enumerator '{}' has value {value}, outside the range {min}..{max} of the {holder} it is serialized as{bound}",
                    e.name, v.name
                ),
            });
        }
        if let Some(first) = seen.insert(value, &v.name) {
            // A repeated name is reported above, with its own message.
            if first != v.name {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!(
                        "enum {}: enumerators '{first}' and '{}' have the same value {value} (the values of an enumerated type are unique)",
                        e.name, v.name
                    ),
                });
            }
        }
    }
}
