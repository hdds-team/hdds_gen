// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Definition parsers for IDL constructs.
//!
//! Submodules handle structs, enums, unions, typedefs, and other definitions.

#![allow(clippy::redundant_pub_crate)]

pub(crate) mod bitmasks;
pub(crate) mod bitsets;
pub(crate) mod consts;
pub(crate) mod enums;
pub(crate) mod forwards;
pub(crate) mod module;
pub(crate) mod structs;
pub(crate) mod typedefs;
pub(crate) mod unions;

use crate::ast::Definition;
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;
use crate::types::Annotation;

use super::Parser;

impl Parser {
    /// Parse any top-level definition (module, struct, union, etc.); a
    /// typedef with several declarators gives one definition per name.
    pub(super) fn parse_definition(&mut self) -> Result<Vec<Definition>> {
        if let Some(annotation_decl) = self.try_parse_annotation_declaration()? {
            return Ok(vec![annotation_decl]);
        }

        let annotations = self.collect_leading_annotations()?;
        let seq = self.next_seq();
        let defs = self.parse_definition_body(annotations)?;
        for def in &defs {
            if let Definition::Enum(e) = def {
                for v in &e.variants {
                    self.declare(crate::scope::join(&self.scope_stack, &v.name), seq);
                }
            }
            if let Some((name, _)) = crate::scope::declared_name(def) {
                let global = crate::scope::join(&self.scope_stack, name);
                if matches!(
                    def,
                    Definition::Struct(_) | Definition::Union(_) | Definition::Typedef(_)
                ) {
                    self.def_seq.insert(global.clone(), seq);
                }
                self.declare(global, seq);
            }
        }
        Ok(defs)
    }

    fn parse_definition_body(&mut self, annotations: Vec<Annotation>) -> Result<Vec<Definition>> {
        if self.check(&TokenKind::Typedef) {
            return Ok(self
                .parse_typedef()?
                .into_iter()
                .map(|t| {
                    Self::attach_definition_annotations(Definition::Typedef(t), annotations.clone())
                })
                .collect());
        }

        let definition = match self.peek() {
            TokenKind::Module => self.parse_module().map(Definition::Module),
            TokenKind::Struct => self
                .try_parse_struct_forward()
                .map_or_else(|| self.parse_struct().map(Definition::Struct), Ok),
            TokenKind::Enum => self.parse_enum().map(Definition::Enum),
            TokenKind::Union => self
                .try_parse_union_forward()
                .map_or_else(|| self.parse_union().map(Definition::Union), Ok),
            TokenKind::Bitset => self.parse_bitset().map(Definition::Bitset),
            TokenKind::Bitmask => self.parse_bitmask().map(Definition::Bitmask),
            #[cfg(feature = "interfaces")]
            TokenKind::Interface => self.parse_interface().map(Definition::Interface),
            #[cfg(feature = "interfaces")]
            TokenKind::Exception => self.parse_exception().map(Definition::Exception),
            #[cfg(not(feature = "interfaces"))]
            TokenKind::Interface | TokenKind::Exception => Err(ParseError::new(
                ErrorKind::UnexpectedToken,
                self.current_position(),
                "IDL interfaces and exceptions are not supported by this build of \
                 hddsgen (they are parsed only when built with `--features \
                 interfaces`, and no code is generated for them)",
            )),
            TokenKind::Const => self.parse_const().map(Definition::Const),
            _ => Err(ParseError::new(
                ErrorKind::UnexpectedToken,
                self.current_position(),
                format!("Expected definition, found {:?}", self.peek()),
            )),
        }?;

        Ok(vec![Self::attach_definition_annotations(
            definition,
            annotations,
        )])
    }
}
