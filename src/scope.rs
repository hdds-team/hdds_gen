// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL 4.2 name resolution (7.5), the one reading of IDL scopes shared by
//! the parser (constants, declaration order), the validator and every
//! backend.
//!
//! A name is resolved from the scope of the reference (the modules
//! enclosing it, outermost first):
//!
//! - `::a::T` (absolute) is looked up from the file scope only;
//! - `T` is looked up in the scope of the reference, then in each enclosing
//!   scope outwards; the innermost scope that declares `T` wins (7.5.2);
//! - `a::T` (qualified) resolves its FIRST segment `a` that way, then
//!   looks `T` up in the scope `a` found, and nowhere else (7.5.1): when the
//!   innermost `a` lacks `T`, the name does not resolve, even if an outer
//!   `a` declares one.
//!
//! Enumerators are declared in the scope enclosing their enumeration
//! (7.5.2, "Enumeration value names are introduced into the enclosing
//! scope"). A resolved name is its global name without the leading `::`
//! (`a::b::T`, `T` for a type at file scope).

use std::collections::HashMap;

use crate::ast::{Definition, IdlFile, Struct, Union};
use crate::types::{Annotation, IdlType};

/// Join `scope` and `name` into a global name (`a::b::name`).
#[must_use]
pub fn join<S: AsRef<str>>(scope: &[S], name: &str) -> String {
    let mut out = String::new();
    for s in scope {
        out.push_str(s.as_ref());
        out.push_str("::");
    }
    out.push_str(name);
    out
}

/// Resolve `name` referenced from `scope` (module names, outermost first).
///
/// `declared` says whether a global name is declared; the rules are in the
/// module documentation. `None` when the name does not resolve.
pub fn resolve<S: AsRef<str>>(
    scope: &[S],
    name: &str,
    declared: impl Fn(&str) -> bool,
) -> Option<String> {
    if let Some(abs) = name.strip_prefix("::") {
        return declared(abs).then(|| abs.to_string());
    }
    let (first, rest) = name
        .split_once("::")
        .map_or((name, None), |(f, r)| (f, Some(r)));
    for depth in (0..=scope.len()).rev() {
        let head = join(&scope[..depth], first);
        if declared(&head) {
            let full = match rest {
                Some(rest) => format!("{head}::{rest}"),
                None => head,
            };
            return declared(&full).then_some(full);
        }
    }
    None
}

/// What a global name declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeclKind {
    /// A module.
    Module,
    /// A type (struct, union, enum, typedef, bitset, bitmask, or a forward
    /// declaration).
    Type,
    /// A constant.
    Const,
    /// An enumerator, declared in the scope enclosing its enumeration.
    Enumerator,
}

/// Every global name an IDL file declares.
#[derive(Clone, Debug, Default)]
pub struct Declarations {
    names: HashMap<String, DeclKind>,
}

impl Declarations {
    /// The declarations of `ast`.
    #[must_use]
    pub fn of(ast: &IdlFile) -> Self {
        let mut d = Self::default();
        d.add(&ast.definitions, &mut Vec::new());
        d
    }

    fn add(&mut self, defs: &[Definition], scope: &mut Vec<String>) {
        for def in defs {
            if let Definition::Module(m) = def {
                self.names.insert(join(scope, &m.name), DeclKind::Module);
                scope.push(m.name.clone());
                self.add(&m.definitions, scope);
                scope.pop();
                continue;
            }
            if let Definition::Enum(e) = def {
                for v in &e.variants {
                    self.names
                        .entry(join(scope, &v.name))
                        .or_insert(DeclKind::Enumerator);
                }
            }
            if let Some((name, kind)) = declared_name(def) {
                let key = join(scope, name);
                // A forward declaration and its definition: one type.
                self.names.entry(key).or_insert(kind);
            }
        }
    }

    /// What `global` (`a::b::T`) declares, if anything.
    #[must_use]
    pub fn kind(&self, global: &str) -> Option<DeclKind> {
        self.names.get(global).copied()
    }

    /// The global name `name` referenced from `scope` resolves to.
    #[must_use]
    pub fn resolve<S: AsRef<str>>(&self, scope: &[S], name: &str) -> Option<String> {
        resolve(scope, name, |q| self.names.contains_key(q))
    }

    /// The global name of the TYPE `name` referenced from `scope` names:
    /// `None` when it does not resolve, or resolves to something else.
    #[must_use]
    pub fn resolve_type<S: AsRef<str>>(&self, scope: &[S], name: &str) -> Option<String> {
        self.resolve(scope, name)
            .filter(|g| self.kind(g) == Some(DeclKind::Type))
    }

    /// Every declared global name of the given kind.
    pub fn names_of(&self, kind: DeclKind) -> impl Iterator<Item = &str> {
        self.names
            .iter()
            .filter(move |(_, k)| **k == kind)
            .map(|(n, _)| n.as_str())
    }
}

/// The name a definition declares in its scope, and its kind (modules
/// excepted).
#[must_use]
pub const fn declared_name(def: &Definition) -> Option<(&str, DeclKind)> {
    let (name, kind) = match def {
        Definition::Struct(s) => (&s.name, DeclKind::Type),
        Definition::Union(u) => (&u.name, DeclKind::Type),
        Definition::Enum(e) => (&e.name, DeclKind::Type),
        Definition::Typedef(t) => (&t.name, DeclKind::Type),
        Definition::Bitset(b) => (&b.name, DeclKind::Type),
        Definition::Bitmask(b) => (&b.name, DeclKind::Type),
        Definition::ForwardDecl(f) => (&f.name, DeclKind::Type),
        Definition::Const(c) => (&c.name, DeclKind::Const),
        _ => return None,
    };
    Some((name.as_str(), kind))
}

/// Calls `f` on every type reference of `def` (members, base,
/// discriminator, typedef target), with mutable access.
pub fn for_each_reference_mut(def: &mut Definition, f: &mut impl FnMut(&mut String)) {
    fn ty(t: &mut IdlType, f: &mut impl FnMut(&mut String)) {
        match t {
            IdlType::Named(n) => f(n),
            IdlType::Sequence { inner, .. } | IdlType::Array { inner, .. } => ty(inner, f),
            IdlType::Map { key, value, .. } => {
                ty(key, f);
                ty(value, f);
            }
            IdlType::Primitive(_) | IdlType::BoundedString { .. } => {}
        }
    }
    fn structure(s: &mut Struct, f: &mut impl FnMut(&mut String)) {
        if let Some(b) = &mut s.base_struct {
            f(b);
        }
        for m in &mut s.fields {
            ty(&mut m.field_type, f);
        }
    }
    fn union(u: &mut Union, f: &mut impl FnMut(&mut String)) {
        ty(&mut u.discriminator, f);
        for c in &mut u.cases {
            ty(&mut c.field.field_type, f);
        }
    }
    match def {
        Definition::Struct(s) => structure(s, f),
        Definition::Union(u) => union(u, f),
        Definition::Typedef(t) => ty(&mut t.base_type, f),
        #[cfg(feature = "interfaces")]
        Definition::Exception(e) => {
            for m in &mut e.members {
                ty(&mut m.field_type, f);
            }
        }
        _ => {}
    }
}

/// A copy of `ast` whose scope-dependent type references are absolute.
///
/// Every type reference whose spelling a lookup that ignores scopes would
/// read differently is written as the absolute global name it resolves to (`::a::b::T`): a qualified spelling that is
/// not the global name (`b::T` from inside `a`), or an unqualified one
/// that several modules declare. Other references are left as written, so
/// generated code keeps the IDL spelling where it is unambiguous. The
/// backends that look types up by name without a scope (the shared
/// `TypeIndex`, and the Rust, micro and C++ generators) run on this copy.
#[must_use]
pub fn disambiguate(ast: &IdlFile) -> IdlFile {
    rewrite(ast, false)
}

/// A copy of `ast` whose type references are absolute wherever their
/// meaning depends on the scope they are read from.
///
/// Every reference to a type declared in a module is written as its
/// absolute global name (`::a::T`); a reference to a root type spelled as
/// its name stays as written. Generated code copies a referenced type
/// (a typedef target unrolled into the member that uses it, a member of a
/// nested key) into the scope of ANOTHER definition, where the spelling
/// from the declaring scope (`P` inside `A`) names nothing or something
/// else; an absolute name means the same type from anywhere.
#[must_use]
pub fn absolutize(ast: &IdlFile) -> IdlFile {
    rewrite(ast, true)
}

fn rewrite(ast: &IdlFile, absolute: bool) -> IdlFile {
    fn walk(
        defs: &mut [Definition],
        scope: &mut Vec<String>,
        absolute: bool,
        decls: &Declarations,
        simple: &HashMap<&str, usize>,
    ) {
        for def in defs {
            if let Definition::Module(m) = def {
                scope.push(m.name.clone());
                walk(&mut m.definitions, scope, absolute, decls, simple);
                scope.pop();
                continue;
            }
            for_each_reference_mut(def, &mut |n: &mut String| {
                if n.starts_with("::") {
                    return;
                }
                let Some(global) = decls.resolve_type(scope, n) else {
                    return;
                };
                let unique = simple
                    .get(global.rsplit("::").next().unwrap_or(&global))
                    .copied()
                    == Some(1);
                // Absolute mode: a root type spelled as its name stays (a
                // root name that another module also declares is made
                // absolute in the `TypeIndex` copy, which is where a type
                // copied into another scope is read from).
                let plain = if absolute {
                    *n == global && !global.contains("::")
                } else if n.contains("::") {
                    *n == global
                } else {
                    unique
                };
                if !plain {
                    *n = format!("::{global}");
                }
            });
        }
    }

    let decls = Declarations::of(ast);
    let mut simple: HashMap<&str, usize> = HashMap::new();
    for g in decls.names_of(DeclKind::Type) {
        *simple
            .entry(g.rsplit("::").next().unwrap_or(g))
            .or_default() += 1;
    }
    let mut out = ast.clone();
    walk(
        &mut out.definitions,
        &mut Vec::new(),
        absolute,
        &decls,
        &simple,
    );
    out
}

/// The copy of `ast` the Rust, C++ and micro backends generate from.
///
/// [`absolutize`], and every struct and union without an `@autoid` of
/// its own given the one of its innermost enclosing module that has one
/// (`XTypes` 1.3 7.3.1.2.1.1: "the type or any of the containing
/// modules"), so the type alone says how its member ids are assigned, even
/// when another module declares a type of the same name. The copy every
/// backend generates from.
#[must_use]
pub fn for_codegen(ast: &IdlFile) -> IdlFile {
    carry_autoid(absolutize(ast))
}

/// What the shared `TypeIndex` indexes: [`disambiguate`] (an AST a backend
/// already rewrote, with [`absolutize`] or its own scoping pass, is left
/// as it is) and the module `@autoid` carried by each type.
#[must_use]
pub fn for_index(ast: &IdlFile) -> IdlFile {
    carry_autoid(disambiguate(ast))
}

fn carry_autoid(mut out: IdlFile) -> IdlFile {
    fn walk(defs: &mut [Definition], inherited: Option<&Annotation>) {
        for def in defs {
            let own = |anns: &[Annotation]| anns.iter().any(|a| matches!(a, Annotation::AutoId(_)));
            match def {
                Definition::Module(m) => {
                    let here = m
                        .annotations
                        .iter()
                        .find(|a| matches!(a, Annotation::AutoId(_)))
                        .cloned();
                    walk(&mut m.definitions, here.as_ref().or(inherited));
                }
                Definition::Struct(s) => {
                    if let Some(a) = inherited.filter(|_| !own(&s.annotations)) {
                        s.annotations.push(a.clone());
                    }
                }
                Definition::Union(u) => {
                    if let Some(a) = inherited.filter(|_| !own(&u.annotations)) {
                        u.annotations.push(a.clone());
                    }
                }
                _ => {}
            }
        }
    }
    walk(&mut out.definitions, None);
    out
}

/// A copy of `ast` with every reopened module merged into its first opening.
///
/// For a target language whose modules cannot be reopened (Rust): the
/// definitions of each opening are appended, in order, recursively.
#[must_use]
pub fn merge_reopened_modules(ast: &IdlFile) -> IdlFile {
    fn merge(defs: Vec<Definition>) -> Vec<Definition> {
        let mut out: Vec<Definition> = Vec::with_capacity(defs.len());
        for def in defs {
            if let Definition::Module(m) = def {
                let existing = out.iter_mut().find_map(|d| match d {
                    Definition::Module(x) if x.name == m.name => Some(x),
                    _ => None,
                });
                match existing {
                    Some(x) => {
                        x.definitions.extend(m.definitions);
                        x.annotations.extend(m.annotations);
                    }
                    None => out.push(Definition::Module(m)),
                }
            } else {
                out.push(def);
            }
        }
        for d in &mut out {
            if let Definition::Module(m) = d {
                m.definitions = merge(std::mem::take(&mut m.definitions));
            }
        }
        out
    }
    IdlFile {
        definitions: merge(ast.definitions.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;

    fn declared<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |q| names.contains(&q)
    }

    #[test]
    fn innermost_scope_wins() {
        let d = declared(&["X", "a", "a::X", "a::b", "a::b::X"]);
        assert_eq!(resolve(&["a", "b"], "X", &d).as_deref(), Some("a::b::X"));
        assert_eq!(resolve(&["a"], "X", &d).as_deref(), Some("a::X"));
        assert_eq!(resolve::<&str>(&[], "X", &d).as_deref(), Some("X"));
        assert_eq!(resolve(&["a", "c"], "X", &d).as_deref(), Some("a::X"));
    }

    #[test]
    fn absolute_names_start_at_the_root() {
        let d = declared(&["X", "a", "a::X"]);
        assert_eq!(resolve(&["a"], "::X", &d).as_deref(), Some("X"));
        assert_eq!(resolve(&["a"], "::a::X", &d).as_deref(), Some("a::X"));
        assert_eq!(resolve(&["a"], "::Y", &d), None);
    }

    /// 7.5.1: the first segment is found from the innermost scope, the
    /// rest only inside what it found.
    #[test]
    fn qualified_names_resolve_their_first_segment() {
        let d = declared(&["b", "b::X", "a", "a::b", "a::b::Y", "a::c"]);
        assert_eq!(resolve(&["a", "c"], "b::Y", &d).as_deref(), Some("a::b::Y"));
        assert_eq!(resolve(&["a", "c"], "b::X", &d), None);
        assert_eq!(resolve::<&str>(&[], "b::X", &d).as_deref(), Some("b::X"));
    }
}
