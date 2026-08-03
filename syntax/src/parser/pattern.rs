use super::*;

impl Parser {
    pub(super) fn parse_pattern(&mut self) -> Result<Pattern> {
        let mut patterns = vec![self.parse_primary_pattern()?];
        while self.match_symbol("|") {
            patterns.push(self.parse_primary_pattern()?);
        }
        if patterns.len() == 1 {
            Ok(patterns.pop().expect("one pattern"))
        } else {
            Ok(Pattern::Or(patterns))
        }
    }

    pub(super) fn parse_primary_pattern(&mut self) -> Result<Pattern> {
        if self.match_ident("_") {
            return Ok(Pattern::Wildcard);
        }
        if self.match_symbol("{") {
            return Ok(Pattern::Record {
                fields: self.parse_pattern_fields_after_open_brace()?,
            });
        }
        if self.match_ident("true") {
            return Ok(Pattern::Literal(Literal::Bool(true)));
        }
        if self.match_ident("false") {
            return Ok(Pattern::Literal(Literal::Bool(false)));
        }
        if self.match_ident("null") {
            return Ok(Pattern::Literal(Literal::Null));
        }
        if matches!(self.peek().kind, TokenKind::String(_)) {
            let TokenKind::String(value) = self.advance().kind.clone() else {
                unreachable!();
            };
            return Ok(Pattern::Literal(Literal::String(value)));
        }
        if matches!(self.peek().kind, TokenKind::Number(_)) {
            let TokenKind::Number(value) = self.advance().kind.clone() else {
                unreachable!();
            };
            return Ok(Pattern::Literal(Literal::Number(value)));
        }

        let name = self.expect_ident("expected pattern")?;
        let snapshot = self.snapshot();
        let type_args = if self.check_symbol("<") {
            match self.parse_generic_args() {
                Ok(type_args) => type_args,
                Err(_) => {
                    self.restore(snapshot);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };
        if self.match_symbol("{") {
            return Ok(Pattern::Nominal {
                name,
                type_args,
                fields: self.parse_pattern_fields_after_open_brace()?,
            });
        }
        self.restore(snapshot);
        Ok(Pattern::Binding(name))
    }

    pub(super) fn parse_pattern_fields_after_open_brace(&mut self) -> Result<Vec<PatternField>> {
        let mut fields = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            let name = self.expect_ident("expected record pattern field name")?;
            let pattern = if self.match_symbol(":") {
                Some(self.parse_pattern()?)
            } else {
                None
            };
            fields.push(PatternField { name, pattern });
            self.match_symbol(",");
        }
        self.expect_symbol("}")?;
        Ok(fields)
    }
}
