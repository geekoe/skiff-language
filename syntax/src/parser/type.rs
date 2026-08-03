use super::*;

impl Parser {
    pub(super) fn parse_type(&mut self) -> Result<TypeRef> {
        let mut name = self.parse_nullable_type()?;
        while self.match_symbol("|") {
            let rhs = self.parse_nullable_type()?;
            name.push_str(" | ");
            name.push_str(&rhs);
        }
        Ok(TypeRef { name })
    }

    pub(super) fn parse_nullable_type(&mut self) -> Result<String> {
        let mut name = self.parse_primary_type()?;
        while self.match_symbol("?") {
            name.push('?');
        }
        Ok(name)
    }

    pub(super) fn parse_primary_type(&mut self) -> Result<String> {
        if self.match_ident("any") {
            let interface = self.parse_primary_type()?;
            return Ok(format!("any {interface}"));
        }
        if self.check_ident("fn") {
            return self.parse_function_type_name();
        }
        if self.match_symbol("{") {
            return self.parse_record_type_name();
        }

        let token = self.advance().clone();
        let mut name = match token.kind {
            TokenKind::Ident(value) => value,
            TokenKind::String(value) => quote_string_type(&value),
            _ => return Err(CompileError::syntax("expected type name", token.span.start)),
        };
        if self.match_symbol("/") {
            name.push('/');
            name.push_str(&self.expect_ident("expected source module after dependency /")?);
        }
        while self.match_symbol(".") {
            name.push('.');
            name.push_str(&self.expect_ident("expected qualified type segment")?);
        }

        if self.match_symbol("<") {
            name.push('<');
            if !self.check_symbol(">") {
                loop {
                    name.push_str(&self.parse_type()?.name);
                    if !self.match_symbol(",") {
                        break;
                    }
                    name.push_str(", ");
                }
            }
            self.expect_symbol(">")?;
            name.push('>');
        }
        Ok(name)
    }

    pub(super) fn parse_function_type_name(&mut self) -> Result<String> {
        self.expect_ident_value("fn")?;
        self.expect_symbol("(")?;
        let params = self.parse_params()?;
        self.expect_symbol(")")?;
        self.expect_symbol("->")?;
        let return_type = self.parse_type()?;
        let params = params
            .iter()
            .map(|param| format!("{}: {}", param.name, param.ty.name))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!("fn({params}) -> {}", return_type.name))
    }

    pub(super) fn parse_record_type_name(&mut self) -> Result<String> {
        let mut fields = Vec::new();
        if !self.check_symbol("}") {
            loop {
                let field = self.expect_ident("expected record field name")?;
                self.expect_symbol(":")?;
                let ty = self.parse_type()?;
                fields.push(format!("{field}: {}", ty.name));
                if !self.match_symbol(",") {
                    break;
                }
                if self.check_symbol("}") {
                    break;
                }
            }
        }
        self.expect_symbol("}")?;
        if fields.is_empty() {
            Ok("{}".to_string())
        } else {
            Ok(format!("{{ {} }}", fields.join(", ")))
        }
    }
}

fn quote_string_type(value: &str) -> String {
    let escaped = value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other => vec![other],
        })
        .collect::<String>();
    format!("\"{escaped}\"")
}
