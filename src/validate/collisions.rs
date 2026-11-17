// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Identifier collisions (IDL 4.2 7.2.3.1, 7.5.2).
//!
//! One scope has one namespace: a module, a type, a constant and an
//! enumerator (declared in the scope enclosing its enumeration) may not
//! share a name, compared case-insensitively. A module may be reopened, and
//! a forward declaration is followed by its definition, both with the same
//! spelling. A structure or union member may not be named like its type
//! (the type's name may not be redefined in its immediate scope), and the
//! bitfields of a bitset may not collide. A member that differs from its
//! type only by case is a warning, not an error: ROS 2 messages use it
//! (`unique_identifier_msgs::msg::UUID` has a member `uuid`), and that
//! structure generates for every backend (`tests/member_case_collision.rs`).
//! Identical spelling stays an error: a member named like its class is a
//! constructor clash in the C++ backend.

use std::collections::HashMap;

use super::diagnostics::{Level, ValidationDiag};
use crate::ast::{Definition, IdlFile};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Module,
    Forward,
    Type,
    Const,
    Enumerator,
}

pub(super) fn validate_collisions(ast: &IdlFile, diags: &mut Vec<ValidationDiag>) {
    let mut scopes: HashMap<String, Vec<(String, Kind)>> = HashMap::new();
    collect(&ast.definitions, &mut Vec::new(), &mut scopes, diags);
    let mut keys: Vec<&String> = scopes.keys().collect();
    keys.sort();
    for scope in keys {
        let mut seen: HashMap<String, (&str, Kind)> = HashMap::new();
        for (name, kind) in &scopes[scope] {
            let folded = name.to_ascii_lowercase();
            match seen.get(&folded) {
                None => {
                    seen.insert(folded, (name, *kind));
                }
                Some(&(first, first_kind)) => {
                    let allowed = first == name
                        && matches!(
                            (first_kind, *kind),
                            (Kind::Module, Kind::Module)
                                | (Kind::Forward, Kind::Forward | Kind::Type)
                                | (Kind::Type, Kind::Forward)
                        );
                    if !allowed {
                        let at = if scope.is_empty() {
                            "the file scope".to_string()
                        } else {
                            format!("module {scope}")
                        };
                        diags.push(ValidationDiag {
                            level: Level::Error,
                            message: format!(
                                "{at}: '{name}' collides with '{first}' declared in the same scope"
                            ),
                        });
                    }
                }
            }
        }
    }
}

fn collect(
    defs: &[Definition],
    scope: &mut Vec<String>,
    scopes: &mut HashMap<String, Vec<(String, Kind)>>,
    diags: &mut Vec<ValidationDiag>,
) {
    let key = scope.join("::");
    for def in defs {
        let entry = scopes.entry(key.clone()).or_default();
        match def {
            Definition::Module(m) => {
                entry.push((m.name.clone(), Kind::Module));
                scope.push(m.name.clone());
                collect(&m.definitions, scope, scopes, diags);
                scope.pop();
            }
            Definition::ForwardDecl(f) => entry.push((f.name.clone(), Kind::Forward)),
            Definition::Const(c) => entry.push((c.name.clone(), Kind::Const)),
            Definition::Enum(e) => {
                entry.push((e.name.clone(), Kind::Type));
                for v in &e.variants {
                    entry.push((v.name.clone(), Kind::Enumerator));
                }
            }
            Definition::Struct(s) => {
                entry.push((s.name.clone(), Kind::Type));
                for f in &s.fields {
                    member_named_like_type("struct", &s.name, &f.name, diags);
                }
            }
            Definition::Union(u) => {
                entry.push((u.name.clone(), Kind::Type));
                for c in &u.cases {
                    member_named_like_type("union", &u.name, &c.field.name, diags);
                }
            }
            Definition::Bitset(b) => {
                entry.push((b.name.clone(), Kind::Type));
                let mut seen: HashMap<String, &str> = HashMap::new();
                for f in &b.fields {
                    if let Some(first) = seen.insert(f.name.to_ascii_lowercase(), &f.name) {
                        diags.push(ValidationDiag {
                            level: Level::Error,
                            message: format!(
                                "bitset {}: bitfield '{}' collides with '{first}'",
                                b.name, f.name
                            ),
                        });
                    }
                }
            }
            other => {
                if let Some((name, _)) = crate::scope::declared_name(other) {
                    entry.push((name.to_string(), Kind::Type));
                }
            }
        }
    }
}

fn member_named_like_type(what: &str, ty: &str, member: &str, diags: &mut Vec<ValidationDiag>) {
    if member == ty {
        diags.push(ValidationDiag {
            level: Level::Error,
            message: format!(
                "{what} {ty}: member '{member}' is named like its {what} (IDL 4.2 7.5.2)"
            ),
        });
    } else if member.eq_ignore_ascii_case(ty) {
        diags.push(ValidationDiag {
            level: Level::Warning,
            message: format!(
                "{what} {ty}: member '{member}' differs from its {what} only by case \
                 (IDL 4.2 7.5.2): accepted for ROS 2 compatibility"
            ),
        });
    }
}
