// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Enum definition parsing.
//!
//! Parses IDL enum declarations with variants and optional values.

use crate::ast::{Enum, EnumVariant};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;

use super::Parser;

impl Parser {
    pub(super) fn parse_enum(&mut self) -> Result<Enum> {
        self.expect(&TokenKind::Enum, "Expected 'enum'")?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected enum name",
            ));
        };
        self.advance();

        self.expect(&TokenKind::LeftBrace, "Expected '{' after enum name")?;

        let mut enum_def = Enum::new(name);

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let variant_name = if let TokenKind::Identifier(name) = &self.current_token().kind {
                name.clone()
            } else {
                return Err(ParseError::new(
                    ErrorKind::InvalidIdentifier,
                    self.current_position(),
                    "Expected enum variant name",
                ));
            };
            self.advance();

            let value = if self.check(&TokenKind::Equal) {
                self.advance();
                match self.parse_const_expression(0) {
                    Ok(cv) => match cv.as_int(self.current_position()) {
                        Ok(i) => Some(i),
                        Err(_) => {
                            return Err(ParseError::new(
                                ErrorKind::InvalidSyntax,
                                self.current_position(),
                                "Enum value must be an integer constant",
                            ));
                        }
                    },
                    Err(e) => return Err(e),
                }
            } else {
                None
            };

            enum_def.add_variant(EnumVariant::new(variant_name, value));

            if self.check(&TokenKind::Comma) {
                self.advance();
            }
        }

        self.expect(&TokenKind::RightBrace, "Expected '}' after enum body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after enum")?;

        Ok(enum_def)
    }
}
