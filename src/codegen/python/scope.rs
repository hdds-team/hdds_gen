// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL scoping for the Python and TypeScript backends.
//!
//! `qualify` returns a copy of the AST where every declared type carries a
//! KEY, its full scoped name with `.` between the segments (`a::b::T` ->
//! `a.b.T`; a type outside any module keeps its name), and every reference
//! to a type is resolved, by the IDL 4.2 scoping rules (7.5.2: the name is
//! looked up in the scope of the reference, then in each enclosing scope),
//! to the key of the type it names. Two types of the same name in two
//! modules thus stay two types for the shared `TypeIndex` (which looks names
//! up without their module qualifiers: a key has no `::`) and for the
//! generated code. A reference that names no declared type keeps its
//! segments, joined by `.`, so that it never resolves to an unrelated type
//! of the same unqualified name.
//!
//! The TypeScript backend uses this module too (one reading of IDL scopes
//! for both backends).

use std::collections::HashSet;

use crate::ast::{Definition, Field, IdlFile, Struct, Union};
use crate::types::IdlType;

/// Kind of a declared name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    /// A structure.
    Struct,
    /// A union.
    Union,
    /// An enumeration.
    Enum,
    /// A typedef.
    Typedef,
    /// A bitset.
    Bitset,
    /// A bitmask.
    Bitmask,
    /// A constant.
    Const,
}

/// A declared name: its key and kind.
#[derive(Clone, Debug)]
pub struct Decl {
    /// Full scoped name, segments joined by `.`.
    pub key: String,
    /// What it declares.
    pub kind: DeclKind,
}

/// The qualified AST and the declared names, in declaration order (a name
/// declared twice, as a forward declaration and its definition, once).
pub struct Scoped {
    /// The AST with keys for names and resolved references.
    pub ast: IdlFile,
    /// Every declared name.
    pub decls: Vec<Decl>,
}

/// Segments of a key.
pub fn segments(key: &str) -> Vec<&str> {
    key.split('.').collect()
}

/// The last segment of a key (the name as declared).
pub fn simple(key: &str) -> &str {
    key.rsplit('.').next().unwrap_or(key)
}

/// The IDL spelling of a key (`a.b.T` -> `a::b::T`).
pub fn idl_name(key: &str) -> String {
    key.replace('.', "::")
}

fn join(scope: &[String], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}.{name}", scope.join("."))
    }
}

/// Qualify `ast` (see the module documentation).
#[must_use]
pub fn qualify(ast: &IdlFile) -> Scoped {
    let mut decls = Vec::new();
    let mut seen = HashSet::new();
    let mut forward = HashSet::new();
    collect(
        &ast.definitions,
        &mut Vec::new(),
        &mut decls,
        &mut seen,
        &mut forward,
    );
    let mut types: HashSet<String> = decls
        .iter()
        .filter(|d| d.kind != DeclKind::Const)
        .map(|d| d.key.clone())
        .collect();
    types.extend(forward);
    let r = Resolver {
        types: &types,
        decls: crate::scope::Declarations::of(ast),
    };
    let mut out = ast.clone();
    out.definitions = r.defs(&ast.definitions, &mut Vec::new());
    Scoped { ast: out, decls }
}

fn collect(
    defs: &[Definition],
    scope: &mut Vec<String>,
    decls: &mut Vec<Decl>,
    seen: &mut HashSet<String>,
    forward: &mut HashSet<String>,
) {
    for def in defs {
        let (name, kind) = match def {
            Definition::Module(m) => {
                scope.push(m.name.clone());
                collect(&m.definitions, scope, decls, seen, forward);
                scope.pop();
                continue;
            }
            Definition::Struct(s) => (&s.name, DeclKind::Struct),
            Definition::Union(u) => (&u.name, DeclKind::Union),
            Definition::Enum(e) => (&e.name, DeclKind::Enum),
            Definition::Typedef(t) => (&t.name, DeclKind::Typedef),
            Definition::Bitset(b) => (&b.name, DeclKind::Bitset),
            Definition::Bitmask(b) => (&b.name, DeclKind::Bitmask),
            Definition::Const(c) => (&c.name, DeclKind::Const),
            Definition::ForwardDecl(f) => {
                // The definition that follows declares it; a forward
                // declaration alone still names a type references resolve
                // to.
                forward.insert(join(scope, &f.name));
                continue;
            }
            // Deliberately a catch-all, not an exhaustive variant list:
            // `Definition` grows feature-gated variants (Interface,
            // Exception, ...) that the Python backend does not declare.
            #[allow(clippy::match_wildcard_for_single_variants)]
            _ => continue,
        };
        let key = join(scope, name);
        if seen.insert(key.clone()) {
            decls.push(Decl { key, kind });
        }
    }
}

struct Resolver<'a> {
    types: &'a HashSet<String>,
    /// Every name the file declares, for the IDL 4.2 lookup.
    decls: crate::scope::Declarations,
}

impl Resolver<'_> {
    /// The key of the type `name` names from `scope`, or its segments
    /// joined by `.` when no declared type matches.
    fn name(&self, name: &str, scope: &[String]) -> String {
        if let Some(global) = self.decls.resolve_type(scope, name) {
            let key = global.replace("::", ".");
            if self.types.contains(&key) {
                return key;
            }
        }
        name.trim_start_matches("::")
            .split("::")
            .collect::<Vec<_>>()
            .join(".")
    }

    fn ty(&self, ty: &IdlType, scope: &[String]) -> IdlType {
        match ty {
            IdlType::Named(n) => IdlType::Named(self.name(n, scope)),
            IdlType::Sequence { inner, bound } => IdlType::Sequence {
                inner: Box::new(self.ty(inner, scope)),
                bound: *bound,
            },
            IdlType::Array { inner, size } => IdlType::Array {
                inner: Box::new(self.ty(inner, scope)),
                size: *size,
            },
            IdlType::Map { key, value, bound } => IdlType::Map {
                key: Box::new(self.ty(key, scope)),
                value: Box::new(self.ty(value, scope)),
                bound: *bound,
            },
            IdlType::Primitive(_) | IdlType::BoundedString { .. } => ty.clone(),
        }
    }

    fn field(&self, f: &Field, scope: &[String]) -> Field {
        let mut f = f.clone();
        f.field_type = self.ty(&f.field_type, scope);
        f
    }

    fn structure(&self, s: &Struct, scope: &[String]) -> Struct {
        let mut s = s.clone();
        s.base_struct = s.base_struct.as_ref().map(|b| self.name(b, scope));
        s.fields = s.fields.iter().map(|f| self.field(f, scope)).collect();
        s.name = join(scope, &s.name);
        s
    }

    fn union(&self, u: &Union, scope: &[String]) -> Union {
        let mut u = u.clone();
        u.discriminator = self.ty(&u.discriminator, scope);
        for c in &mut u.cases {
            c.field = self.field(&c.field, scope);
        }
        u.name = join(scope, &u.name);
        u
    }

    fn defs(&self, defs: &[Definition], scope: &mut Vec<String>) -> Vec<Definition> {
        defs.iter()
            .map(|def| match def {
                Definition::Module(m) => {
                    let mut m = m.clone();
                    scope.push(m.name.clone());
                    m.definitions = self.defs(&m.definitions, scope);
                    scope.pop();
                    Definition::Module(m)
                }
                Definition::Struct(s) => Definition::Struct(self.structure(s, scope)),
                Definition::Union(u) => Definition::Union(self.union(u, scope)),
                Definition::Typedef(t) => {
                    let mut t = t.clone();
                    t.base_type = self.ty(&t.base_type, scope);
                    t.name = join(scope, &t.name);
                    Definition::Typedef(t)
                }
                Definition::Enum(e) => {
                    let mut e = e.clone();
                    e.name = join(scope, &e.name);
                    Definition::Enum(e)
                }
                Definition::Bitset(b) => {
                    let mut b = b.clone();
                    b.name = join(scope, &b.name);
                    Definition::Bitset(b)
                }
                Definition::Bitmask(b) => {
                    let mut b = b.clone();
                    b.name = join(scope, &b.name);
                    Definition::Bitmask(b)
                }
                Definition::Const(c) => {
                    let mut c = c.clone();
                    c.const_type = self.ty(&c.const_type, scope);
                    c.name = join(scope, &c.name);
                    Definition::Const(c)
                }
                Definition::ForwardDecl(f) => {
                    let mut f = f.clone();
                    f.name = join(scope, &f.name);
                    Definition::ForwardDecl(f)
                }
                // Deliberately a catch-all: see the matching comment above.
                #[allow(clippy::match_wildcard_for_single_variants)]
                other => other.clone(),
            })
            .collect()
    }
}

/// The flat identifier of every key (`a.b.T` -> `a_b_T`), or an error when
/// two keys flatten to the same identifier (a type `a_T` beside a type `T`
/// in a module `a`).
///
/// # Errors
///
/// Returns the two colliding IDL names.
pub fn check_flat_names(decls: &[Decl]) -> std::result::Result<(), String> {
    let mut by_flat = std::collections::HashMap::new();
    for d in decls {
        let flat = flat(&d.key);
        if let Some(other) = by_flat.insert(flat.clone(), d.key.clone()) {
            if other != d.key {
                return Err(format!(
                    "`{}` and `{}` would both be named `{flat}` in the generated code",
                    idl_name(&other),
                    idl_name(&d.key)
                ));
            }
        }
    }
    Ok(())
}

/// The flat identifier of a key.
pub fn flat(key: &str) -> String {
    key.replace('.', "_")
}

/// A module of the file and what it declares, for the namespace views.
#[derive(Default, Debug)]
pub struct ModuleTree {
    /// Declarations directly in this module: (simple name, key, kind).
    pub items: Vec<(String, String, DeclKind)>,
    /// Sub-modules, in order of first appearance.
    pub modules: Vec<(String, Self)>,
}

impl ModuleTree {
    /// The module tree of the declarations (top-level declarations are not
    /// in any module and are left out).
    #[must_use]
    pub fn new(decls: &[Decl]) -> Self {
        let mut root = Self::default();
        for d in decls {
            let segs = segments(&d.key);
            if segs.len() < 2 {
                continue;
            }
            let mut node = &mut root;
            for m in &segs[..segs.len() - 1] {
                let pos = node.modules.iter().position(|(n, _)| n == m);
                let pos = pos.unwrap_or_else(|| {
                    node.modules.push(((*m).to_string(), Self::default()));
                    node.modules.len() - 1
                });
                node = &mut node.modules[pos].1;
            }
            node.items
                .push((simple(&d.key).to_string(), d.key.clone(), d.kind));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn scoped(idl: &str) -> Scoped {
        let ast = Parser::try_new(idl)
            .and_then(|mut p| p.parse())
            .unwrap_or_default();
        qualify(&ast)
    }

    fn field_types(s: &Scoped, key: &str) -> Vec<String> {
        fn find<'a>(defs: &'a [Definition], key: &str) -> Option<&'a Struct> {
            defs.iter().find_map(|d| match d {
                Definition::Struct(s) if s.name == key => Some(s),
                Definition::Module(m) => find(&m.definitions, key),
                _ => None,
            })
        }
        find(&s.ast.definitions, key)
            .map(|st| {
                st.fields
                    .iter()
                    .map(|f| match &f.field_type {
                        IdlType::Named(n) => n.clone(),
                        other => other.to_idl_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// IDL 4.2 7.5.2: a name is looked up in the scope of the reference,
    /// then in each enclosing scope. (The parser drops a leading `::` of a
    /// type name, so `::T` is looked up as `T`.)
    #[test]
    fn references_resolve_by_the_idl_scoping_rules() {
        let s = scoped(
            "struct T { long v; };\n\
             module a { struct T { octet x; }; };\n\
             module b { struct T { string s; };\n\
               module c { struct U { a::T t; T bt; b::T qb; }; }; };\n\
             struct H { a::T at; b::T bt; b::c::U u; T t; ::a::T abs; x::Y missing; };",
        );
        assert_eq!(field_types(&s, "b.c.U"), ["a.T", "b.T", "b.T"]);
        assert_eq!(
            field_types(&s, "H"),
            ["a.T", "b.T", "b.c.U", "T", "a.T", "x.Y"]
        );
        let keys: Vec<&str> = s.decls.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, ["T", "a.T", "b.T", "b.c.U", "H"]);
    }

    #[test]
    fn flat_names_that_collide_are_refused() {
        let s = scoped("module a { struct T { long v; }; }; struct a_T { long w; };");
        assert!(check_flat_names(&s.decls).is_err());
        let ok = scoped("module a { struct T { long v; }; }; module b { struct T { long w; }; };");
        assert!(check_flat_names(&ok.decls).is_ok());
    }

    #[test]
    fn module_tree_groups_reopened_modules() {
        let s = scoped(
            "module a { struct X { long v; }; };\n\
             module b { struct Y { long v; }; };\n\
             module a { module n { struct Z { long v; }; }; struct W { long v; }; };",
        );
        let t = ModuleTree::new(&s.decls);
        let names: Vec<&str> = t.modules.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        let a = &t.modules[0].1;
        let items: Vec<&str> = a.items.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(items, ["X", "W"]);
        assert_eq!(a.modules[0].0, "n");
    }
}
