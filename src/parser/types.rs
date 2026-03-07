// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Type parsing for IDL.
//!
//! Parses primitive types, sequences, arrays, maps, and user-defined type references.

use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;
use crate::types::{IdlType, PrimitiveType};

use super::Parser;

impl Parser {
    /// Parse a type
    #[allow(clippy::too_many_lines)]
    pub(super) fn parse_type(&mut self) -> Result<IdlType> {
        // Handle namespace prefix (::Mod1::Type)
        while self.check(&TokenKind::DoubleColon) {
            self.advance();
            // For now, skip namespace resolution
        }

        match self.peek() {
            // Primitive types
            TokenKind::Void => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Void))
            }
            TokenKind::Boolean => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Boolean))
            }
            TokenKind::Char => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Char))
            }
            TokenKind::WChar => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::WChar))
            }
            TokenKind::Octet => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Octet))
            }
            TokenKind::Short => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Short))
            }
            TokenKind::Long => {
                self.advance();
                // Check for "long double" or "long long"
                if self.check(&TokenKind::Double) {
                    self.advance();
                    Ok(IdlType::Primitive(PrimitiveType::LongDouble))
                } else if self.check(&TokenKind::Long) {
                    self.advance();
                    Ok(IdlType::Primitive(PrimitiveType::LongLong))
                } else {
                    Ok(IdlType::Primitive(PrimitiveType::Long))
                }
            }
            TokenKind::Unsigned => {
                self.advance();
                // Must be followed by short, long, or long long
                match self.peek() {
                    TokenKind::Short => {
                        self.advance();
                        Ok(IdlType::Primitive(PrimitiveType::UInt16))
                    }
                    TokenKind::Long => {
                        self.advance();
                        // Check for "unsigned long long"
                        if self.check(&TokenKind::Long) {
                            self.advance();
                            Ok(IdlType::Primitive(PrimitiveType::UInt64))
                        } else {
                            Ok(IdlType::Primitive(PrimitiveType::UInt32))
                        }
                    }
                    _ => Err(ParseError::new(
                        ErrorKind::UnknownType,
                        self.current_position(),
                        "Expected 'short' or 'long' after 'unsigned'",
                    )),
                }
            }
            TokenKind::UnsignedShort | TokenKind::UInt16 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::UInt16))
            }
            TokenKind::UnsignedLong | TokenKind::UInt32 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::UInt32))
            }
            TokenKind::UnsignedLongLong | TokenKind::UInt64 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::UInt64))
            }
            TokenKind::Float => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Float))
            }
            TokenKind::Double => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Double))
            }
            TokenKind::String => {
                self.advance();
                // Check for bounded string: string<N> or string<CONST>
                if self.check(&TokenKind::LeftAngle) {
                    self.advance(); // consume <
                    let bound = Some(self.parse_bound_u32("string bound")?);
                    self.expect_angle_close("Expected '>' after string bound")?;

                    // Return as a bounded sequence of char (for bounded strings)
                    Ok(IdlType::Sequence {
                        inner: Box::new(IdlType::Primitive(PrimitiveType::Char)),
                        bound,
                    })
                } else {
                    Ok(IdlType::Primitive(PrimitiveType::String))
                }
            }
            TokenKind::WString => {
                self.advance();
                if self.check(&TokenKind::LeftAngle) {
                    self.advance();
                    let bound = Some(self.parse_bound_u32("wstring bound")?);
                    self.expect_angle_close("Expected '>' after wstring bound")?;
                    Ok(IdlType::Sequence {
                        inner: Box::new(IdlType::Primitive(PrimitiveType::WChar)),
                        bound,
                    })
                } else {
                    Ok(IdlType::Primitive(PrimitiveType::WString))
                }
            }
            TokenKind::Int8 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Int8))
            }
            TokenKind::Int16 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Int16))
            }
            TokenKind::Int32 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Int32))
            }
            TokenKind::Int64 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::Int64))
            }
            TokenKind::UInt8 => {
                self.advance();
                Ok(IdlType::Primitive(PrimitiveType::UInt8))
            }
            // Sequence type
            TokenKind::Sequence => {
                self.advance();
                self.expect(&TokenKind::LeftAngle, "Expected '<' after 'sequence'")?;

                let inner = Box::new(self.parse_type()?);

                // Check for bound
                let bound = if self.check(&TokenKind::Comma) {
                    self.advance();
                    Some(self.parse_bound_u32("sequence bound")?)
                } else {
                    None
                };

                self.expect_angle_close("Expected '>' after sequence type")?;

                Ok(IdlType::Sequence { inner, bound })
            }

            // Map type: map<KeyType, ValueType[, N]>
            TokenKind::Map => {
                self.advance();
                self.expect(&TokenKind::LeftAngle, "Expected '<' after 'map'")?;

                let key = Box::new(self.parse_type()?);
                self.expect(&TokenKind::Comma, "Expected ',' in map type")?;
                let value = Box::new(self.parse_type()?);

                // Optional bound: , N or , CONST
                let bound = if self.check(&TokenKind::Comma) {
                    self.advance();
                    let val = self.parse_bound_u32("map bound")?;
                    if val == 0 {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Bound for map<K,V,N> must be > 0",
                        ));
                    }
                    Some(val)
                } else {
                    None
                };

                self.expect_angle_close("Expected '>' after map type")?;

                Ok(IdlType::Map { key, value, bound })
            }
            TokenKind::Fixed => {
                self.advance();
                self.expect(&TokenKind::LeftAngle, "Expected '<' after 'fixed'")?;
                let digits = self.parse_bound_u32("fixed digits")?;
                self.expect(&TokenKind::Comma, "Expected ',' in fixed<d,s>")?;
                let scale = self.parse_bound_u32("fixed scale")?;
                self.expect_angle_close("Expected '>' after fixed<d,s>")?;
                Ok(IdlType::Primitive(PrimitiveType::Fixed { digits, scale }))
            }

            // Named type (typedef or struct)
            TokenKind::Identifier(name) => {
                let mut type_name = name.clone();
                self.advance();

                // Handle qualified names (Namespace::Type)
                while self.check(&TokenKind::DoubleColon) {
                    self.advance(); // consume ::
                    if let TokenKind::Identifier(next) = &self.current_token().kind {
                        type_name.push_str("::");
                        type_name.push_str(next);
                        self.advance();
                    } else {
                        return Err(ParseError::new(
                            ErrorKind::InvalidSyntax,
                            self.current_position(),
                            "Expected identifier after '::'",
                        ));
                    }
                }

                Ok(IdlType::Named(type_name))
            }

            _ => Err(ParseError::new(
                ErrorKind::UnknownType,
                self.current_position(),
                format!("Expected type, found {:?}", self.peek()),
            )),
        }
    }
}
