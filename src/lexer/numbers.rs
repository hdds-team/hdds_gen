// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Numeric literal lexing.
//!
//! Handles integer, hex, octal, and floating-point literals.

use super::state::Lexer;
use crate::error::{ErrorKind, ParseError, Position, Result};
use crate::token::{Token, TokenKind};

impl Lexer {
    pub(crate) fn read_number(&mut self) -> Result<Token> {
        let start_pos = self.current_position();
        let mut lexeme = String::new();
        let mut is_float = false;
        let mut seen_exponent = false;

        if self.peek() == Some('0') {
            if let Some(next) = self.peek_ahead(1) {
                match next {
                    'x' | 'X' => return self.read_hex_literal(start_pos),
                    'b' | 'B' => return self.read_binary_literal(start_pos),
                    'o' | 'O' => return self.read_octal_literal(start_pos),
                    c if c.is_ascii_digit() => return self.read_leading_zero_literal(start_pos),
                    _ => {}
                }
            }
        }

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                lexeme.push(ch);
                self.advance();
            } else if ch == '.'
                && !is_float
                && self.peek_ahead(1).is_some_and(|c| c.is_ascii_digit())
            {
                is_float = true;
                lexeme.push(ch);
                self.advance();
            } else if matches!(ch, 'e' | 'E')
                && !seen_exponent
                && (self.peek_ahead(1).is_some_and(|c| c.is_ascii_digit())
                    || (self.peek_ahead(1).is_some_and(|c| matches!(c, '+' | '-'))
                        && self.peek_ahead(2).is_some_and(|c| c.is_ascii_digit())))
            {
                // The exponent of a floating-point literal (IDL 4.2 7.2.6.4),
                // after an integer part (`1e3`) or a fraction (`1.5e3`).
                is_float = true;
                seen_exponent = true;
                lexeme.push(ch);
                self.advance();
                if let Some(sign) = self.peek() {
                    if matches!(sign, '+' | '-') {
                        lexeme.push(sign);
                        self.advance();
                    }
                }
            } else {
                break;
            }
        }

        let kind = if is_float {
            let value = lexeme.parse::<f64>().map_err(|_| {
                ParseError::new(
                    ErrorKind::InvalidSyntax,
                    start_pos,
                    format!("Invalid float literal: {lexeme}"),
                )
            })?;
            TokenKind::FloatLiteral(value)
        } else {
            let value = lexeme.parse::<i64>().map_err(|_| {
                ParseError::new(
                    ErrorKind::InvalidSyntax,
                    start_pos,
                    format!("Invalid integer literal: {lexeme}"),
                )
            })?;
            TokenKind::IntegerLiteral(value)
        };

        Ok(Token::new(kind, lexeme, start_pos))
    }

    fn read_hex_literal(&mut self, start_pos: Position) -> Result<Token> {
        let mut digits = String::new();
        let mut lexeme = String::from("0x");

        self.advance();
        self.advance();

        while let Some(ch) = self.peek() {
            if ch.is_ascii_hexdigit() {
                digits.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if digits.is_empty() {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                "Invalid hex literal",
            ));
        }

        let value = i64::from_str_radix(&digits, 16).map_err(|_| {
            ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                format!("Invalid hex literal: 0x{digits}"),
            )
        })?;
        lexeme.push_str(&digits);
        Ok(Token::new(
            TokenKind::IntegerLiteral(value),
            lexeme,
            start_pos,
        ))
    }

    fn read_binary_literal(&mut self, start_pos: Position) -> Result<Token> {
        let mut digits = String::new();
        let mut lexeme = String::from("0b");

        self.advance();
        self.advance();

        while let Some(ch) = self.peek() {
            if matches!(ch, '0' | '1') {
                digits.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if digits.is_empty() {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                "Invalid binary literal",
            ));
        }

        let value = i64::from_str_radix(&digits, 2).map_err(|_| {
            ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                format!("Invalid binary literal: 0b{digits}"),
            )
        })?;
        lexeme.push_str(&digits);
        Ok(Token::new(
            TokenKind::IntegerLiteral(value),
            lexeme,
            start_pos,
        ))
    }

    fn read_octal_literal(&mut self, start_pos: Position) -> Result<Token> {
        let mut digits = String::new();
        let mut lexeme = String::from("0o");

        self.advance();
        self.advance();

        while let Some(ch) = self.peek() {
            if ('0'..='7').contains(&ch) {
                digits.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if digits.is_empty() {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                "Invalid octal literal",
            ));
        }

        let value = i64::from_str_radix(&digits, 8).map_err(|_| {
            ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                format!("Invalid octal literal: 0o{digits}"),
            )
        })?;
        lexeme.push_str(&digits);
        Ok(Token::new(
            TokenKind::IntegerLiteral(value),
            lexeme,
            start_pos,
        ))
    }

    fn read_leading_zero_literal(&mut self, start_pos: Position) -> Result<Token> {
        self.advance();
        let mut digits = String::from("0");

        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() {
                digits.push(ch);
                self.advance();
            } else {
                break;
            }
        }

        if digits.chars().skip(1).all(|d| matches!(d, '0'..='7')) && digits.len() > 1 {
            let oct = &digits[1..];
            let value = i64::from_str_radix(oct, 8).map_err(|_| {
                ParseError::new(
                    ErrorKind::InvalidSyntax,
                    start_pos,
                    format!("Invalid octal literal: {digits}"),
                )
            })?;
            return Ok(Token::new(
                TokenKind::IntegerLiteral(value),
                digits,
                start_pos,
            ));
        }

        let value = digits.parse::<i64>().map_err(|_| {
            ParseError::new(
                ErrorKind::InvalidSyntax,
                start_pos,
                format!("Invalid integer literal: {digits}"),
            )
        })?;
        Ok(Token::new(
            TokenKind::IntegerLiteral(value),
            digits,
            start_pos,
        ))
    }
}
