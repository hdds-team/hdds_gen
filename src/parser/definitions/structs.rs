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
            // `struct D : ::B`: an absolute base keeps its leading `::`.
            let absolute = self.check(&TokenKind::DoubleColon);
            if absolute {
                self.advance();
            }
            if let TokenKind::Identifier(base) = &self.current_token().kind {
                let mut base_name = if absolute {
                    format!("::{base}")
                } else {
                    base.clone()
                };
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
            for field in self.parse_members()? {
                struct_def.add_field(field);
            }
        }

        self.expect(&TokenKind::RightBrace, "Expected '}' after struct body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after struct")?;

        Ok(struct_def)
    }

    /// One member with a single declarator, `type name[dims];` (a union
    /// branch, IDL 4.2 7.4.1.4.4.4 `element_spec`).
    pub(crate) fn parse_field(&mut self) -> Result<Field> {
        let mut fields = self.parse_member(false)?;
        Ok(fields.remove(0))
    }

    /// One struct or exception member, `type d1, d2[N], ...;` (IDL 4.2
    /// 7.4.1.4.4.2 `member ::= type_spec declarators ";"`): a field per
    /// declarator, each with the member's type and annotations, an array
    /// declarator wrapping that type for its own name only.
    pub(crate) fn parse_members(&mut self) -> Result<Vec<Field>> {
        self.parse_member(true)
    }

    fn parse_member(&mut self, allow_list: bool) -> Result<Vec<Field>> {
        let mut annotations = Vec::new();
        while self.check(&TokenKind::Annotation) {
            self.advance();
            let annotation = self.parse_annotation()?;
            annotations.push(annotation);
        }

        let base_type = self.parse_type()?;

        let mut fields = Vec::new();
        loop {
            let (name, field_type) = self.parse_declarator(&base_type, "field")?;
            let mut field = Field::new(name, field_type);
            field.annotations.clone_from(&annotations);
            fields.push(field);
            if allow_list && self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(&TokenKind::Semicolon, "Expected ';' after field")?;
        Ok(fields)
    }

    /// A declarator, `name` or `name[N][M]...`, applied to `base`: the array
    /// dimensions wrap `base` right to left, so `[3][4]` is
    /// `Array{3, Array{4, base}}`.
    pub(crate) fn parse_declarator(
        &mut self,
        base: &IdlType,
        what: &str,
    ) -> Result<(String, IdlType)> {
        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                format!("Expected {what} name"),
            ));
        };
        self.advance();

        let mut dimensions = Vec::new();
        while self.check(&TokenKind::LeftBracket) {
            self.advance();
            let size = self.parse_bound_u32("array bound")?;
            self.expect(&TokenKind::RightBracket, "Expected ']' after array size")?;
            dimensions.push(size);
        }

        let mut ty = base.clone();
        for &size in dimensions.iter().rev() {
            ty = IdlType::Array {
                inner: Box::new(ty),
                size,
            };
        }
        Ok((name, ty))
    }
}
