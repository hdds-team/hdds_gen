// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! The `KeyHolder` of a keyed type, as every backend needs it to emit the
//! instance `KeyHash` of DDSI-RTPS 2.5 9.6.4.8 / DDS-`XTypes` 1.3 7.6.8.
//!
//! # The algorithm the generated code implements
//!
//! 1. `KeyHolder(T)` keeps the key members of `T` (all members when `T` has
//!    none), in ascending member-id order, extensibility FINAL.
//! 2. The rule applies recursively to members that are aggregated types: a
//!    nested structure with key members contributes those, one without
//!    contributes all its members. A union whose discriminator is a key
//!    contributes the discriminator alone, otherwise the whole union.
//! 3. The `KeyHolder` object is serialized as `PLAIN_CDR2` big endian, alignment
//!    capped at 4, zero padding, from a buffer aligned to 4.
//! 4. If the `KeyHolder`'s MAXIMUM serialized size is at most 16 bytes, the
//!    `KeyHash` is the serialization zero-extended to 16 bytes, otherwise its
//!    MD5. The choice follows the TYPE, never the value.
//!
//! This module decides everything that is known at generation time: which
//! members, in which order, written with which primitive width, and the
//! maximum size. A backend walks the resulting [`KhStruct`] and emits one
//! writer call per leaf.
//!
//! # Member ids
//!
//! The order is the order of the member ids `XTypes` 1.3 7.3.1.2.1.1 assigns:
//! an explicit `@id` wins; otherwise `@autoid(HASH)` (or `@hashid`) gives the
//! first four bytes of the MD5 of the name, little endian, masked to 28 bits;
//! otherwise the id is the previous member's id plus one, starting at zero
//! (SEQUENTIAL, the default), counting the base type's members first.
//!
//! # What is not covered
//!
//! A key member whose type is a map, a `fixed`, a `long double`, an
//! `@optional` member, or a sequence or array whose element is not a
//! primitive (strings, enums, structures) is refused: `XTypes` serializes such
//! collections with a DHEADER while the `KeyHash` note excludes "type headers",
//! and which reading peers implement is not established (the HDDS runtime
//! offers no writer call for it either). A backend emits no `KeyHash` for a
//! type it is refused for, and says so in the generated code.

use std::collections::HashMap;

use crate::ast::{Definition, Field, IdlFile, Struct, Union, UnionLabel};
use crate::types::{Annotation, AutoIdKind, IdlType, PrimitiveType};

/// Length of a `KeyHash`, and the largest maximum size that selects the padded
/// form (RTPS 2.5 9.6.4.8 step 5.1).
pub const KEY_HASH_LEN: usize = 16;

/// `PLAIN_CDR2` caps alignment at 4 bytes (`XTypes` 1.3 7.4.2).
const MAX_ALIGN: usize = 4;

/// Deepest nesting of aggregated types followed before giving up (a key
/// that reaches a recursive type through a nested member).
const MAX_DEPTH: usize = 32;

/// One primitive writer call of the `KeyHolder` serialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KhPrim {
    /// `boolean`: one byte, 0 or 1.
    Bool,
    /// `octet`, `uint8`.
    Octet,
    /// `char`: one byte.
    Char,
    /// `int8`.
    I8,
    /// `short`, `int16`.
    I16,
    /// `unsigned short`, `uint16`.
    U16,
    /// `wchar`: its UTF-16 code unit.
    WChar,
    /// `long`, `int32`.
    I32,
    /// `unsigned long`, `uint32`.
    U32,
    /// `long long`, `int64`.
    I64,
    /// `unsigned long long`, `uint64`.
    U64,
    /// `float`.
    F32,
    /// `double`.
    F64,
}

impl KhPrim {
    /// Serialized size in bytes.
    #[must_use]
    pub const fn size(self) -> usize {
        match self {
            Self::Bool | Self::Octet | Self::Char | Self::I8 => 1,
            Self::I16 | Self::U16 | Self::WChar => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
            Self::I64 | Self::U64 | Self::F64 => 8,
        }
    }
}

/// A single primitive value and how the language value maps to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KhScalar {
    /// An IDL primitive.
    Prim(KhPrim),
    /// An enumeration, written as a signed integer of its `@bit_bound`
    /// width (`XTypes` 1.3 7.3.1.2.1.6: 1-8 -> int8, 9-16 -> int16, else int32).
    Enum {
        /// Unqualified enum name.
        name: String,
        /// `I8`, `I16` or `I32`.
        prim: KhPrim,
    },
    /// A bitmask, written as an unsigned integer of its `@bit_bound` width
    /// (default 32).
    Bitmask {
        /// Unqualified bitmask name.
        name: String,
        /// `Octet`, `U16`, `U32` or `U64`.
        prim: KhPrim,
    },
}

impl KhScalar {
    /// The primitive written for this value.
    #[must_use]
    pub const fn prim(&self) -> KhPrim {
        match self {
            Self::Prim(p) | Self::Enum { prim: p, .. } | Self::Bitmask { prim: p, .. } => *p,
        }
    }
}

/// One node of a `KeyHolder`.
#[derive(Clone, Debug)]
pub enum KhNode {
    /// A primitive, enum or bitmask value.
    Scalar(KhScalar),
    /// `string` or `string<N>`: length (NUL counted), bytes, NUL.
    String {
        /// The bound, `None` when unbounded.
        bound: Option<u32>,
    },
    /// `wstring` or `wstring<N>`: length in bytes, UTF-16 code units.
    WString {
        /// The bound in characters, `None` when unbounded.
        bound: Option<u32>,
    },
    /// `sequence<P>` of a primitive `P`: length, then the elements.
    Sequence {
        /// The element.
        elem: KhScalar,
        /// The bound, `None` when unbounded.
        bound: Option<u32>,
    },
    /// `P[N]...` of a primitive `P`: the elements, no length.
    Array {
        /// The element.
        elem: KhScalar,
        /// The dimensions, outermost first.
        dims: Vec<u32>,
    },
    /// A nested structure, through its own `KeyHolder`.
    Struct(KhStruct),
    /// A nested union.
    Union(Box<KhUnion>),
}

impl KhNode {
    /// See [`KhStruct::base_member`].
    #[must_use]
    pub fn base_member(&self) -> Option<(String, String)> {
        match self {
            Self::Struct(s) => s.base_member(),
            Self::Union(u) => u
                .cases
                .iter()
                .filter_map(|c| c.node.as_ref())
                .find_map(Self::base_member),
            _ => None,
        }
    }
}

/// The `KeyHolder` of a structure.
#[derive(Clone, Debug)]
pub struct KhStruct {
    /// Unqualified name of the structure.
    pub name: String,
    /// Global name of the structure (`a::b::T`), the unqualified name when
    /// it is not indexed.
    pub global: String,
    /// The `KeyHolder` members, in DECLARATION order (base members first).
    /// [`KhStruct::hash_order`] gives the member-id order the `KeyHash` uses.
    pub members: Vec<KhMember>,
}

/// One member of a `KeyHolder`.
#[derive(Clone, Debug)]
pub struct KhMember {
    /// The IDL member.
    pub field: Field,
    /// Name of the structure that declares the member (the type itself or
    /// one of its bases).
    pub owner: String,
    /// How many `base` hops lead from the type to `owner` (0: own member).
    pub base_depth: usize,
    /// Index of the member in `owner`'s declaration.
    pub decl_index: usize,
    /// Member id as `XTypes` 1.3 7.3.1.2.1.1 assigns it.
    pub member_id: u32,
    /// The member's `KeyHolder` node.
    pub node: KhNode,
}

/// The `KeyHolder` of a union.
#[derive(Clone, Debug)]
pub struct KhUnion {
    /// Unqualified name of the union.
    pub name: String,
    /// The discriminator.
    pub disc: KhScalar,
    /// The discriminator's IDL type, for backends that spell case labels.
    pub disc_type: IdlType,
    /// The discriminator is a key: the `KeyHolder` is the discriminator alone.
    pub disc_is_key: bool,
    /// The union's extensibility, which frames its part of a key-only
    /// payload (`XTypes` 1.3 7.4.3.5.3 rules 27-30).
    pub ext: crate::codegen::data_model::Ext,
    /// Every case, in declaration order, with its value's `KeyHolder` node.
    pub cases: Vec<KhCase>,
}

/// One case of a union `KeyHolder`.
#[derive(Clone, Debug)]
pub struct KhCase {
    /// Position of the case in the union declaration.
    pub index: usize,
    /// The case labels.
    pub labels: Vec<UnionLabel>,
    /// The case member.
    pub field: Field,
    /// The value's node; `None` for a `void` case.
    pub node: Option<KhNode>,
}

impl KhStruct {
    /// The members in ascending member-id order (RTPS 2.5 9.6.4.8 step 1).
    /// Equal ids keep declaration order.
    #[must_use]
    pub fn hash_order(&self) -> Vec<&KhMember> {
        let mut members: Vec<&KhMember> = self.members.iter().collect();
        members.sort_by_key(|m| m.member_id);
        members
    }

    /// The first member, at any depth, that a base type declares, as
    /// (member, base): backends whose types drop base members cannot read
    /// it.
    #[must_use]
    pub fn base_member(&self) -> Option<(String, String)> {
        self.members.iter().find_map(|m| {
            if m.base_depth > 0 {
                Some((m.field.name.clone(), m.owner.clone()))
            } else {
                m.node.base_member()
            }
        })
    }

    /// Maximum serialized size of this `KeyHolder` (RTPS 2.5 9.6.4.8 step 5),
    /// `None` when unbounded.
    #[must_use]
    pub fn max_size(&self) -> Option<usize> {
        walk_struct_max(self, 0)
    }

    /// The `KeyHash` is the zero-extended serialization (maximum at most 16)
    /// rather than its MD5.
    #[must_use]
    pub fn is_padded_form(&self) -> bool {
        matches!(self.max_size(), Some(n) if n <= KEY_HASH_LEN)
    }
}

const fn align_up(offset: usize, size: usize) -> usize {
    let align = if size < MAX_ALIGN { size } else { MAX_ALIGN };
    if align <= 1 {
        offset
    } else {
        offset.div_ceil(align) * align
    }
}

/// End offset after a node written at `offset` with its largest value. The
/// end offset is monotone in the start offset and in every length, so the
/// largest value of every member gives the largest serialization.
fn walk_max(node: &KhNode, offset: usize) -> Option<usize> {
    match node {
        KhNode::Scalar(s) => {
            let size = s.prim().size();
            Some(align_up(offset, size) + size)
        }
        KhNode::String { bound } => {
            let n = usize::try_from((*bound)?).ok()?;
            Some(align_up(offset, 4) + 4 + n + 1)
        }
        KhNode::WString { bound } => {
            let n = usize::try_from((*bound)?).ok()?;
            Some(align_up(offset, 4) + 4 + 2 * n)
        }
        KhNode::Sequence { elem, bound } => {
            let n = usize::try_from((*bound)?).ok()?;
            let mut end = align_up(offset, 4) + 4;
            let size = elem.prim().size();
            for _ in 0..n {
                end = align_up(end, size) + size;
            }
            Some(end)
        }
        KhNode::Array { elem, dims } => {
            let mut count: usize = 1;
            for d in dims {
                count = count.checked_mul(usize::try_from(*d).ok()?)?;
            }
            let size = elem.prim().size();
            let mut end = offset;
            for _ in 0..count {
                end = align_up(end, size) + size;
            }
            Some(end)
        }
        KhNode::Struct(s) => walk_struct_max(s, offset),
        KhNode::Union(u) => {
            let size = u.disc.prim().size();
            let after_disc = align_up(offset, size) + size;
            if u.disc_is_key {
                return Some(after_disc);
            }
            let mut end = after_disc;
            for case in &u.cases {
                if let Some(node) = &case.node {
                    end = end.max(walk_max(node, after_disc)?);
                }
            }
            Some(end)
        }
    }
}

fn walk_struct_max(s: &KhStruct, offset: usize) -> Option<usize> {
    let mut end = offset;
    for m in s.hash_order() {
        end = walk_max(&m.node, end)?;
    }
    Some(end)
}

/// A named type of an IDL file, as the backends need to serialize it.
#[derive(Clone, Debug)]
pub enum Named {
    /// A structure.
    Struct(Struct),
    /// A union.
    Union(Union),
    /// An enumeration: its `@bit_bound` and its literals with their values.
    Enum {
        /// `@bit_bound`, if any (default 32).
        bit_bound: Option<u32>,
        /// Literal names and values, in declaration order.
        literals: Vec<(String, i64)>,
    },
    /// A bitmask: its `@bit_bound` (default 32).
    Bitmask {
        /// `@bit_bound`, if any.
        bit_bound: Option<u32>,
    },
    /// A typedef and the type it names.
    Typedef(IdlType),
    /// A bitset.
    Bitset,
}

/// Every named type of an IDL file, by global name (`a::b::T`).
///
/// The file is indexed after [`crate::scope::disambiguate`], so every type
/// reference inside an indexed type (a member, a base, a typedef target)
/// is either its global name, absolute (`::a::T`), or an unqualified name
/// only one type declares. A name is looked up as such: absolute (`::a::T`
/// or the Rust path `__hdds_root::a::T`) and qualified names by global
/// name, an unqualified one by the only type declaring it (the last one
/// indexed when several do: callers holding a declaration use
/// [`TypeIndex::global_of_struct`] and the like instead).
#[derive(Clone, Debug, Default)]
pub struct TypeIndex {
    /// Unqualified name to the global names declaring it, in order.
    types: HashMap<String, Vec<String>>,
    /// Global name to type.
    qualified: HashMap<String, Named>,
    /// `@autoid` of the innermost enclosing module that declares one, per
    /// structure or union global name (`XTypes` 1.3 7.3.1.2.1.1: "the type
    /// or any of the containing modules").
    module_autoid: HashMap<String, AutoIdKind>,
}

fn bit_bound(annotations: &[Annotation]) -> Option<u32> {
    annotations.iter().find_map(|a| match a {
        Annotation::BitBound(n) => Some(*n),
        _ => None,
    })
}

fn unqualified(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

impl TypeIndex {
    /// Index every named type of `ast`, modules included.
    #[must_use]
    pub fn new(ast: &IdlFile) -> Self {
        let mut index = Self::default();
        index.add_file(ast);
        index
    }

    /// Add every named type of `ast` (an included file, say). A name
    /// already indexed is replaced.
    pub fn add_file(&mut self, ast: &IdlFile) {
        fn walk(
            index: &mut TypeIndex,
            defs: &[Definition],
            autoid: Option<AutoIdKind>,
            path: &mut Vec<String>,
        ) {
            for def in defs {
                let named = match def {
                    Definition::Module(m) => {
                        let inner = own_autoid(&m.annotations).or(autoid);
                        path.push(m.name.clone());
                        walk(index, &m.definitions, inner, path);
                        path.pop();
                        continue;
                    }
                    Definition::Struct(s) => (&s.name, Named::Struct(s.clone())),
                    Definition::Union(u) => (&u.name, Named::Union(u.clone())),
                    Definition::Enum(e) => {
                        let literals = e
                            .variants
                            .iter()
                            .map(|v| v.name.clone())
                            .zip(e.resolved_values())
                            .collect();
                        (
                            &e.name,
                            Named::Enum {
                                bit_bound: bit_bound(&e.annotations),
                                literals,
                            },
                        )
                    }
                    Definition::Bitset(b) => (&b.name, Named::Bitset),
                    Definition::Bitmask(b) => (
                        &b.name,
                        Named::Bitmask {
                            bit_bound: bit_bound(&b.annotations),
                        },
                    ),
                    Definition::Typedef(t) => (&t.name, Named::Typedef(t.base_type.clone())),
                    _ => continue,
                };
                let (name, named) = named;
                let global = crate::scope::join(path, name);
                if matches!(named, Named::Struct(_) | Named::Union(_)) {
                    match autoid {
                        Some(kind) => index.module_autoid.insert(global.clone(), kind),
                        None => index.module_autoid.remove(&global),
                    };
                }
                let same = index.types.entry(name.clone()).or_default();
                same.retain(|g| *g != global);
                same.push(global.clone());
                index.qualified.insert(global, named);
            }
        }
        let ast = crate::scope::for_index(ast);
        walk(self, &ast.definitions, None, &mut Vec::new());
    }

    /// The global name (`a::b::T`) of the type `name` names, if indexed:
    /// see [`TypeIndex`] for how a name is looked up.
    #[must_use]
    pub fn global(&self, name: &str) -> Option<String> {
        let absolute = name.strip_prefix("::").or_else(|| {
            name.strip_prefix(crate::codegen::rust_paths::ROOT)
                .and_then(|rest| rest.strip_prefix("::"))
        });
        if let Some(abs) = absolute {
            return self.qualified.contains_key(abs).then(|| abs.to_string());
        }
        if self.qualified.contains_key(name) {
            return Some(name.to_string());
        }
        if name.contains("::") {
            return None;
        }
        self.types.get(name).and_then(|g| g.last()).cloned()
    }

    /// The global name of the structure `s` (one of the indexed ones, or a
    /// copy of one): the only structure of that name, else the one equal to
    /// it, else the one with the same member names.
    #[must_use]
    pub fn global_of_struct(&self, s: &Struct) -> Option<String> {
        let candidates: Vec<(&String, &Struct)> = self
            .types
            .get(&s.name)
            .into_iter()
            .flatten()
            .filter_map(|g| match self.qualified.get(g) {
                Some(Named::Struct(x)) => Some((g, x)),
                _ => None,
            })
            .collect();
        let names = |x: &Struct| x.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
        match candidates.as_slice() {
            [(g, _)] => Some((*g).clone()),
            _ => candidates
                .iter()
                .find(|(_, x)| *x == s)
                .or_else(|| candidates.iter().find(|(_, x)| names(x) == names(s)))
                .map(|(g, _)| (*g).clone()),
        }
    }

    /// The global name of the union `u`, as [`TypeIndex::global_of_struct`].
    #[must_use]
    pub fn global_of_union(&self, u: &Union) -> Option<String> {
        let candidates: Vec<(&String, &Union)> = self
            .types
            .get(&u.name)
            .into_iter()
            .flatten()
            .filter_map(|g| match self.qualified.get(g) {
                Some(Named::Union(x)) => Some((g, x)),
                _ => None,
            })
            .collect();
        match candidates.as_slice() {
            [(g, _)] => Some((*g).clone()),
            _ => candidates
                .iter()
                .find(|(_, x)| *x == u)
                .map(|(g, _)| (*g).clone()),
        }
    }

    /// Whether `s` assigns its implicit member ids by name hash: its own
    /// `@autoid`, else that of the innermost enclosing module, else
    /// SEQUENTIAL.
    #[must_use]
    pub fn hashes_member_ids(&self, s: &Struct) -> bool {
        own_autoid(&s.annotations).or_else(|| {
            self.global_of_struct(s)
                .and_then(|g| self.module_autoid.get(&g).copied())
        }) == Some(AutoIdKind::Hash)
    }

    /// Whether the implicit member ids of union `u`'s cases are name
    /// hashes: its own `@autoid`, else the innermost enclosing module's.
    #[must_use]
    pub fn union_hashes_member_ids(&self, u: &Union) -> bool {
        own_autoid(&u.annotations).or_else(|| {
            self.global_of_union(u)
                .and_then(|g| self.module_autoid.get(&g).copied())
        }) == Some(AutoIdKind::Hash)
    }

    fn get(&self, name: &str) -> Option<&Named> {
        self.global(name).and_then(|g| self.qualified.get(&g))
    }

    /// The named type `name` (qualified or not), if any.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&Named> {
        self.get(name)
    }

    /// The structure named `name`, if any.
    #[must_use]
    pub fn struct_def(&self, name: &str) -> Option<&Struct> {
        match self.get(name) {
            Some(Named::Struct(s)) => Some(s),
            _ => None,
        }
    }

    /// The union named `name`, if any.
    #[must_use]
    pub fn union_def(&self, name: &str) -> Option<&Union> {
        match self.get(name) {
            Some(Named::Union(u)) => Some(u),
            _ => None,
        }
    }

    /// Follow typedefs from `ty` to a type that is not an alias.
    #[must_use]
    pub fn resolve(&self, ty: &IdlType) -> IdlType {
        let mut current = ty.clone();
        for _ in 0..MAX_DEPTH {
            match &current {
                IdlType::Named(n) => match self.get(n) {
                    Some(Named::Typedef(base)) => current = base.clone(),
                    _ => return current,
                },
                _ => return current,
            }
        }
        current
    }

    /// The structure `s` followed by its bases, outermost (the root base)
    /// first, or an error for an unknown or cyclic base.
    ///
    /// # Errors
    ///
    /// Returns the reason when a base is unknown or the chain is cyclic.
    pub fn base_chain<'a>(&'a self, s: &'a Struct) -> Result<Vec<&'a Struct>, String> {
        let mut chain = vec![s];
        let mut current = s;
        while let Some(base) = &current.base_struct {
            if chain.len() > MAX_DEPTH {
                return Err(format!("base chain of `{}` is cyclic", s.name));
            }
            let Some(b) = self.struct_def(base) else {
                return Err(format!(
                    "base type `{base}` of `{}` is unknown",
                    current.name
                ));
            };
            chain.push(b);
            current = b;
        }
        chain.reverse();
        Ok(chain)
    }

    /// The `KeyHolder` of `s` when `s` is a keyed type (has a key member,
    /// its own or a base's): `Ok(None)` when it has none, `Err(reason)` when
    /// a key member's type is not covered (see the module documentation).
    ///
    /// # Errors
    ///
    /// Returns the reason the `KeyHolder` cannot be built.
    pub fn key_holder(&self, s: &Struct) -> Result<Option<KhStruct>, String> {
        let chain = self.base_chain(s)?;
        if !chain.iter().any(|c| c.fields.iter().any(Field::is_key)) {
            return Ok(None);
        }
        self.struct_holder(s, 0).map(Some)
    }

    /// Whether `s` or one of its bases declares a key member.
    #[must_use]
    pub fn is_keyed(&self, s: &Struct) -> bool {
        self.base_chain(s).map_or_else(
            |_| s.fields.iter().any(Field::is_key),
            |chain| chain.iter().any(|c| c.fields.iter().any(Field::is_key)),
        )
    }

    fn struct_holder(&self, s: &Struct, depth: usize) -> Result<KhStruct, String> {
        if depth > MAX_DEPTH {
            return Err(format!("`{}` nests itself through its key", s.name));
        }
        let chain = self.base_chain(s)?;
        let modes: Vec<(&Struct, bool)> = chain
            .iter()
            .map(|c| (*c, self.hashes_member_ids(c)))
            .collect();
        let ids = spec_member_ids_with(&modes);
        let any_key = chain.iter().any(|c| c.fields.iter().any(Field::is_key));
        let mut members = Vec::new();
        let mut id_iter = ids.into_iter();
        let hops = chain.len() - 1;
        for (level, owner) in chain.iter().enumerate() {
            for (decl_index, field) in owner.fields.iter().enumerate() {
                let member_id = id_iter.next().unwrap_or(0);
                if field.is_non_serialized() || (any_key && !field.is_key()) {
                    continue;
                }
                if field.is_optional() {
                    return Err(format!(
                        "member `{}` of `{}` is @optional",
                        field.name, owner.name
                    ));
                }
                let node = self
                    .node(&field.field_type, depth + 1)
                    .map_err(|e| format!("member `{}` of `{}`: {e}", field.name, owner.name))?;
                members.push(KhMember {
                    field: field.clone(),
                    owner: owner.name.clone(),
                    base_depth: hops - level,
                    decl_index,
                    member_id,
                    node,
                });
            }
        }
        Ok(KhStruct {
            name: s.name.clone(),
            global: self.global_of_struct(s).unwrap_or_else(|| s.name.clone()),
            members,
        })
    }

    fn scalar(&self, ty: &IdlType) -> Result<Option<KhScalar>, String> {
        let prim = match self.resolve(ty) {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Boolean => KhPrim::Bool,
                PrimitiveType::Octet | PrimitiveType::UInt8 => KhPrim::Octet,
                PrimitiveType::Char => KhPrim::Char,
                PrimitiveType::Int8 => KhPrim::I8,
                PrimitiveType::Short | PrimitiveType::Int16 => KhPrim::I16,
                PrimitiveType::UnsignedShort | PrimitiveType::UInt16 => KhPrim::U16,
                PrimitiveType::WChar => KhPrim::WChar,
                PrimitiveType::Long | PrimitiveType::Int32 => KhPrim::I32,
                PrimitiveType::UnsignedLong | PrimitiveType::UInt32 => KhPrim::U32,
                PrimitiveType::LongLong | PrimitiveType::Int64 => KhPrim::I64,
                PrimitiveType::UnsignedLongLong | PrimitiveType::UInt64 => KhPrim::U64,
                PrimitiveType::Float => KhPrim::F32,
                PrimitiveType::Double => KhPrim::F64,
                PrimitiveType::LongDouble => return Err("`long double` is not covered".into()),
                PrimitiveType::Fixed { .. } => return Err("`fixed` is not covered".into()),
                PrimitiveType::Void => return Err("`void` has no value".into()),
                PrimitiveType::String | PrimitiveType::WString => return Ok(None),
            },
            IdlType::Named(n) => match self.get(&n) {
                Some(Named::Enum { bit_bound, .. }) => {
                    let prim = match bit_bound.unwrap_or(32) {
                        1..=8 => KhPrim::I8,
                        9..=16 => KhPrim::I16,
                        _ => KhPrim::I32,
                    };
                    return Ok(Some(KhScalar::Enum {
                        name: unqualified(&n).to_string(),
                        prim,
                    }));
                }
                Some(Named::Bitmask { bit_bound }) => {
                    let prim = match bit_bound.unwrap_or(32) {
                        1..=8 => KhPrim::Octet,
                        9..=16 => KhPrim::U16,
                        17..=32 => KhPrim::U32,
                        _ => KhPrim::U64,
                    };
                    return Ok(Some(KhScalar::Bitmask {
                        name: unqualified(&n).to_string(),
                        prim,
                    }));
                }
                Some(_) => return Ok(None),
                None => return Err(format!("type `{n}` is unknown")),
            },
            _ => return Ok(None),
        };
        Ok(Some(KhScalar::Prim(prim)))
    }

    fn node(&self, ty: &IdlType, depth: usize) -> Result<KhNode, String> {
        if let Some(s) = self.scalar(ty)? {
            return Ok(KhNode::Scalar(s));
        }
        match self.resolve(ty) {
            IdlType::Primitive(PrimitiveType::String) => Ok(KhNode::String { bound: None }),
            IdlType::Primitive(PrimitiveType::WString) => Ok(KhNode::WString { bound: None }),
            IdlType::BoundedString { wide: false, bound } => {
                Ok(KhNode::String { bound: Some(bound) })
            }
            IdlType::BoundedString { wide: true, bound } => {
                Ok(KhNode::WString { bound: Some(bound) })
            }
            IdlType::Sequence { inner, bound } => {
                // `sequence<char>` / `sequence<wchar>` included: an element
                // count and the elements, no NUL (rule 11, not rules 3-4).
                match self.scalar(&inner)? {
                    Some(KhScalar::Prim(p)) => Ok(KhNode::Sequence {
                        elem: KhScalar::Prim(p),
                        bound,
                    }),
                    _ => Err(format!(
                        "a sequence of `{}` is not covered (only sequences of primitives are)",
                        inner.to_idl_string()
                    )),
                }
            }
            IdlType::Array { inner, size } => {
                let mut dims = vec![size];
                let mut elem = self.resolve(&inner);
                while let IdlType::Array { inner, size } = elem {
                    dims.push(size);
                    elem = self.resolve(&inner);
                }
                match self.scalar(&elem)? {
                    Some(KhScalar::Prim(p)) => Ok(KhNode::Array {
                        elem: KhScalar::Prim(p),
                        dims,
                    }),
                    _ => Err(format!(
                        "an array of `{}` is not covered (only arrays of primitives are)",
                        elem.to_idl_string()
                    )),
                }
            }
            IdlType::Map { .. } => Err("a map is not covered".into()),
            IdlType::Named(n) => match self.get(&n) {
                Some(Named::Struct(s)) => {
                    let s = s.clone();
                    Ok(KhNode::Struct(self.struct_holder(&s, depth)?))
                }
                Some(Named::Union(u)) => {
                    let u = u.clone();
                    Ok(KhNode::Union(Box::new(self.union_holder(&u, depth)?)))
                }
                _ => Err(format!("type `{n}` is unknown")),
            },
            IdlType::Primitive(p) => Err(format!(
                "`{}` is not covered",
                IdlType::Primitive(p).to_idl_string()
            )),
        }
    }

    /// The `KeyHolder` of a union (its part of an enclosing `KeyHolder`).
    ///
    /// # Errors
    ///
    /// Returns the reason the union is not covered.
    pub fn union_key_holder(&self, u: &Union) -> Result<KhUnion, String> {
        self.union_holder(u, 0)
    }

    fn union_holder(&self, u: &Union, depth: usize) -> Result<KhUnion, String> {
        if depth > MAX_DEPTH {
            return Err(format!("`{}` nests itself through its key", u.name));
        }
        let disc = self
            .scalar(&u.discriminator)?
            .ok_or_else(|| format!("discriminator of `{}` is not a scalar", u.name))?;
        let disc_is_key = u.discriminator_is_key();
        let mut cases = Vec::new();
        for (index, case) in u.cases.iter().enumerate() {
            let node = if disc_is_key
                || matches!(
                    case.field.field_type,
                    IdlType::Primitive(PrimitiveType::Void)
                ) {
                None
            } else {
                Some(
                    self.node(&case.field.field_type, depth + 1)
                        .map_err(|e| format!("case `{}` of `{}`: {e}", case.field.name, u.name))?,
                )
            };
            cases.push(KhCase {
                index,
                labels: case.labels.clone(),
                field: case.field.clone(),
                node,
            });
        }
        Ok(KhUnion {
            name: u.name.clone(),
            disc,
            disc_type: u.discriminator.clone(),
            disc_is_key,
            ext: crate::codegen::data_model::union_ext(u),
            cases,
        })
    }
}

/// Whether a case carries the `default` label.
#[must_use]
pub fn is_default_case(labels: &[UnionLabel]) -> bool {
    labels.iter().any(|l| matches!(l, UnionLabel::Default))
}

/// The `@autoid` among `annotations`, if any.
fn own_autoid(annotations: &[Annotation]) -> Option<AutoIdKind> {
    annotations.iter().find_map(|a| match a {
        Annotation::AutoId(kind) => Some(*kind),
        _ => None,
    })
}

/// Member ids of the members of `chain` (root base first), in that order.
///
/// As `XTypes` 1.3 7.3.1.2.1.1 assigns them, from the structures' own
/// `@autoid` only (see [`TypeIndex::hashes_member_ids`] for the modules').
#[must_use]
pub fn spec_member_ids(chain: &[&Struct]) -> Vec<u32> {
    let modes: Vec<(&Struct, bool)> = chain
        .iter()
        .map(|s| (*s, own_autoid(&s.annotations) == Some(AutoIdKind::Hash)))
        .collect();
    spec_member_ids_with(&modes)
}

/// Member ids of the members of `chain` (root base first, each with whether
/// it hashes its implicit ids).
#[must_use]
pub fn spec_member_ids_with(chain: &[(&Struct, bool)]) -> Vec<u32> {
    let mut ids = Vec::new();
    let mut next: u32 = 0;
    for &(s, hash_mode) in chain {
        for field in &s.fields {
            let explicit = field.annotations.iter().find_map(|a| match a {
                Annotation::Id(id) => Some(*id),
                _ => None,
            });
            let hashid = field.annotations.iter().find_map(|a| match a {
                Annotation::Custom { name, params } if name == "hashid" => Some(
                    params
                        .first()
                        .map(|p| p.value.trim_matches('"').to_string())
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| field.name.clone()),
                ),
                _ => None,
            });
            let id = explicit.unwrap_or_else(|| {
                hashid.map_or_else(
                    || {
                        if hash_mode {
                            name_hash_id(&field.name)
                        } else {
                            next
                        }
                    },
                    |name| name_hash_id(&name),
                )
            });
            next = id.wrapping_add(1);
            ids.push(id);
        }
    }
    ids
}

/// Member id from a name: the first four bytes of its MD5, little endian,
/// masked to 28 bits (`XTypes` 1.3 7.3.1.2.1.1 and 7.2.2.4.4.4.5).
#[must_use]
pub fn name_hash_id(name: &str) -> u32 {
    let d = md5(name.as_bytes());
    u32::from_le_bytes([d[0], d[1], d[2], d[3]]) & 0x0FFF_FFFF
}

/// The 64 additive constants of MD5 (RFC 1321 3.4), shared with the MD5
/// that backends emit into generated code.
pub const MD5_K: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];

/// The 64 per-round left-rotation amounts of MD5 (RFC 1321 3.4).
pub const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

/// MD5 (RFC 1321) of `data`, for generation-time member-name hashes.
#[must_use]
// The single-letter names are RFC 1321's.
#[allow(clippy::many_single_char_names)]
pub fn md5(data: &[u8]) -> [u8; 16] {
    let mut a0: u32 = 0x6745_2301;
    let mut b0: u32 = 0xefcd_ab89;
    let mut c0: u32 = 0x98ba_dcfe;
    let mut d0: u32 = 0x1032_5476;
    let mut msg = data.to_vec();
    let bit_len = u64::try_from(data.len())
        .unwrap_or(u64::MAX)
        .wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut m = [0u32; 16];
        for (i, w) in m.iter_mut().enumerate() {
            *w = u32::from_le_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(MD5_K[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(MD5_S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
    }
    let mut out = [0u8; 16];
    out[..4].copy_from_slice(&a0.to_le_bytes());
    out[4..8].copy_from_slice(&b0.to_le_bytes());
    out[8..12].copy_from_slice(&c0.to_le_bytes());
    out[12..].copy_from_slice(&d0.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::parser::Parser;
    use std::fmt::Write as _;

    fn index(idl: &str) -> TypeIndex {
        let mut p = Parser::try_new(idl).expect("lexer");
        TypeIndex::new(&p.parse().expect("parse"))
    }

    fn holder(idl: &str, name: &str) -> KhStruct {
        let idx = index(idl);
        let s = idx.struct_def(name).expect("struct").clone();
        idx.key_holder(&s).expect("covered").expect("keyed")
    }

    fn hex(b: &[u8]) -> String {
        b.iter().fold(String::new(), |mut out, x| {
            let _ = write!(out, "{x:02x}");
            out
        })
    }

    #[test]
    fn md5_matches_rfc_1321_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&md5(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    /// `XTypes` 1.3 7.3.1.2.1.1 / Annex: "color" has `NameHash` 70 DD A5 DF.
    #[test]
    fn name_hash_of_color_is_the_spec_example() {
        assert_eq!(name_hash_id("color"), 0x0FA5_DD70);
    }

    #[test]
    fn sequential_ids_continue_after_an_explicit_id_and_across_the_base() {
        let idx = index(
            "struct B { long a; @id(10) long b; long c; };
             struct D : B { long d; };",
        );
        let d = idx.struct_def("D").expect("D");
        let chain = idx.base_chain(d).expect("chain");
        assert_eq!(spec_member_ids(&chain), vec![0, 10, 11, 12]);
    }

    #[test]
    fn autoid_hash_uses_the_md5_name_hash() {
        let idx = index("@autoid(HASH) struct H { long color; @id(3) long y; };");
        let h = idx.struct_def("H").expect("H");
        assert_eq!(spec_member_ids(&[h]), vec![0x0FA5_DD70, 3]);
    }

    /// RTPS 2.5 9.6.4.8 Example 2: 4 + 12 + 1 = 17, padded to 20, + 8.
    #[test]
    fn spec_example_2_maximum_is_28() {
        let kh = holder(
            "struct Foo { @key string<12> label; @key long long id; long x; };",
            "Foo",
        );
        assert_eq!(kh.max_size(), Some(28));
        assert!(!kh.is_padded_form());
    }

    /// RTPS 2.5 9.6.4.8 Example 3: member-id order puts the nested key
    /// first; maximum 4 + (4 + 12 + 1) = 21.
    #[test]
    fn spec_example_3_order_and_maximum() {
        let kh = holder(
            "@mutable struct Nested { @key long m_long; long u; long w; };
             @mutable struct Foo { @id(40) @key string<12> label; @id(30) @key Nested m_nested;
                                   @id(20) long x; @id(10) long y; };",
            "Foo",
        );
        let order: Vec<&str> = kh
            .hash_order()
            .iter()
            .map(|m| m.field.name.as_str())
            .collect();
        assert_eq!(order, vec!["m_nested", "label"]);
        assert_eq!(kh.max_size(), Some(21));
    }

    /// A `@mutable` type without `@id`: `XTypes` 1.3 7.3.1.2.1.1 numbers its
    /// members 0, 1, 2 (SEQUENTIAL is the default), so the `KeyHolder` keeps
    /// declaration order (a name hash would order it differently).
    #[test]
    fn mutable_without_id_orders_by_sequential_ids_not_wire_hashes() {
        let kh = holder(
            "@mutable struct MutSeq { @key long a; @key short b; @key string c; };",
            "MutSeq",
        );
        let ids: Vec<u32> = kh.members.iter().map(|m| m.member_id).collect();
        assert_eq!(ids, vec![0, 1, 2]);
        let order: Vec<&str> = kh
            .hash_order()
            .iter()
            .map(|m| m.field.name.as_str())
            .collect();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    /// `@autoid(HASH)` on a containing module hashes the ids of the
    /// structures inside (`XTypes` 1.3 7.3.1.2.1.1); a structure's own
    /// `@autoid` overrides it. The name hash of `shapesize` is below that of
    /// `color`, so it comes first; Cyclone DDS 0.11 gives the same `KeyHash`.
    #[test]
    fn module_autoid_hash_reaches_the_structures_it_contains() {
        let kh = holder(
            "@autoid(HASH) module MH { @final struct ModHashed { @key long color; @key long shapesize; }; };",
            "ModHashed",
        );
        let order: Vec<&str> = kh
            .hash_order()
            .iter()
            .map(|m| m.field.name.as_str())
            .collect();
        assert_eq!(order, vec!["shapesize", "color"]);
        let own = holder(
            "@autoid(HASH) module MH { @autoid(SEQUENTIAL) struct S { @key long color; @key long shapesize; }; };",
            "S",
        );
        let order: Vec<&str> = own
            .hash_order()
            .iter()
            .map(|m| m.field.name.as_str())
            .collect();
        assert_eq!(order, vec!["color", "shapesize"]);
    }

    /// The innermost module's `@autoid` wins over an enclosing one's.
    #[test]
    fn the_innermost_module_autoid_wins() {
        let idl = "@autoid(HASH) module A {
                     @autoid(SEQUENTIAL) module B { struct S1 { @key long color; @key long shapesize; }; };
                     struct S2 { @key long color; @key long shapesize; };
                   };";
        let order = |name: &str| -> Vec<String> {
            holder(idl, name)
                .hash_order()
                .iter()
                .map(|m| m.field.name.clone())
                .collect()
        };
        assert_eq!(order("S1"), vec!["color", "shapesize"]);
        assert_eq!(order("S2"), vec!["shapesize", "color"]);
    }

    /// A key declared in a base type is found through a nested struct key
    /// and through a union case, for the backends that must refuse it.
    #[test]
    fn base_members_are_found_through_nested_structs_and_unions() {
        let idl = "struct BaseK { @key long id; };
                   struct DerivedK : BaseK { @key short sub; };
                   struct Holder { @key DerivedK d; };
                   union UB switch (long) { case 1: DerivedK d; case 2: long l; };
                   struct UHolder { @key UB u; };
                   struct Plain { @key long a; };";
        let found = |name: &str| holder(idl, name).base_member();
        assert_eq!(
            found("Holder"),
            Some(("id".to_string(), "BaseK".to_string()))
        );
        assert_eq!(
            found("UHolder"),
            Some(("id".to_string(), "BaseK".to_string()))
        );
        assert_eq!(found("Plain"), None);
    }

    /// `@non_serialized` members are not part of the type's serialization,
    /// so not of its `KeyHolder` either.
    #[test]
    fn non_serialized_members_are_left_out() {
        let kh = holder(
            "struct In { long a; @non_serialized long b; long c; }; struct Out { @key In p; };",
            "Out",
        );
        let inner = match &kh.members[0].node {
            KhNode::Struct(inner) => Some(inner),
            _ => None,
        }
        .expect("struct node");
        let names: Vec<&str> = inner
            .members
            .iter()
            .map(|m| m.field.name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "c"]);
        assert_eq!(kh.max_size(), Some(8));
    }

    #[test]
    fn an_optional_key_member_is_refused() {
        let idx = index("struct S { @key @optional long a; long b; };");
        let s = idx.struct_def("S").expect("S").clone();
        let err = idx.key_holder(&s).err().unwrap_or_default();
        assert!(err.contains("@optional"), "{err}");
    }

    /// `@hashid("color")` hashes its argument, not the member name; the next
    /// implicit id follows it.
    #[test]
    fn hashid_hashes_its_argument() {
        let kh = holder(
            "struct S { @hashid(\"color\") @key long x; @key long y; };",
            "S",
        );
        let ids: Vec<u32> = kh.members.iter().map(|m| m.member_id).collect();
        assert_eq!(ids, vec![0x0FA5_DD70, 0x0FA5_DD71]);
        let bare = holder("struct S { @hashid @key long color; };", "S");
        assert_eq!(bare.members[0].member_id, 0x0FA5_DD70);
    }

    /// A `wstring<N>` holds at most N UTF-16 code units, two bytes each, and
    /// no NUL: `wstring<6>` is 4 + 12 = 16 (padded), `wstring<7>` 18.
    #[test]
    fn wstring_maximum_counts_two_bytes_per_unit() {
        assert_eq!(
            holder("struct S { @key wstring<6> w; };", "S").max_size(),
            Some(16)
        );
        let past = holder("struct S { @key wstring<7> w; };", "S");
        assert_eq!(past.max_size(), Some(18));
        assert!(!past.is_padded_form());
    }

    /// Every dimension counts: `long a[2][3]` is 6 longs.
    #[test]
    fn multi_dimensional_array_maximum_counts_every_dimension() {
        assert_eq!(
            holder("struct S { @key long a[2][3]; };", "S").max_size(),
            Some(24)
        );
        assert_eq!(
            holder("struct S { @key octet o[3][5]; };", "S").max_size(),
            Some(15)
        );
    }

    #[test]
    fn the_16_byte_boundary_is_padded_and_17_is_not() {
        let at = holder("struct S { @key string<11> l; };", "S");
        let past = holder("struct S { @key string<12> l; };", "S");
        assert_eq!(at.max_size(), Some(16));
        assert!(at.is_padded_form());
        assert_eq!(past.max_size(), Some(17));
        assert!(!past.is_padded_form());
    }

    #[test]
    fn unbounded_members_make_the_maximum_unbounded() {
        assert_eq!(holder("struct S { @key string l; };", "S").max_size(), None);
        assert_eq!(
            holder("struct S { @key sequence<long> l; };", "S").max_size(),
            None
        );
        assert_eq!(
            holder("struct S { @key sequence<short, 3> l; };", "S").max_size(),
            Some(10)
        );
    }

    #[test]
    fn a_nested_struct_without_keys_contributes_all_its_members() {
        let kh = holder(
            "struct In { long a; long b; }; struct Out { @key In p; long x; };",
            "Out",
        );
        let inner = match &kh.members[0].node {
            KhNode::Struct(inner) => Some(inner),
            _ => None,
        }
        .expect("struct node");
        assert_eq!(inner.members.len(), 2);
        assert_eq!(kh.max_size(), Some(8));
    }

    #[test]
    fn enum_and_bitmask_widths_follow_bit_bound() {
        let kh = holder(
            "@bit_bound(8) enum E8 { A, B }; enum E { C };
             @bit_bound(16) bitmask M16 { F }; bitmask M { G };
             struct S { @key E8 a; @key E b; @key M16 c; @key M d; };",
            "S",
        );
        let prims: Vec<KhPrim> = kh
            .members
            .iter()
            .filter_map(|m| match &m.node {
                KhNode::Scalar(s) => Some(s.prim()),
                _ => None,
            })
            .collect();
        assert_eq!(
            prims,
            vec![KhPrim::I8, KhPrim::I32, KhPrim::U16, KhPrim::U32]
        );
    }

    #[test]
    fn uncovered_key_types_are_refused_with_a_reason() {
        let idx = index("struct S { @key sequence<string> l; };");
        let s = idx.struct_def("S").expect("S").clone();
        let err = idx.key_holder(&s).err().unwrap_or_default();
        assert!(err.contains("sequence"), "{err}");
    }

    #[test]
    fn a_keyless_struct_has_no_key_holder() {
        let idx = index("struct S { long a; };");
        let s = idx.struct_def("S").expect("S").clone();
        assert!(idx.key_holder(&s).expect("ok").is_none());
    }
}
