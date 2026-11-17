// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Bitset definition parsing.
//!
//! Parses IDL bitset declarations with bitfield members.

use crate::ast::{BitfieldDecl, Bitset};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;

use super::Parser;

impl Parser {
    pub(super) fn parse_bitset(&mut self) -> Result<Bitset> {
        self.expect(&TokenKind::Bitset, "Expected 'bitset'")?;

        let name = if let TokenKind::Identifier(name) = &self.current_token().kind {
            name.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected bitset name",
            ));
        };
        self.advance();

        self.expect(&TokenKind::LeftBrace, "Expected '{' after bitset name")?;

        let mut bitset = Bitset::new(name);

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let mut annotations = Vec::new();
            while self.check(&TokenKind::Annotation) {
                self.advance();
                annotations.push(self.parse_annotation()?);
            }

            match self.peek() {
                TokenKind::Identifier(id) if id == "bitfield" => {
                    self.advance();
                }
                _ => {
                    return Err(ParseError::new(
                        ErrorKind::InvalidSyntax,
                        self.current_position(),
                        "Expected 'bitfield' declaration",
                    ));
                }
            }

            self.expect(&TokenKind::LeftAngle, "Expected '<' after bitfield")?;
            let width = if let TokenKind::IntegerLiteral(n) = self.peek() {
                let literal_pos = self.current_position();
                let raw = *n;
                self.advance();
                Self::literal_u32(raw, literal_pos, "Bitfield width")?
            } else {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    self.current_position(),
                    "Expected bitfield width",
                ));
            };
            self.expect(&TokenKind::RightAngle, "Expected '>' after bitfield width")?;

            // `bitfield<N> a, b, c;` (IDL 4.2 7.4.13.4.3.2): one bitfield per name.
            let mut names = Vec::new();
            loop {
                let TokenKind::Identifier(n) = &self.current_token().kind else {
                    return Err(ParseError::new(
                        ErrorKind::InvalidIdentifier,
                        self.current_position(),
                        "Expected bitfield name",
                    ));
                };
                names.push(n.clone());
                self.advance();
                if self.check(&TokenKind::Comma)
                    && matches!(
                        self.tokens.get(self.current + 1).map(|t| &t.kind),
                        Some(TokenKind::Identifier(_))
                    )
                {
                    self.advance();
                } else {
                    break;
                }
            }

            while self.check(&TokenKind::Comma) {
                self.advance();
                if self.check(&TokenKind::Annotation) {
                    self.advance();
                    annotations.push(self.parse_annotation()?);
                } else {
                    break;
                }
            }

            self.expect(&TokenKind::Semicolon, "Expected ';' after bitfield")?;

            for field_name in names {
                let mut decl = BitfieldDecl::new(width, field_name);
                decl.annotations.clone_from(&annotations);
                bitset.add_field(decl);
            }
        }

        self.expect(&TokenKind::RightBrace, "Expected '}' after bitset body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after bitset")?;
        Ok(bitset)
    }
}
