// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Typedef definition parsing.
//!
//! Parses IDL typedef declarations creating type aliases.

use super::Parser;
use crate::ast::Typedef;
use crate::error::Result;
use crate::token::TokenKind;

impl Parser {
    /// `typedef type d1, d2[N][M], ...;` (IDL 4.2 7.4.1.4.3): a typedef per
    /// declarator, an array declarator wrapping the type for its own name.
    pub(super) fn parse_typedef(&mut self) -> Result<Vec<Typedef>> {
        self.expect(&TokenKind::Typedef, "Expected 'typedef'")?;

        let base_type = self.parse_type()?;

        let mut typedefs = Vec::new();
        loop {
            let (name, ty) = self.parse_declarator(&base_type, "typedef")?;
            typedefs.push(Typedef::new(name, ty));
            if self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(&TokenKind::Semicolon, "Expected ';' after typedef")?;

        Ok(typedefs)
    }
}
