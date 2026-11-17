// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Validation engine for semantic analysis.
//!
//! Orchestrates validation passes across all AST definitions.

use crate::ast::{AnnotationDecl, Definition, IdlFile};
use crate::types::IdlType;
use std::collections::HashMap;

#[cfg(feature = "interfaces")]
use super::collect::collect_exceptions;
use super::collect::{collect_annotation_decls, collect_typedefs};
use super::diagnostics::ValidationDiag;
use super::references::validate_references;
#[cfg(feature = "interfaces")]
use super::rules::validate_interface;
use super::rules::{
    validate_bitmask, validate_bitset, validate_enum, validate_struct, validate_typedef,
    validate_union,
};

#[must_use]
/// Run semantic validation checks and return the collected diagnostics.
pub fn validate(ast: &IdlFile) -> Vec<ValidationDiag> {
    let mut diags = Vec::new();

    // Collect declared custom annotations
    let annotations_index = collect_annotation_decls(ast);
    #[cfg(feature = "interfaces")]
    let exceptions_index = collect_exceptions(ast);

    // First, validate references (forward decl resolution / unknown types)
    validate_references(ast, &mut diags);
    super::collisions::validate_collisions(ast, &mut diags);

    // Build typedef map once (recursive)
    let mut typedefs: HashMap<String, IdlType> = HashMap::new();
    collect_typedefs(ast, "", &mut typedefs);

    validate_inner(
        ast,
        &typedefs,
        &annotations_index,
        #[cfg(feature = "interfaces")]
        &exceptions_index,
        &mut diags,
    );

    diags
}

pub(super) fn validate_inner(
    file: &IdlFile,
    typedefs: &HashMap<String, IdlType>,
    annotations_index: &HashMap<String, AnnotationDecl>,
    #[cfg(feature = "interfaces")] exceptions_index: &std::collections::HashSet<String>,
    diags: &mut Vec<ValidationDiag>,
) {
    for def in &file.definitions {
        match def {
            Definition::Bitmask(b) => validate_bitmask(b, annotations_index, diags),
            Definition::Bitset(b) => validate_bitset(b, annotations_index, diags),
            Definition::Enum(e) => validate_enum(e, annotations_index, diags),
            Definition::Struct(s) => validate_struct(s, typedefs, annotations_index, diags),
            Definition::Typedef(t) => validate_typedef(t, annotations_index, diags),
            Definition::Union(u) => validate_union(u, typedefs, annotations_index, diags),
            #[cfg(feature = "interfaces")]
            Definition::Interface(i) => validate_interface(i, typedefs, exceptions_index, diags),
            #[cfg(feature = "interfaces")]
            Definition::Exception(_) => {}
            Definition::AnnotationDecl(_) => {}
            Definition::Module(m) => {
                let sub = IdlFile {
                    definitions: m.definitions.clone(),
                };
                validate_inner(
                    &sub,
                    typedefs,
                    annotations_index,
                    #[cfg(feature = "interfaces")]
                    exceptions_index,
                    diags,
                );
            }
            _ => {}
        }
    }
}
