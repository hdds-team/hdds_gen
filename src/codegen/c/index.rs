// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Definition index for C code generation.
//!
//! Indexes all type definitions in the AST for lookup during code generation.

use crate::ast::{Bitmask, Bitset, Definition, Enum, IdlFile, Struct, Typedef, Union};
use std::collections::HashMap;

/// The definitions of an IDL file by name, for lookups while emitting C.
pub struct DefinitionIndex<'a> {
    pub structs: HashMap<String, &'a Struct>,
    pub enums: HashMap<String, &'a Enum>,
    pub typedefs: HashMap<String, &'a Typedef>,
    pub unions: HashMap<String, &'a Union>,
    pub bitsets: HashMap<String, &'a Bitset>,
    pub bitmasks: HashMap<String, &'a Bitmask>,
    /// Every named type, for the `KeyHolder` of keyed types.
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

    #[must_use]
    pub fn from_file(file: &'a IdlFile) -> (Self, Vec<&'a Definition>) {
        let mut index = Self::new();
        index.keys = crate::codegen::key_holder::TypeIndex::new(file);
        let mut flat = Vec::new();
        index.collect(&file.definitions, &mut flat);
        (index, flat)
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
                // Emitted as a `#define`, in declaration order.
                Definition::Const(_) => flat.push(def),
                _ => {}
            }
        }
    }
}
