// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Union validation rules.
//!
//! Validates discriminated union definitions for case coverage and type correctness.

#![allow(clippy::redundant_pub_crate)]

use super::super::diagnostics::{Level, ValidationDiag};
use super::helpers::{
    enforce_extensibility_conflicts, validate_custom_annotations, validate_field_annotations,
    validate_type_usage, validate_unique_member_names,
};
use crate::ast::{AnnotationDecl, Union, UnionLabel};
use crate::types::{Annotation, AutoIdKind, IdlType};
use std::collections::{HashMap, HashSet};

pub(crate) fn validate_union(
    u: &Union,
    typedefs: &HashMap<String, IdlType>,
    ann_index: &HashMap<String, AnnotationDecl>,
    diags: &mut Vec<ValidationDiag>,
) {
    validate_custom_annotations(
        &u.annotations,
        ann_index,
        &format!("union {}", u.name),
        diags,
    );
    enforce_extensibility_conflicts(&u.annotations, &format!("union {}", u.name), diags);
    validate_unique_member_names(
        &format!("union {}", u.name),
        u.cases.iter().map(|c| c.field.name.as_str()),
        diags,
    );

    let mut seen_ids: HashSet<u32> = HashSet::new();
    let mut explicit_ids_in_order: Vec<(usize, u32)> = Vec::new();
    let mut seen_default = false;
    let mut seen_labels: HashSet<String> = HashSet::new();

    for (idx, c) in u.cases.iter().enumerate() {
        if c.labels.iter().any(|l| matches!(l, UnionLabel::Default)) {
            if seen_default {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!("union {}: multiple default labels", u.name),
                });
            }
            seen_default = true;
        }

        for ann in &c.field.annotations {
            if let Annotation::Id(id) = ann {
                if !seen_ids.insert(*id) {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!(
                            "union {}.{}: duplicate @id({})",
                            u.name, c.field.name, id
                        ),
                    });
                }
                explicit_ids_in_order.push((idx, *id));
            }
        }

        let has_default_ann = c
            .field
            .annotations
            .iter()
            .any(|a| matches!(a, Annotation::Default | Annotation::DefaultLiteral));
        if has_default_ann {
            if seen_default {
                diags.push(ValidationDiag {
                    level: Level::Error,
                    message: format!("union {}: multiple default labels", u.name),
                });
            }
            seen_default = true;
        }

        validate_custom_annotations(
            &c.field.annotations,
            ann_index,
            &format!("union {}.{}", u.name, c.field.name),
            diags,
        );

        for lbl in &c.labels {
            if let UnionLabel::Value(v) = lbl {
                if !seen_labels.insert(v.clone()) {
                    diags.push(ValidationDiag {
                        level: Level::Error,
                        message: format!("union {}: duplicate case label '{}'", u.name, v),
                    });
                }
            }
        }

        validate_type_usage(
            &c.field.field_type,
            &format!("union {}.{}", u.name, c.field.name),
            diags,
            typedefs,
        );
        validate_field_annotations(&u.name, &c.field, diags);
    }

    let autoid_seq = u
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
                            "union {}: @autoid(SEQUENTIAL) violated: ids not increasing ({} then {})",
                            u.name, prev, id
                        ),
                });
                    break;
                }
            }
            last = Some(id);
        }
    }
}
