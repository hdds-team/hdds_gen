// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Definition index for C++ code generation.
//!
//! Indexes all type definitions in the AST for lookup during code generation.

use crate::ast::{Bitmask, Bitset, Definition, Enum, IdlFile, Struct, Typedef, Union};
use std::collections::HashMap;

/// The definitions of an IDL file by name, for lookups while emitting C++.
pub struct DefinitionIndex<'a> {
    pub structs: HashMap<String, &'a Struct>,
    pub enums: HashMap<String, &'a Enum>,
    pub typedefs: HashMap<String, &'a Typedef>,
    pub unions: HashMap<String, &'a Union>,
    pub bitsets: HashMap<String, &'a Bitset>,
    pub bitmasks: HashMap<String, &'a Bitmask>,
    /// Every named type, dependencies included, for the `KeyHolder` of keyed
    /// types.
    pub keys: crate::codegen::key_holder::TypeIndex,
}

impl<'a> DefinitionIndex<'a> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            structs: HashMap::new(),
            enums: HashMap::new(),
            typedefs: HashMap::new(),
            unions: HashMap::new(),
            bitsets: HashMap::new(),
            bitmasks: HashMap::new(),
            keys: crate::codegen::key_holder::TypeIndex::default(),
        }
    }

    /// The enumerator names of the enumeration `name` names (absolute,
    /// qualified or unique: the AST went through `scope::for_codegen`).
    #[must_use]
    pub fn enum_variants(&self, name: &str) -> Option<Vec<String>> {
        match self.keys.named(name) {
            Some(crate::codegen::key_holder::Named::Enum { literals, .. }) => {
                Some(literals.iter().map(|(n, _)| n.clone()).collect())
            }
            _ => None,
        }
    }

    /// Whether `name` names a structure.
    #[must_use]
    pub fn is_struct(&self, name: &str) -> bool {
        self.keys.struct_def(name).is_some()
    }

    /// Whether a struct of this index has a `KeyHolder` the model covers.
    #[must_use]
    pub fn has_key_holder(&self) -> bool {
        self.structs
            .values()
            .any(|s| matches!(self.keys.key_holder(s), Ok(Some(_))))
            || self.unions.values().any(|u| u.discriminator_is_key())
    }

    #[must_use]
    pub fn from_file(file: &'a IdlFile) -> (Self, Vec<&'a Definition>) {
        let mut index = Self::new();
        index.keys.add_file(file);
        let mut flat = Vec::new();
        index.collect(&file.definitions, &mut flat);
        (index, flat)
    }

    /// Index definitions from dependency files (for cross-module type resolution).
    ///
    /// These definitions are added to the lookup index but NOT to the emit list,
    /// so they won't appear in the generated output.
    pub fn index_deps(&mut self, dep_files: &'a [IdlFile]) {
        let mut discarded = Vec::new();
        for dep in dep_files {
            self.keys.add_file(dep);
            self.collect(&dep.definitions, &mut discarded);
        }
    }

    fn collect(&mut self, defs: &'a [Definition], flat: &mut Vec<&'a Definition>) {
        for def in defs {
            match def {
                Definition::Module(m) => self.collect(&m.definitions, flat),
                Definition::Struct(s) => {
                    self.structs.insert(s.name.clone(), s);
                    flat.push(def);
                }
                Definition::Enum(e) => {
                    self.enums.insert(e.name.clone(), e);
                    flat.push(def);
                }
                Definition::Typedef(t) => {
                    self.typedefs.insert(t.name.clone(), t);
                    flat.push(def);
                }
                Definition::Union(u) => {
                    self.unions.insert(u.name.clone(), u);
                    flat.push(def);
                }
                Definition::Bitset(b) => {
                    self.bitsets.insert(b.name.clone(), b);
                    flat.push(def);
                }
                Definition::Bitmask(m) => {
                    self.bitmasks.insert(m.name.clone(), m);
                    flat.push(def);
                }
                _ => {}
            }
        }
    }
}
