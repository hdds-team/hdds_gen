// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Definition index for C code generation.
//!
//! Indexes all type definitions in the AST for lookup during code generation.

use crate::ast::{Bitmask, Bitset, Definition, Enum, IdlFile, Struct, Typedef, Union};
use crate::types::{IdlType, PrimitiveType};
use std::collections::HashMap;

use super::helpers::last_ident;

pub struct DefinitionIndex<'a> {
    pub structs: HashMap<String, &'a Struct>,
    pub enums: HashMap<String, &'a Enum>,
    pub typedefs: HashMap<String, &'a Typedef>,
    pub unions: HashMap<String, &'a Union>,
    pub bitsets: HashMap<String, &'a Bitset>,
    pub bitmasks: HashMap<String, &'a Bitmask>,
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
        }
    }

    #[must_use]
    pub fn from_file(file: &'a IdlFile) -> (Self, Vec<&'a Definition>) {
        let mut index = Self::new();
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
                _ => {}
            }
        }
    }

    #[must_use]
    pub fn align_of(&self, ty: &IdlType) -> usize {
        match ty {
            IdlType::Primitive(p) => match p {
                PrimitiveType::Octet
                | PrimitiveType::UInt8
                | PrimitiveType::Int8
                | PrimitiveType::Boolean
                | PrimitiveType::Char => 1,
                PrimitiveType::Short
                | PrimitiveType::Int16
                | PrimitiveType::UnsignedShort
                | PrimitiveType::UInt16 => 2,
                PrimitiveType::Long
                | PrimitiveType::Int32
                | PrimitiveType::UnsignedLong
                | PrimitiveType::UInt32
                | PrimitiveType::Float
                | PrimitiveType::WChar
                | PrimitiveType::String
                | PrimitiveType::WString
                | PrimitiveType::Void
                | PrimitiveType::Fixed { .. } => 4,
                // XCDR2 §7.4.3.4.1 Tab.15: 8-byte primitives align on 4 (cap-4),
                // not 8. Used by `encode_array` to pre-align before the loop;
                // mismatched value here would make `array<double>` /
                // `array<int64>` re-introduce the F01 alignment bug through
                // a different code path than `primitive_scalar_layout`.
                PrimitiveType::LongLong
                | PrimitiveType::Int64
                | PrimitiveType::UnsignedLongLong
                | PrimitiveType::UInt64
                | PrimitiveType::Double
                | PrimitiveType::LongDouble => 4,
            },
            IdlType::Sequence { .. } | IdlType::Map { .. } => 4,
            IdlType::Array { inner, .. } => self.align_of(inner),
            IdlType::Named(nm) => {
                let ident = last_ident(nm);
                if self.enums.contains_key(ident)
                    || self.structs.contains_key(ident)
                    || self.bitsets.contains_key(ident)
                    || self.bitmasks.contains_key(ident)
                {
                    4
                } else if let Some(td) = self.typedefs.get(ident) {
                    self.align_of(&td.base_type)
                } else {
                    4
                }
            }
        }
    }
}
