// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Struct validation rules.
//!
//! Validates struct definitions for semantic correctness.

#![allow(clippy::redundant_pub_crate)]

use super::super::diagnostics::{Level, ValidationDiag};
use super::helpers::{
    enforce_extensibility_conflicts, validate_custom_annotations, validate_field_annotations,
    validate_type_usage, validate_unique_member_names,
};
use crate::ast::{AnnotationDecl, Struct};
use crate::types::{Annotation, AutoIdKind, IdlType};
use std::collections::{HashMap, HashSet};

pub(crate) fn validate_struct(
    s: &Struct,
    typedefs: &HashMap<String, IdlType>,
    ann_index: &HashMap<String, AnnotationDecl>,
    diags: &mut Vec<ValidationDiag>,
) {
    validate_custom_annotations(
        &s.annotations,
        ann_index,
        &format!("struct {}", s.name),
        diags,
    );
    enforce_extensibility_conflicts(&s.annotations, &format!("struct {}", s.name), diags);
    validate_unique_member_names(
        &format!("struct {}", s.name),
        s.fields.iter().map(|f| f.name.as_str()),
        diags,
    );

    let mut seen_ids: HashSet<u32> = HashSet::new();
    let mut explicit_ids_in_order: Vec<(usize, u32)> = Vec::new();
    for (idx, f) in s.fields.iter().enumerate() {
        validate_type_usage(
            &f.field_type,
            &format!("{}.{}", s.name, f.name),
            diags,
            typedefs,
        );
        for ann in &f.annotations {
            if let Annotation::Id(id) = ann {
                if !seen_ids.insert(*id) {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!("{}.{}: duplicate @id({}) in struct", s.name, f.name, id),
                    });
                }
                explicit_ids_in_order.push((idx, *id));
            }
        }
        validate_field_annotations(&s.name, f, diags);
        validate_custom_annotations(
            &f.annotations,
            ann_index,
            &format!("struct {}.{}", s.name, f.name),
            diags,
        );
    }

    let autoid_seq = s
        .annotations
        .iter()
        .any(|a| matches!(a, Annotation::AutoId(AutoIdKind::Sequential)));
    if autoid_seq {
        let mut last: Option<u32> = None;
        for (_idx, id) in explicit_ids_in_order {
            if let Some(prev) = last {
                if id <= prev {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "struct {}: @autoid(SEQUENTIAL) violated: ids not increasing ({} then {})",
                            s.name, prev, id
                        ),
                    });
                    break;
                }
            }
            last = Some(id);
        }
    }
}
