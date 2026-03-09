// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Module definition parsing.
//!
//! Parses IDL module declarations that provide namespace scoping.

#![allow(clippy::redundant_pub_crate)]

use crate::ast::{Definition, Module};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;

use super::Parser;

impl Parser {
    pub(super) fn parse_module(&mut self) -> Result<Module> {
        self.expect(&TokenKind::Module, "Expected 'module'")?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected module name",
            ));
        };
        self.advance();

        self.expect(&TokenKind::LeftBrace, "Expected '{' after module name")?;

        let mut module = Module::new(name.clone());

        // Push module scope for FQN resolution of constants
        self.scope_stack.push(name);

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            if matches!(
                self.peek(),
                TokenKind::PreprocessorDefine
                    | TokenKind::PreprocessorInclude
                    | TokenKind::PreprocessorIfdef
                    | TokenKind::PreprocessorIfndef
                    | TokenKind::PreprocessorElse
                    | TokenKind::PreprocessorElif
                    | TokenKind::PreprocessorEndif
                    | TokenKind::PreprocessorUndef
                    | TokenKind::PreprocessorPragma
            ) {
                self.advance();
                while !self.is_at_end() {
                    match self.peek() {
                        TokenKind::Module
                        | TokenKind::Struct
                        | TokenKind::Enum
                        | TokenKind::Union
                        | TokenKind::Typedef
                        | TokenKind::Const
                        | TokenKind::Annotation
                        | TokenKind::RightBrace => break,
                        _ => {
                            self.advance();
                        }
                    }
                }
                continue;
            }

            let def = self.parse_definition()?;
            match def {
                Definition::Module(nested) => merge_module_into(&mut module.definitions, nested),
                other => module.add_definition(other),
            }
        }

        self.expect(&TokenKind::RightBrace, "Expected '}' after module body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after module")?;

        // Pop module scope
        self.scope_stack.pop();

        Ok(module)
    }
}

pub(crate) fn merge_module_into(defs: &mut Vec<Definition>, mut incoming: Module) {
    for def in defs.iter_mut() {
        if let Definition::Module(existing) = def {
            if existing.name == incoming.name {
                existing.definitions.append(&mut incoming.definitions);
                return;
            }
        }
    }
    defs.push(Definition::Module(incoming));
}
