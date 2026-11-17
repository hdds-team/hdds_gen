// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Module scoping of the C and C-micro backends.
//!
//! C has a single namespace: a type declared in a module gets its full
//! module path as a prefix (`module a { module inner { struct T; }; };` is
//! the C type `a_inner_T`; a type outside any module keeps its name), and
//! every reference to a type is resolved with the IDL 4.2 scoping rules
//! (7.5: an absolute name `::a::T`, else the name looked up from the
//! innermost enclosing scope outwards) and replaced by that C name. The
//! backends then work on an AST whose type names are unique, so two
//! modules each declaring `struct T` give two types, never one mixing
//! both. Two paths giving the same C name (`a_b::T` and `a::b_T`) are
//! refused.

use std::collections::HashMap;

use crate::ast::{Definition, IdlFile};
use crate::types::IdlType;

/// An AST whose type names are their C names, and the IDL qualified name
/// (`a::inner::T`) of each.
pub struct Scoped {
    /// The rewritten AST (modules kept, so module annotations still apply).
    pub ast: IdlFile,
    /// C name to IDL qualified name.
    pub idl_names: HashMap<String, String>,
}

/// The C name of the type `name` declared in the modules `scope`.
fn c_name(scope: &[String], name: &str) -> String {
    let mut out = String::new();
    for m in scope {
        out.push_str(m);
        out.push('_');
    }
    out.push_str(name);
    out
}

fn type_name(def: &Definition) -> Option<&str> {
    match def {
        Definition::Struct(s) => Some(&s.name),
        Definition::Union(u) => Some(&u.name),
        Definition::Enum(e) => Some(&e.name),
        Definition::Typedef(t) => Some(&t.name),
        Definition::Bitset(b) => Some(&b.name),
        Definition::Bitmask(b) => Some(&b.name),
        Definition::ForwardDecl(f) => Some(&f.name),
        _ => None,
    }
}

/// Every type's IDL qualified name (`a::b::T`) and C name.
fn collect(defs: &[Definition], scope: &mut Vec<String>, qualified: &mut HashMap<String, String>) {
    for def in defs {
        if let Definition::Module(m) = def {
            scope.push(m.name.clone());
            collect(&m.definitions, scope, qualified);
            scope.pop();
        } else if let Some(name) = type_name(def) {
            let mut q = scope.join("::");
            if !q.is_empty() {
                q.push_str("::");
            }
            q.push_str(name);
            qualified.insert(q, c_name(scope, name));
        }
    }
}

struct Resolver {
    /// IDL qualified name to C name.
    qualified: HashMap<String, String>,
    /// Every name the file declares, for the IDL 4.2 lookup.
    decls: crate::scope::Declarations,
}

impl Resolver {
    /// The C name of the type `name` referenced from the modules `scope`
    /// ([`crate::scope`], IDL 4.2 7.5), or `name` itself when it names no
    /// declared type.
    fn resolve(&self, scope: &[String], name: &str) -> String {
        self.decls
            .resolve_type(scope, name)
            .and_then(|g| self.qualified.get(&g).cloned())
            .unwrap_or_else(|| name.to_string())
    }

    fn ty(&self, scope: &[String], t: &IdlType) -> IdlType {
        match t {
            IdlType::Named(n) => IdlType::Named(self.resolve(scope, n)),
            IdlType::Sequence { inner, bound } => IdlType::Sequence {
                inner: Box::new(self.ty(scope, inner)),
                bound: *bound,
            },
            IdlType::Array { inner, size } => IdlType::Array {
                inner: Box::new(self.ty(scope, inner)),
                size: *size,
            },
            IdlType::Map { key, value, bound } => IdlType::Map {
                key: Box::new(self.ty(scope, key)),
                value: Box::new(self.ty(scope, value)),
                bound: *bound,
            },
            other => other.clone(),
        }
    }

    fn defs(&self, defs: &[Definition], scope: &mut Vec<String>) -> Vec<Definition> {
        let mut out = Vec::with_capacity(defs.len());
        for def in defs {
            let mut d = def.clone();
            match &mut d {
                Definition::Module(m) => {
                    scope.push(m.name.clone());
                    m.definitions = self.defs(&m.definitions, scope);
                    scope.pop();
                }
                Definition::Struct(s) => {
                    s.name = c_name(scope, &s.name);
                    if let Some(b) = &mut s.base_struct {
                        *b = self.resolve(scope, b);
                    }
                    for f in &mut s.fields {
                        f.field_type = self.ty(scope, &f.field_type);
                    }
                }
                Definition::Union(u) => {
                    u.name = c_name(scope, &u.name);
                    u.discriminator = self.ty(scope, &u.discriminator);
                    for c in &mut u.cases {
                        c.field.field_type = self.ty(scope, &c.field.field_type);
                    }
                }
                Definition::Typedef(t) => {
                    t.name = c_name(scope, &t.name);
                    t.base_type = self.ty(scope, &t.base_type);
                }
                Definition::Enum(e) => e.name = c_name(scope, &e.name),
                Definition::Bitset(b) => b.name = c_name(scope, &b.name),
                Definition::Bitmask(b) => b.name = c_name(scope, &b.name),
                Definition::ForwardDecl(f) => f.name = c_name(scope, &f.name),
                Definition::Const(c) => c.name = c_name(scope, &c.name),
                // Deliberately a catch-all, not an exhaustive variant list:
                // `Definition` grows feature-gated variants (Interface,
                // Exception, ...) that the C backend does not scope, and a
                // wildcard here keeps that refusal implicit rather than
                // demanding a dead arm per future variant.
                #[allow(clippy::match_wildcard_for_single_variants)]
                _ => {}
            }
            out.push(d);
        }
        out
    }
}

/// `ast` with every type named by its C name and every reference resolved.
///
/// # Errors
///
/// Returns the two IDL names when two types get the same C name.
pub fn scope(ast: &IdlFile) -> Result<Scoped, String> {
    let mut qualified = HashMap::new();
    collect(&ast.definitions, &mut Vec::new(), &mut qualified);
    let mut idl_names: HashMap<String, String> = HashMap::new();
    let mut sorted: Vec<(&String, &String)> = qualified.iter().collect();
    sorted.sort();
    for (q, c) in sorted {
        if let Some(other) = idl_names.insert(c.clone(), q.clone()) {
            return Err(format!(
                "`{other}` and `{q}` both map to the C name `{c}`; rename one of them"
            ));
        }
    }
    let resolver = Resolver {
        qualified,
        decls: crate::scope::Declarations::of(ast),
    };
    let definitions = resolver.defs(&ast.definitions, &mut Vec::new());
    Ok(Scoped {
        ast: IdlFile { definitions },
        idl_names,
    })
}

/// The IDL text of `t` with the C names of its named types replaced by
/// their IDL qualified names.
#[must_use]
pub fn idl_string(t: &IdlType, idl_names: &HashMap<String, String>) -> String {
    fn back(t: &IdlType, map: &HashMap<String, String>) -> IdlType {
        match t {
            IdlType::Named(n) => IdlType::Named(map.get(n).cloned().unwrap_or_else(|| n.clone())),
            IdlType::Sequence { inner, bound } => IdlType::Sequence {
                inner: Box::new(back(inner, map)),
                bound: *bound,
            },
            IdlType::Array { inner, size } => IdlType::Array {
                inner: Box::new(back(inner, map)),
                size: *size,
            },
            IdlType::Map { key, value, bound } => IdlType::Map {
                key: Box::new(back(key, map)),
                value: Box::new(back(value, map)),
                bound: *bound,
            },
            other => other.clone(),
        }
    }
    back(t, idl_names).to_idl_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Parser;

    fn field_type(ast: &IdlFile, path: &[&str], s: &str, f: usize) -> IdlType {
        let mut defs = &ast.definitions;
        for m in path {
            defs = defs
                .iter()
                .find_map(|d| match d {
                    Definition::Module(x) if x.name == *m => Some(&x.definitions),
                    _ => None,
                })
                .unwrap_or_else(|| unreachable!("module {m}"));
        }
        defs.iter()
            .find_map(|d| match d {
                Definition::Struct(x) if x.name == s => Some(x.fields[f].field_type.clone()),
                _ => None,
            })
            .unwrap_or_else(|| unreachable!("struct {s}"))
    }

    /// An absolute name (`::T`) names the top-level type even inside a
    /// module that declares its own `T`; a relative one the innermost.
    #[test]
    fn absolute_and_relative_names() {
        let idl =
            "struct T { long w; }; module b { struct T { short p; }; struct U { ::T a; T b; }; };";
        let ast = Parser::try_new(idl)
            .and_then(|mut p| p.parse())
            .unwrap_or_else(|e| unreachable!("{e}"));
        let scoped = scope(&ast).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            field_type(&scoped.ast, &["b"], "b_U", 0),
            IdlType::Named("T".into())
        );
        assert_eq!(
            field_type(&scoped.ast, &["b"], "b_U", 1),
            IdlType::Named("b_T".into())
        );
        assert_eq!(
            scoped.idl_names.get("b_T").map(String::as_str),
            Some("b::T")
        );
    }
}
