// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Struct definition parsing.
//!
//! Parses IDL struct declarations with fields and annotations.

use crate::ast::{Field, Struct};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;
use crate::types::IdlType;

use super::Parser;

impl Parser {
    pub(super) fn parse_struct(&mut self) -> Result<Struct> {
        self.expect(&TokenKind::Struct, "Expected 'struct'")?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected struct name",
            ));
        };
        self.advance();

        let base_struct = if self.check(&TokenKind::Colon) {
            self.advance();
            if let TokenKind::Identifier(base) = &self.current_token().kind {
                let mut base_name = base.clone();
                self.advance();

                while self.check(&TokenKind::DoubleColon) {
                    self.advance();
                    if let TokenKind::Identifier(next) = &self.current_token().kind {
                        base_name.push_str("::");
                        base_name.push_str(next);
                        self.advance();
                    } else {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected identifier after '::'",
                        ));
                    }
                }

                Some(base_name)
            } else {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    self.current_position(),
                    "Expected base struct name after ':'",
                ));
            }
        } else {
            None
        };

        self.expect(&TokenKind::LeftBrace, "Expected '{' after struct name")?;

        let mut struct_def = Struct::new(name);
        struct_def.base_struct = base_struct;

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let field = self.parse_field()?;
            struct_def.add_field(field);
        }

        self.expect(&TokenKind::RightBrace, "Expected '}' after struct body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after struct")?;

        Ok(struct_def)
    }

    pub(crate) fn parse_field(&mut self) -> Result<Field> {
        let mut annotations = Vec::new();
        while self.check(&TokenKind::Annotation) {
            self.advance();
            let annotation = self.parse_annotation()?;
            annotations.push(annotation);
        }

        let mut field_type = self.parse_type()?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected field name",
            ));
        };
        self.advance();

        // Collect all array dimensions first
        let mut dimensions = Vec::new();
        while self.check(&TokenKind::LeftBracket) {
            self.advance();

            let size = self.parse_bound_u32("array bound")?;

            self.expect(&TokenKind::RightBracket, "Expected ']' after array size")?;
            dimensions.push(size);
        }

        // Build nested structure from LAST dimension to FIRST (right-to-left)
        // This ensures [3][4] becomes Array{size:3, inner: Array{size:4, inner: long}}
        // not Array{size:4, inner: Array{size:3, inner: long}}
        for &size in dimensions.iter().rev() {
            field_type = IdlType::Array {
                inner: Box::new(field_type),
                size,
            };
        }

        self.expect(&TokenKind::Semicolon, "Expected ';' after field")?;

        let mut field = Field::new(name, field_type);
        field.annotations = annotations;

        Ok(field)
    }
}
