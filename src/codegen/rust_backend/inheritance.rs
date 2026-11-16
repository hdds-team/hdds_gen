// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! IDL struct inheritance flattening for the Rust backend.
//!
//! OMG DDS-XTypes v1.3 ss7.2.2.4.4: a derived type's member sequence is the
//! base type's members followed by the derived type's own members. Rust has
//! no struct inheritance, so the backend flattens the base members into each
//! derived struct before codegen; every emitter (struct definition, encode,
//! decode, builder, `TypeObject`, `compute_key`, `max_size`) then reads one
//! complete `fields` list and needs no inheritance awareness of its own.

use crate::ast::{Definition, Field, IdlFile, Module, Struct};
use crate::error::{ErrorKind, ParseError, Position, Result};
use crate::types::IdlType;
use std::collections::{HashMap, HashSet};

/// Flatten every derived struct in `ast`, returning a rewritten copy.
///
/// The pass copies instead of mutating in place: the C, C++, Python and
/// TypeScript backends render inheritance as composition off the same parsed
/// AST, so flattening must not leak past the Rust backend boundary.
///
/// Errors (rather than panics) on an unresolvable base name, an inheritance
/// cycle, or a derived member that redeclares an inherited one.
pub(super) fn flatten(ast: &IdlFile) -> Result<IdlFile> {
    let mut index = Index::default();
    index.collect(&ast.definitions, &mut Vec::new());

    let mut memo = HashMap::new();
    let definitions = rewrite(&ast.definitions, &mut Vec::new(), &index, &mut memo)?;
    Ok(IdlFile { definitions })
}

/// What flattening needs to know about one struct, keyed by its fully
/// qualified name.
struct StructInfo {
    /// Enclosing module path, for scope-aware name resolution.
    module_path: Vec<String>,
    base: Option<String>,
    fields: Vec<Field>,
}

/// Fully qualified index of the declarations flattening has to resolve:
/// candidate base structs, and every named type a copied member could refer
/// to.
#[derive(Default)]
struct Index {
    structs: HashMap<String, StructInfo>,
    /// All named types (structs, enums, unions, typedefs, bitsets, bitmasks),
    /// needed to tell whether a copied member's type name still means the
    /// same thing in the derived struct's scope.
    type_names: HashSet<String>,
}

impl Index {
    /// Index every declaration under its full accumulated module path, the
    /// same qualified-key scheme as [`super::helpers::set_type_defs`].
    fn collect(&mut self, defs: &[Definition], module_path: &mut Vec<String>) {
        for def in defs {
            let named = match def {
                Definition::Struct(s) => {
                    self.structs.insert(
                        qualify(module_path, &s.name),
                        StructInfo {
                            module_path: module_path.clone(),
                            base: s.base_struct.clone(),
                            fields: s.fields.clone(),
                        },
                    );
                    Some(s.name.as_str())
                }
                Definition::Enum(e) => Some(e.name.as_str()),
                Definition::Union(u) => Some(u.name.as_str()),
                Definition::Typedef(t) => Some(t.name.as_str()),
                Definition::Bitset(b) => Some(b.name.as_str()),
                Definition::Bitmask(bm) => Some(bm.name.as_str()),
                Definition::Module(m) => {
                    module_path.push(m.name.clone());
                    self.collect(&m.definitions, module_path);
                    module_path.pop();
                    None
                }
                _ => None,
            };
            if let Some(name) = named {
                self.type_names.insert(qualify(module_path, name));
            }
        }
    }
}

/// The AST carries no source spans past the parser, so codegen-stage errors
/// report an unknown position.
fn codegen_error(kind: ErrorKind, message: String) -> ParseError {
    ParseError::new(kind, Position::new(0, 0), message)
}

fn qualify(module_path: &[String], name: &str) -> String {
    if module_path.is_empty() {
        name.to_string()
    } else {
        format!("{}::{name}", module_path.join("::"))
    }
}

/// Resolve `name` (bare `Base` or module-qualified `A::Base`, both of which
/// the parser produces) to a fully qualified key, trying the referring
/// scope's enclosing modules innermost-first and finally the global scope,
/// exact match at each step.
///
/// Deliberately stricter than [`super::helpers::lookup_type_def`]: that index
/// also carries a bare-name, first-declaration-wins slot, so a bare `Base`
/// referenced from module `B` can land on an unrelated `A::Base`. Here the
/// referring struct's own scope is always consulted first and the bare name
/// only matches a genuinely global declaration.
fn resolve_in_scope(name: &str, from: &[String], exists: impl Fn(&str) -> bool) -> Option<String> {
    (0..=from.len()).rev().find_map(|i| {
        let candidate = qualify(&from[..i], name);
        exists(&candidate).then_some(candidate)
    })
}

/// Where a member is being copied from and to, for requalification.
struct Move<'a> {
    base: &'a [String],
    derived: &'a [String],
    /// Fully qualified name of the inheriting struct, for error messages.
    derived_key: &'a str,
    member: &'a str,
}

/// Rewrite the named types inside `ty` so they still resolve, and still mean
/// the same type, after the member is copied from the `base` module scope into
/// the `derived` one.
///
/// Generated modules only carry `use super::*`, so a bare `Inner` copied out
/// of `module A` into `module B` would not compile (and, worse, would bind to
/// a same-named `B::Inner` if one existed). Names are therefore rewritten to
/// their fully qualified form, which is what the generator already emits for
/// an explicit cross-module reference.
///
/// That spelling has one hole, which this function rejects rather than
/// mis-emit: when the target is declared in a scope OUTSIDE the derived
/// struct's and some scope on the derived struct's chain declares the same
/// leading path segment, the qualified spelling is shadowed at the point of
/// use (Rust binds a local item over a glob import) and no spelling the
/// generator can currently emit says "the outer one" — see the module note in
/// [`shadowed_inherited_member`].
fn requalify(ty: &IdlType, mv: &Move<'_>, types: &HashSet<String>) -> Result<IdlType> {
    match ty {
        IdlType::Named(name) => {
            let Some(target) = resolve_in_scope(name, mv.base, |c| types.contains(c)) else {
                // Unknown type: not this pass's business to diagnose.
                return Ok(ty.clone());
            };
            let from_derived = resolve_in_scope(name, mv.derived, |c| types.contains(c));
            if from_derived.as_ref() == Some(&target) {
                return Ok(ty.clone());
            }
            // The rewritten spelling must itself round-trip from the derived
            // struct's scope, or it silently names a different type.
            let shadowed = resolve_in_scope(&target, mv.derived, |c| types.contains(c));
            if shadowed.as_deref() == Some(target.as_str()) {
                Ok(IdlType::Named(target))
            } else {
                Err(shadowed_inherited_member(
                    mv,
                    &target,
                    shadowed.as_deref().unwrap_or(&target),
                ))
            }
        }
        IdlType::Sequence { inner, bound } => Ok(IdlType::Sequence {
            inner: Box::new(requalify(inner, mv, types)?),
            bound: *bound,
        }),
        IdlType::Array { inner, size } => Ok(IdlType::Array {
            inner: Box::new(requalify(inner, mv, types)?),
            size: *size,
        }),
        IdlType::Map { key, value, bound } => Ok(IdlType::Map {
            key: Box::new(requalify(key, mv, types)?),
            value: Box::new(requalify(value, mv, types)?),
            bound: *bound,
        }),
        IdlType::Primitive(_) => Ok(ty.clone()),
    }
}

/// An inherited member's type cannot be spelled from the derived struct's
/// scope because a nearer declaration shadows it.
///
/// The generator emits every module as `pub mod M { use super::* }` and refers
/// to types by name, so the only spellings available are relative ones — and
/// Rust resolves a module-local item ahead of a glob import. A root-anchored
/// spelling (`super::Inner`) would bind correctly but is not viable: the
/// name-keyed classifiers the emitters share (`lookup_type_def`,
/// `is_xcdr2_primitive_element_inner`, `classify_key_field`) would all miss on
/// it, and a miss silently changes wire framing — measured: an inherited
/// `sequence<Id>` over a typedef-to-primitive gained an XCDR2 DHEADER the base
/// does not emit. Refusing loudly is the honest outcome until the emitters
/// resolve types through something other than the name string.
fn shadowed_inherited_member(mv: &Move<'_>, target: &str, shadowed_by: &str) -> ParseError {
    codegen_error(
        ErrorKind::DuplicateDefinition,
        format!(
            "Struct '{}' inherits member '{}' of type '{target}', but '{shadowed_by}' \
             shadows that name in its scope; rename one of the two types (the generated \
             module would bind '{}' to '{shadowed_by}')",
            mv.derived_key, mv.member, mv.member,
        ),
    )
}

/// Member list for `key`: its bases' members (deepest base first, in
/// declaration order) followed by its own.
///
/// Every returned member's type names are valid in `key`'s own module scope,
/// which is what makes the requalification below compose across a multi-level
/// chain: each level rewrites into its own scope before the next reads it.
fn flattened_fields(
    key: &str,
    index: &Index,
    memo: &mut HashMap<String, Vec<Field>>,
    stack: &mut Vec<String>,
) -> Result<Vec<Field>> {
    if let Some(cached) = memo.get(key) {
        return Ok(cached.clone());
    }
    if stack.iter().any(|visited| visited == key) {
        // Report the chain without disturbing the visit stack.
        return Err(codegen_error(
            ErrorKind::InvalidSyntax,
            format!(
                "Inheritance cycle in struct '{key}': {} -> {key}",
                stack.join(" -> ")
            ),
        ));
    }
    let info = index.structs.get(key).ok_or_else(|| {
        codegen_error(
            ErrorKind::Other,
            format!("Internal error: struct '{key}' vanished from the inheritance index"),
        )
    })?;

    let mut fields = Vec::new();
    if let Some(base) = &info.base {
        let base_key = resolve_in_scope(base, &info.module_path, |c| index.structs.contains_key(c))
            .ok_or_else(|| {
                codegen_error(
                    ErrorKind::UnknownType,
                    format!("Unknown base struct '{base}' inherited by struct '{key}'"),
                )
            })?;
        stack.push(key.to_string());
        let inherited = flattened_fields(&base_key, index, memo, stack);
        stack.pop();
        let inherited = inherited?;

        let base_path = &index
            .structs
            .get(&base_key)
            .ok_or_else(|| {
                codegen_error(
                    ErrorKind::Other,
                    format!("Internal error: base struct '{base_key}' vanished from the index"),
                )
            })?
            .module_path;
        if base_path == &info.module_path {
            fields = inherited;
        } else {
            for mut f in inherited {
                let mv = Move {
                    base: base_path,
                    derived: &info.module_path,
                    derived_key: key,
                    member: &f.name,
                };
                f.field_type = requalify(&f.field_type, &mv, &index.type_names)?;
                fields.push(f);
            }
        }
    }

    // A derived member may not shadow an inherited one: the flattened list is
    // a single member sequence, and duplicate names would emit invalid Rust.
    for own in &info.fields {
        if fields.iter().any(|f| f.name == own.name) {
            return Err(codegen_error(
                ErrorKind::DuplicateDefinition,
                format!("Struct '{key}' redeclares inherited member '{}'", own.name),
            ));
        }
    }
    fields.extend(info.fields.iter().cloned());

    memo.insert(key.to_string(), fields.clone());
    Ok(fields)
}

/// Rebuild the definition list with each derived struct's fields flattened
/// and its now-consumed base link cleared.
fn rewrite(
    defs: &[Definition],
    module_path: &mut Vec<String>,
    index: &Index,
    memo: &mut HashMap<String, Vec<Field>>,
) -> Result<Vec<Definition>> {
    let mut out = Vec::with_capacity(defs.len());
    for def in defs {
        match def {
            Definition::Struct(s) if s.base_struct.is_some() => {
                let key = qualify(module_path, &s.name);
                let fields = flattened_fields(&key, index, memo, &mut Vec::new())?;
                out.push(Definition::Struct(Struct {
                    fields,
                    base_struct: None,
                    ..s.clone()
                }));
            }
            Definition::Module(m) => {
                module_path.push(m.name.clone());
                let nested = rewrite(&m.definitions, module_path, index, memo);
                module_path.pop();
                out.push(Definition::Module(Module {
                    definitions: nested?,
                    ..m.clone()
                }));
            }
            other => out.push(other.clone()),
        }
    }
    Ok(out)
}
