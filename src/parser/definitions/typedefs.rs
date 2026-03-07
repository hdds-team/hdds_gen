// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Typedef definition parsing.
//!
//! Parses IDL typedef declarations creating type aliases.

use super::Parser;
use crate::ast::Typedef;
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;
use crate::types::IdlType;

impl Parser {
    pub(super) fn parse_typedef(&mut self) -> Result<Typedef> {
        self.expect(&TokenKind::Typedef, "Expected 'typedef'")?;

        let base_type = self.parse_type()?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected typedef name",
            ));
        };
        self.advance();

        // Handle C-style array syntax: typedef double name[9];
        // This wraps the base_type in an Array type
        let final_type = if self.check(&TokenKind::LeftBracket) {
            self.advance(); // consume '['
            let size = self.parse_bound_u32("array bound")?;
            self.expect(&TokenKind::RightBracket, "Expected ']' after array size")?;
            IdlType::Array {
                inner: Box::new(base_type),
                size,
            }
        } else {
            base_type
        };

        self.expect(&TokenKind::Semicolon, "Expected ';' after typedef")?;

        Ok(Typedef::new(name, final_type))
    }
}
