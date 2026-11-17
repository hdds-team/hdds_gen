// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Const definition parsing.
//!
//! Parses IDL const declarations with type and value.

use super::Parser;
use crate::ast::Const;
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;

impl Parser {
    pub(super) fn parse_const(&mut self) -> Result<Const> {
        self.expect(&TokenKind::Const, "Expected 'const'")?;

        let const_type = self.parse_type()?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected const name",
            ));
        };
        self.advance();

        self.expect(&TokenKind::Equal, "Expected '=' after const name")?;
        let evaluated = self.parse_const_expression(0)?;
        let value = evaluated.to_string();

        self.expect(&TokenKind::Semicolon, "Expected ';' after const")?;

        // Keyed by global name; looked up by the IDL 4.2 scoping rules.
        self.const_env
            .insert(crate::scope::join(&self.scope_stack, &name), evaluated);

        Ok(Const::new(name, const_type, value))
    }
}
