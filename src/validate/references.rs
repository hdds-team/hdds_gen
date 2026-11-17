// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Type reference validation.
//!
//! Every type reference must resolve, by the IDL 4.2 scoping rules (7.5,
//! [`crate::scope`]) from the modules enclosing it, to a declared type.

use std::collections::{BTreeSet, HashMap};

use super::diagnostics::{Level, ValidationDiag};
use crate::ast::{Definition, IdlFile};
use crate::scope::{for_each_reference_mut, join, DeclKind, Declarations};
use crate::types::IdlType;

pub(super) fn validate_references(ast: &IdlFile, diags: &mut Vec<ValidationDiag>) {
    let decls = Declarations::of(ast);
    validate_typedef_cycles(ast, &decls, diags);
    let mut unresolved: BTreeSet<String> = BTreeSet::new();
    walk(&ast.definitions, &mut Vec::new(), &decls, &mut unresolved);
    for name in unresolved {
        // A name declared elsewhere but not visible from the reference:
        // say where, so the fix (qualifying it) is plain.
        let last = name.rsplit("::").next().unwrap_or(&name);
        let mut elsewhere: Vec<&str> = decls
            .names_of(DeclKind::Type)
            .filter(|g| g.rsplit("::").next() == Some(last))
            .collect();
        elsewhere.sort_unstable();
        let message = if elsewhere.is_empty() {
            format!("Unresolved type reference: {name}")
        } else {
            format!(
                "Unresolved type reference: {name} (not visible from its scope; declared as {})",
                elsewhere.join(", ")
            )
        };
        diags.push(ValidationDiag {
            level: Level::Error,
            message,
        });
    }
}

fn walk(
    defs: &[Definition],
    scope: &mut Vec<String>,
    decls: &Declarations,
    unresolved: &mut BTreeSet<String>,
) {
    for d in defs {
        if let Definition::Module(m) = d {
            scope.push(m.name.clone());
            walk(&m.definitions, scope, decls, unresolved);
            scope.pop();
            continue;
        }
        let mut d = d.clone();
        for_each_reference_mut(&mut d, &mut |name: &mut String| {
            if decls.resolve_type(scope, name).is_none() {
                unresolved.insert(name.clone());
            }
        });
    }
}

/// A typedef whose chain of aliases comes back to itself (`typedef X X;`,
/// `typedef Y X; typedef X Y;`) names no type: an error, reported once per
/// cycle member.
fn validate_typedef_cycles(ast: &IdlFile, decls: &Declarations, diags: &mut Vec<ValidationDiag>) {
    fn collect(
        defs: &[Definition],
        scope: &mut Vec<String>,
        out: &mut HashMap<String, (Vec<String>, IdlType)>,
    ) {
        for d in defs {
            match d {
                Definition::Module(m) => {
                    scope.push(m.name.clone());
                    collect(&m.definitions, scope, out);
                    scope.pop();
                }
                Definition::Typedef(t) => {
                    out.insert(join(scope, &t.name), (scope.clone(), t.base_type.clone()));
                }
                _ => {}
            }
        }
    }
    let mut aliases = HashMap::new();
    collect(&ast.definitions, &mut Vec::new(), &mut aliases);
    let mut cyclic: BTreeSet<String> = BTreeSet::new();
    for start in aliases.keys() {
        let mut seen: Vec<&str> = vec![start.as_str()];
        let mut current = start.as_str();
        while let Some((scope, IdlType::Named(next))) = aliases.get(current) {
            let Some(global) = decls.resolve_type(scope, next) else {
                break;
            };
            let Some((key, _)) = aliases.get_key_value(global.as_str()) else {
                break;
            };
            if key == start {
                cyclic.insert(start.clone());
                break;
            }
            if seen.contains(&key.as_str()) {
                break;
            }
            seen.push(key);
            current = key;
        }
    }
    for name in cyclic {
        diags.push(ValidationDiag {
            level: Level::Error,
            message: format!("typedef {name}: its alias chain names itself (no type)"),
        });
    }
}
