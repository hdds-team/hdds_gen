// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright (c) 2025-2026 naskel.com

//! Interface parsing for CORBA-style IDL.
//!
//! Parses interface definitions with operations, attributes, and exceptions.

use crate::ast::{Attribute, Exception, Interface, Operation, ParamDir, Parameter};
use crate::error::{ErrorKind, ParseError, Result};
use crate::token::TokenKind;

use super::Parser;

impl Parser {
    #[cfg(feature = "interfaces")]
    pub(super) fn parse_interface(&mut self) -> Result<Interface> {
        self.expect(&TokenKind::Interface, "Expected 'interface'")?;
        let name = if let TokenKind::Identifier(n) = &self.current_token().kind {
            n.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected interface name",
            ));
        };
        self.advance();
        let mut base: Option<String> = None;
        if self.check(&TokenKind::Colon) {
            self.advance();
            if let TokenKind::Identifier(b) = &self.current_token().kind {
                base = Some(b.clone());
                self.advance();
            } else {
                return Err(ParseError::new(
                    ErrorKind::InvalidSyntax,
                    self.current_position(),
                    "Expected base interface name",
                ));
            }
        }
        self.expect(&TokenKind::LeftBrace, "Expected '{' after interface name")?;
        let mut iface = Interface::new(name);
        iface.base = base;
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            self.parse_interface_member(&mut iface)?;
        }
        self.expect(&TokenKind::RightBrace, "Expected '}' after interface body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after interface")?;
        Ok(iface)
    }

    #[cfg(feature = "interfaces")]
    pub(super) fn parse_interface_member(&mut self, iface: &mut Interface) -> Result<()> {
        if self.check(&TokenKind::Attribute) || self.check(&TokenKind::Readonly) {
            return self.parse_interface_attribute(iface);
        }

        self.parse_interface_operation(iface)
    }

    #[cfg(feature = "interfaces")]
    pub(super) fn parse_interface_attribute(&mut self, iface: &mut Interface) -> Result<()> {
        let readonly = self.check(&TokenKind::Readonly);
        if readonly {
            self.advance();
        }
        self.expect(&TokenKind::Attribute, "Expected 'attribute'")?;
        let ty = self.parse_type()?;
        let nm = if let TokenKind::Identifier(n) = &self.current_token().kind {
            n.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected attribute name",
            ));
        };
        self.advance();
        self.expect(&TokenKind::Semicolon, "Expected ';' after attribute")?;
        iface.attributes.push(Attribute {
            readonly,
            name: nm,
            ty,
        });
        Ok(())
    }

    #[cfg(feature = "interfaces")]
    pub(super) fn parse_interface_operation(&mut self, iface: &mut Interface) -> Result<()> {
        let oneway = if self.check(&TokenKind::Oneway) {
            self.advance();
            true
        } else {
            false
        };
        let ret = self.parse_type()?;
        let opname = if let TokenKind::Identifier(n) = &self.current_token().kind {
            n.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected operation name",
            ));
        };
        self.advance();
        self.expect(&TokenKind::LeftParen, "Expected '(' after operation name")?;
        let mut params: Vec<Parameter> = Vec::new();
        while !self.check(&TokenKind::RightParen) {
            let dir = match self.peek() {
                TokenKind::In => {
                    self.advance();
                    ParamDir::In
                }
                TokenKind::Out => {
                    self.advance();
                    ParamDir::Out
                }
                TokenKind::Inout => {
                    self.advance();
                    ParamDir::InOut
                }
                _ => ParamDir::In,
            };
            let pty = self.parse_type()?;
            let pname = if let TokenKind::Identifier(n) = &self.current_token().kind {
                n.clone()
            } else {
                return Err(ParseError::new(
                    ErrorKind::InvalidIdentifier,
                    self.current_position(),
                    "Expected parameter name",
                ));
            };
            self.advance();
            params.push(Parameter {
                dir,
                name: pname,
                ty: pty,
            });
            if self.check(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(&TokenKind::RightParen, "Expected ')' after parameters")?;
        let mut raises: Vec<String> = Vec::new();
        if self.check(&TokenKind::Raises) {
            self.advance();
            self.expect(&TokenKind::LeftParen, "Expected '(' after raises")?;
            loop {
                if let TokenKind::Identifier(n) = &self.current_token().kind {
                    raises.push(n.clone());
                    self.advance();
                } else {
                    return Err(ParseError::new(
                        ErrorKind::InvalidIdentifier,
                        self.current_position(),
                        "Expected exception name",
                    ));
                }
                if self.check(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(&TokenKind::RightParen, "Expected ')' after raises list")?;
        }
        self.expect(&TokenKind::Semicolon, "Expected ';' after operation")?;
        if oneway && (params.iter().any(|p| !matches!(p.dir, ParamDir::In)) || !raises.is_empty()) {
            return Err(ParseError::new(
                ErrorKind::InvalidSyntax,
                self.current_position(),
                "oneway operation cannot have out/inout params or raises",
            ));
        }
        iface.operations.push(Operation {
            oneway,
            name: opname,
            return_type: ret,
            params,
            raises,
        });
        Ok(())
    }

    #[cfg(feature = "interfaces")]
    pub(super) fn parse_exception(&mut self) -> Result<Exception> {
        self.expect(&TokenKind::Exception, "Expected 'exception'")?;
        let name = if let TokenKind::Identifier(n) = &self.current_token().kind {
            n.clone()
        } else {
            return Err(ParseError::new(
                ErrorKind::InvalidIdentifier,
                self.current_position(),
                "Expected exception name",
            ));
        };
        self.advance();
        self.expect(&TokenKind::LeftBrace, "Expected '{' after exception name")?;
        let mut ex = Exception {
            name,
            members: Vec::new(),
        };
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            ex.members.extend(self.parse_members()?);
        }
        self.expect(&TokenKind::RightBrace, "Expected '}' after exception body")?;
        self.expect(&TokenKind::Semicolon, "Expected ';' after exception")?;
        Ok(ex)
    }
}
