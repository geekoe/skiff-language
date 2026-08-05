use super::span::parsed_leaf_expr;
use super::*;

const OLD_DB_DOTTED_OPERATIONS: &[&str] = &[
    "get",
    "require",
    "exists",
    "create",
    "createMany",
    "create_many",
    "append",
    "appendMany",
    "append_many",
    "upsert",
    "findMany",
    "find_many",
    "count",
    "transaction",
];

fn is_old_db_dotted_operation(operation: &str) -> bool {
    OLD_DB_DOTTED_OPERATIONS.contains(&operation)
}

impl Parser {
    pub(super) fn parse_db_decl(&mut self) -> Result<DbDecl> {
        let start = self.expect_ident_value("db")?.span.start;
        let kind = if self.match_ident("object") {
            DbDeclKind::Object
        } else if self.match_ident("contract") {
            DbDeclKind::Contract
        } else {
            return Err(CompileError::syntax(
                "expected `db object` or `db contract` declaration",
                self.peek().span.start,
            ));
        };
        let name = self.expect_ident("expected db object name")?;
        self.expect_symbol("{")?;
        let mut collection_name = None;
        let mut key = None;
        let mut retention = None;
        let mut leases = Vec::new();
        let mut storage = Vec::new();
        let mut indexes = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_symbol(";") {
                continue;
            }
            if self.check_db_field_entry() {
                return Err(CompileError::syntax(
                    "db object stored fields must be declared on the attached type",
                    self.peek().span.start,
                ));
            } else if self.match_ident("name") {
                if kind == DbDeclKind::Contract {
                    return Err(CompileError::syntax(
                        "db contract declarations do not declare a collection name; physical storage belongs to the implementing db object",
                        self.previous().span.start,
                    ));
                }
                collection_name = Some(self.expect_string("expected db collection name string")?);
            } else if self.match_ident("retention") {
                if kind == DbDeclKind::Contract {
                    return Err(CompileError::syntax(
                        "db contract declarations do not declare retention; physical storage belongs to the implementing db object",
                        self.previous().span.start,
                    ));
                }
                retention = Some(self.parse_db_retention()?);
            } else if self.match_ident("lease") {
                if kind == DbDeclKind::Contract {
                    return Err(CompileError::syntax(
                        "db contract declarations do not declare leases; physical storage belongs to the implementing db object",
                        self.previous().span.start,
                    ));
                }
                leases.push(self.parse_db_lease_decl()?);
            } else if self.match_ident("storage") {
                if kind == DbDeclKind::Contract {
                    return Err(CompileError::syntax(
                        "db contract declarations do not declare storage mappings; physical storage belongs to the implementing db object",
                        self.previous().span.start,
                    ));
                }
                let field = self.expect_ident("expected db storage field name")?;
                if self.check_symbol(".") {
                    return Err(CompileError::syntax(
                        "db storage field must be a single top-level identifier",
                        self.peek().span.start,
                    ));
                }
                self.expect_ident_value("using")?;
                let codec = match self
                    .expect_ident("expected db storage codec `encrypted`")?
                    .as_str()
                {
                    "encrypted" => DbStorageCodec::Encrypted,
                    _ => {
                        return Err(CompileError::syntax(
                            "expected db storage codec `encrypted`",
                            self.previous().span.start,
                        ));
                    }
                };
                storage.push(DbStorageDecl { field, codec });
            } else if self.match_ident("key") {
                return Err(CompileError::syntax(
                    "db object key type belongs on the attached type; use primary key(field)",
                    self.previous().span.start,
                ));
            } else if self.match_ident("primary") {
                self.expect_ident_value("key")?;
                self.expect_symbol("(")?;
                if key.is_some() {
                    return Err(CompileError::syntax(
                        "db object key is declared more than once",
                        self.previous().span.start,
                    ));
                }
                let key_name = self.expect_ident("expected db primary key field name")?;
                self.expect_symbol(")")?;
                key = Some(DbObjectKey { name: key_name });
            } else if self.match_ident("relation") {
                return Err(CompileError::syntax(
                    "db object relation declarations are not supported in object DB v1",
                    self.previous().span.start,
                ));
            } else if self.match_ident("unique") {
                self.expect_ident_value("index")?;
                indexes.push(self.parse_db_index_entry(&name, true)?);
            } else if self.match_ident("index") {
                indexes.push(self.parse_db_index_entry(&name, false)?);
            } else {
                return Err(CompileError::syntax(
                    "expected db object declaration entry",
                    self.peek().span.start,
                ));
            }
            self.match_symbol(";");
        }
        self.expect_symbol("}")?;
        let end = self.previous().span.end;
        Ok(DbDecl {
            name,
            kind,
            collection_name,
            key,
            retention,
            leases,
            storage,
            indexes,
            span: SourceSpan { start, end },
        })
    }

    pub(super) fn check_db_field_entry(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(_))
            && matches!(
                self.tokens.get(self.current + 1).map(|token| &token.kind),
                Some(TokenKind::Symbol(symbol)) if symbol == ":"
            )
    }

    pub(super) fn parse_db_retention(&mut self) -> Result<DbRetention> {
        let amount = self.expect_positive_integer("expected db retention amount")?;
        let unit = match self.expect_ident("expected db retention unit")?.as_str() {
            "days" => DbRetentionUnit::Days,
            "hours" => DbRetentionUnit::Hours,
            "minutes" => DbRetentionUnit::Minutes,
            "seconds" => DbRetentionUnit::Seconds,
            _ => {
                return Err(CompileError::syntax(
                    "expected db retention unit days, hours, minutes, or seconds",
                    self.previous().span.start,
                ));
            }
        };
        Ok(DbRetention { amount, unit })
    }

    pub(super) fn parse_db_lease_decl(&mut self) -> Result<DbLeaseDecl> {
        let name = self.expect_ident("expected db lease name")?;
        self.expect_ident_value("ttl")?;
        let ttl_ms = self.expect_positive_integer("expected db lease ttl milliseconds")?;
        let max_ms = if self.match_ident("max") {
            Some(self.expect_positive_integer("expected db lease max milliseconds")?)
        } else {
            None
        };
        Ok(DbLeaseDecl {
            name,
            ttl_ms,
            max_ms,
        })
    }

    pub(super) fn parse_db_index_entry(
        &mut self,
        db_name: &str,
        unique: bool,
    ) -> Result<DbIndexEntry> {
        let name = self.expect_ident("expected db index name")?;
        self.expect_symbol("(")?;
        let mut fields = Vec::new();
        if !self.check_symbol(")") {
            loop {
                let field_path = self.parse_field_path("expected db index field")?;
                let direction = self.parse_index_direction();
                fields.push(DbIndexField {
                    field_path,
                    direction,
                });
                if !self.match_symbol(",") {
                    break;
                }
            }
        }
        self.expect_symbol(")")?;
        let where_expr = if self.match_ident("where") {
            let parsed = self.parse_expression()?;
            self.source_spans
                .db_index_wheres
                .push(DbIndexWhereSourceSpans {
                    db_name: db_name.to_string(),
                    index_name: name.clone(),
                    expression: parsed.spans,
                });
            Some(parsed.expr)
        } else {
            None
        };
        Ok(DbIndexEntry {
            name,
            unique,
            fields,
            where_expr,
        })
    }

    pub(super) fn parse_index_direction(&mut self) -> DbIndexDirection {
        if self.match_ident("desc") {
            DbIndexDirection::Desc
        } else {
            self.match_ident("asc");
            DbIndexDirection::Asc
        }
    }

    pub(super) fn parse_field_path(&mut self, message: &str) -> Result<Vec<String>> {
        let mut path = vec![self.expect_ident(message)?];
        while self.match_symbol(".") {
            path.push(self.expect_ident("expected field path segment")?);
        }
        Ok(path)
    }
    pub(super) fn parse_db_expression(&mut self, span: SourceSpan) -> Result<ParsedExpr> {
        if self.check_symbol(".") {
            if let Some(TokenKind::Ident(operation)) =
                self.tokens.get(self.current + 1).map(|token| &token.kind)
            {
                if is_old_db_dotted_operation(operation) {
                    return Err(CompileError::syntax(
                        "old db.transaction/db.* syntax is not supported; use `db transaction` or explicit db operations",
                        self.peek().span.start,
                    ));
                }
            }
            return Ok(parsed_leaf_expr(Expr::Identifier("db".to_string()), span));
        }
        let start = span.start;
        if self.match_ident("transaction") {
            return self.parse_db_transaction_expr(start);
        }
        if self.match_ident("claim") {
            return self.parse_db_lease_claim_expr(start);
        }
        if self.match_ident("lease") {
            return self.parse_db_lease_read_expr(start);
        }
        if self.match_ident("query") {
            return self.parse_db_query_expr(start);
        }
        self.parse_db_operation_expr(start)
    }

    pub(super) fn parse_db_transaction_expr(
        &mut self,
        start: SourceLocation,
    ) -> Result<ParsedExpr> {
        let mode = if self.match_ident("value") {
            DbBlockMode::Value
        } else {
            DbBlockMode::Effect
        };
        let (body_expr, body_spans) = self.parse_block(false)?.into_parts();
        let body_end = body_spans.span.end;
        Ok(ParsedExpr::with_children_and_parts(
            Expr::DbTransaction(DbTransaction {
                mode,
                body: body_expr,
            }),
            SourceSpan {
                start,
                end: body_end,
            },
            Vec::new(),
            vec![body_spans],
            Vec::new(),
        ))
    }

    pub(super) fn parse_db_query_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let target = self.parse_db_operation_target()?;
        let (query, children) = self.parse_db_query_block()?;
        Ok(ParsedExpr::new(
            Expr::DbQuery(DbQuery { target, query }),
            SourceSpan {
                start,
                end: self.previous().span.end,
            },
            children,
        ))
    }

    pub(super) fn parse_db_lease_claim_expr(
        &mut self,
        start: SourceLocation,
    ) -> Result<ParsedExpr> {
        let (target, key, slot, key_spans) = self.parse_db_lease_access()?;
        let binding = if self.match_ident("as") {
            Some(self.expect_ident("expected db claim binding name")?)
        } else {
            None
        };
        let (body_expr, body_spans) = self.parse_block(false)?.into_parts();
        let body_end = body_spans.span.end;
        Ok(ParsedExpr::with_children_and_parts(
            Expr::DbLeaseClaim(DbLeaseClaim {
                target,
                key: Box::new(key),
                slot,
                binding,
                body: body_expr,
            }),
            SourceSpan {
                start,
                end: body_end,
            },
            vec![key_spans],
            vec![body_spans],
            Vec::new(),
        ))
    }

    pub(super) fn parse_db_lease_read_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let (target, key, slot, key_spans) = self.parse_db_lease_access()?;
        Ok(ParsedExpr::new(
            Expr::DbLeaseRead(DbLeaseRead {
                target,
                key: Box::new(key),
                slot,
            }),
            SourceSpan {
                start,
                end: self.previous().span.end,
            },
            vec![key_spans],
        ))
    }

    pub(super) fn parse_db_lease_access(
        &mut self,
    ) -> Result<(TypeRef, Expr, String, ExprSourceSpans)> {
        let target = self.parse_db_operation_target()?;
        self.expect_symbol("(")?;
        let key = self.parse_expression()?;
        self.expect_symbol(")")?;
        self.expect_symbol(".")?;
        let slot = self.expect_ident("expected db lease slot name")?;
        Ok((target, key.expr, slot, key.spans))
    }

    pub(super) fn parse_db_operation_expr(&mut self, start: SourceLocation) -> Result<ParsedExpr> {
        let op_token = self.advance().clone();
        let op = match &op_token.kind {
            TokenKind::Ident(value) => match value.as_str() {
                "find" => DbOperationKind::Find,
                "optional" => DbOperationKind::Optional,
                "require" => DbOperationKind::Require,
                "insert" => DbOperationKind::Insert,
                "update" => DbOperationKind::Update,
                "upsert" => DbOperationKind::Upsert,
                "replace" => DbOperationKind::Replace,
                "delete" => DbOperationKind::Delete,
                "count" => DbOperationKind::Count,
                "exists" => DbOperationKind::Exists,
                _ => {
                    return Err(CompileError::syntax(
                        "expected db operation",
                        op_token.span.start,
                    ));
                }
            },
            _ => {
                return Err(CompileError::syntax(
                    "expected db operation",
                    op_token.span.start,
                ))
            }
        };
        let supports_many = matches!(
            op,
            DbOperationKind::Find
                | DbOperationKind::Insert
                | DbOperationKind::Update
                | DbOperationKind::Delete
        );
        if !supports_many && self.check_ident("many") {
            return Err(CompileError::syntax(
                "many is not supported for this db operation",
                self.peek().span.start,
            ));
        }
        let many = supports_many && self.match_ident("many");
        let target = self.parse_db_operation_target()?;

        let mut selector = None;
        let mut query = None;
        let mut projection = None;
        let mut body = None;
        let mut insert_body = None;
        let mut change = None;
        let mut children = Vec::new();

        match op {
            DbOperationKind::Find | DbOperationKind::Optional | DbOperationKind::Require => {
                if self.match_symbol("(") {
                    if many {
                        return Err(CompileError::syntax(
                            "db many operations do not support key selectors",
                            self.previous().span.start,
                        ));
                    }
                    let (key_expr, key_spans) = self.parse_expression()?.into_parts();
                    self.expect_symbol(")")?;
                    children.push(key_spans);
                    selector = Some(DbSelector::Key {
                        value: Box::new(key_expr),
                    });
                }
                if self.check_symbol("{") {
                    let (read_query, read_projection, read_children) =
                        self.parse_db_read_block()?;
                    if matches!(selector, Some(DbSelector::Key { .. })) && !read_query.is_empty() {
                        return Err(CompileError::syntax(
                            "db key reads only support fields in the following block",
                            op_token.span.start,
                        ));
                    }
                    if !read_query.is_empty() {
                        query = Some(read_query);
                    }
                    projection = read_projection;
                    children.extend(read_children);
                }
            }
            DbOperationKind::Count | DbOperationKind::Exists => {
                if self.match_symbol("(") {
                    let (key_expr, key_spans) = self.parse_expression()?.into_parts();
                    self.expect_symbol(")")?;
                    children.push(key_spans);
                    selector = Some(DbSelector::Key {
                        value: Box::new(key_expr),
                    });
                } else if self.check_symbol("{") {
                    let (parsed_query, query_children) = self.parse_db_query_block()?;
                    selector = Some(DbSelector::Query {
                        query: parsed_query.clone(),
                    });
                    query = Some(parsed_query);
                    children.extend(query_children);
                }
            }
            DbOperationKind::Insert if many => {
                self.expect_ident_value("values")?;
                let (values_expr, values_spans) = self.parse_expression()?.into_parts();
                children.push(values_spans);
                body = Some(DbBody::Values {
                    value: Box::new(values_expr),
                });
            }
            DbOperationKind::Insert => {
                let (fields, field_children) = self.parse_db_object_body()?;
                children.extend(field_children);
                body = Some(DbBody::ObjectFields { fields });
            }
            DbOperationKind::Update => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                if many && matches!(parsed_selector, Some(DbSelector::Key { .. })) {
                    return Err(CompileError::syntax(
                        "db many operations do not support key selectors",
                        op_token.span.start,
                    ));
                }
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
                let (parsed_change, change_children) = self.parse_db_change_block()?;
                change = Some(parsed_change);
                children.extend(change_children);
            }
            DbOperationKind::Upsert => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                if matches!(parsed_selector, Some(DbSelector::Query { .. })) {
                    return Err(CompileError::syntax(
                        "db upsert only supports key selectors in the first Object DB version",
                        op_token.span.start,
                    ));
                }
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
                let (fields, field_children) = self.parse_db_object_body()?;
                insert_body = Some(DbBody::ObjectFields { fields });
                children.extend(field_children);
                let (parsed_change, change_children) = self.parse_db_change_block()?;
                change = Some(parsed_change);
                children.extend(change_children);
            }
            DbOperationKind::Replace => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
                let (fields, field_children) = self.parse_db_object_body()?;
                body = Some(DbBody::ObjectFields { fields });
                children.extend(field_children);
            }
            DbOperationKind::Delete => {
                let (parsed_selector, parsed_query, selector_children) =
                    self.parse_db_selector_or_query()?;
                if many && matches!(parsed_selector, Some(DbSelector::Key { .. })) {
                    return Err(CompileError::syntax(
                        "db many operations do not support key selectors",
                        op_token.span.start,
                    ));
                }
                selector = parsed_selector;
                query = parsed_query;
                children.extend(selector_children);
            }
        }

        Ok(ParsedExpr::new(
            Expr::DbOperation(DbOperation {
                op,
                many,
                target,
                selector,
                query,
                projection,
                body,
                insert_body,
                change,
            }),
            SourceSpan {
                start,
                end: self.previous().span.end,
            },
            children,
        ))
    }

    pub(super) fn parse_db_operation_target(&mut self) -> Result<TypeRef> {
        self.parse_type()
    }

    pub(super) fn parse_db_selector_or_query(
        &mut self,
    ) -> Result<(
        Option<DbSelector>,
        Option<DbQueryBlock>,
        Vec<ExprSourceSpans>,
    )> {
        if self.match_symbol("(") {
            let key = self.parse_expression()?;
            self.expect_symbol(")")?;
            return Ok((
                Some(DbSelector::Key {
                    value: Box::new(key.expr),
                }),
                None,
                vec![key.spans],
            ));
        }
        if self.check_symbol("{") {
            let (query, children) = self.parse_db_query_block()?;
            return Ok((
                Some(DbSelector::Query {
                    query: query.clone(),
                }),
                Some(query),
                children,
            ));
        }
        Ok((None, None, Vec::new()))
    }

    pub(super) fn parse_db_read_block(
        &mut self,
    ) -> Result<(DbQueryBlock, Option<DbProjection>, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut query = DbQueryBlock::default();
        let mut projection = None;
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_statement_terminator() {
                continue;
            }
            if self.match_ident("fields") {
                if !self.check_symbol("{") {
                    return Err(CompileError::syntax(
                        "db read projection now uses `fields { ... }`; replace old `fields name visits` syntax with `fields { name, visits }`",
                        self.peek().span.start,
                    ));
                }
                projection = Some(DbProjection {
                    fields: self.parse_db_projection_fields()?,
                });
            } else if self.check_ident("load") {
                return Err(CompileError::syntax(
                    "db load blocks are not supported in the first Object DB version",
                    self.peek().span.start,
                ));
            } else {
                self.parse_db_query_entry(&mut query, &mut children)?;
            }
        }
        self.expect_symbol("}")?;
        Ok((query, projection, children))
    }

    pub(super) fn parse_db_query_block(&mut self) -> Result<(DbQueryBlock, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut query = DbQueryBlock::default();
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_statement_terminator() {
                continue;
            }
            if self.check_ident("fields") {
                return Err(CompileError::syntax(
                    "fields is only supported on db read operations",
                    self.peek().span.start,
                ));
            }
            if self.check_ident("load") {
                return Err(CompileError::syntax(
                    "db load blocks are not supported in the first Object DB version",
                    self.peek().span.start,
                ));
            }
            self.parse_db_query_entry(&mut query, &mut children)?;
        }
        self.expect_symbol("}")?;
        Ok((query, children))
    }

    pub(super) fn parse_db_query_entry(
        &mut self,
        query: &mut DbQueryBlock,
        children: &mut Vec<ExprSourceSpans>,
    ) -> Result<()> {
        if self.match_ident("where") {
            if self.match_ident("if") {
                let (condition_expr, condition_spans) =
                    self.parse_header_expression()?.into_parts();
                self.expect_symbol("{")?;
                let (predicate_expr, predicate_spans) = self.parse_expression()?.into_parts();
                self.expect_symbol("}")?;
                children.push(condition_spans);
                children.push(predicate_spans);
                query.where_clauses.push(DbWhereClause::Conditional {
                    condition: condition_expr,
                    predicate: predicate_expr,
                });
            } else {
                let (predicate_expr, predicate_spans) = self.parse_expression()?.into_parts();
                children.push(predicate_spans);
                query.where_clauses.push(DbWhereClause::Predicate {
                    predicate: predicate_expr,
                });
            }
        } else if self.match_ident("order") {
            let field = self.parse_db_field_path("expected db order field")?;
            let direction = self.parse_index_direction();
            query.order.push(DbOrderEntry { field, direction });
        } else if self.match_ident("limit") {
            let (limit_expr, limit_spans) = self.parse_expression()?.into_parts();
            children.push(limit_spans);
            query.limit = Some(Box::new(limit_expr));
        } else if self.match_ident("offset") {
            let (offset_expr, offset_spans) = self.parse_expression()?.into_parts();
            children.push(offset_spans);
            query.offset = Some(Box::new(offset_expr));
        } else if self.check_ident("after") {
            return Err(CompileError::syntax(
                "db query after is not supported; use offset",
                self.peek().span.start,
            ));
        } else {
            return Err(CompileError::syntax(
                "expected db query entry",
                self.peek().span.start,
            ));
        }
        let _ = self.match_statement_terminator();
        Ok(())
    }

    pub(super) fn parse_db_projection_fields(&mut self) -> Result<Vec<FieldPath>> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_statement_terminator() {
                continue;
            }
            fields.push(self.parse_db_field_path("expected db fields entry")?);
            let _ = self.match_statement_terminator();
        }
        self.expect_symbol("}")?;
        Ok(fields)
    }

    pub(super) fn parse_db_object_body(
        &mut self,
    ) -> Result<(Vec<DbObjectFieldValue>, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut fields = Vec::new();
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_statement_terminator() {
                continue;
            }
            let field = self.expect_ident("expected db object body field")?;
            self.expect_symbol("=")?;
            let (value_expr, value_spans) = self.parse_expression()?.into_parts();
            children.push(value_spans);
            fields.push(DbObjectFieldValue {
                field,
                value: value_expr,
            });
            let _ = self.match_statement_terminator();
        }
        self.expect_symbol("}")?;
        Ok((fields, children))
    }

    pub(super) fn parse_db_change_block(&mut self) -> Result<(DbChange, Vec<ExprSourceSpans>)> {
        self.expect_symbol("{")?;
        let mut ops = Vec::new();
        let mut children = Vec::new();
        while !self.check_symbol("}") && !self.is_at_end() {
            if self.match_statement_terminator() {
                continue;
            }
            if self.match_ident("unset") {
                let path = self.parse_db_field_path("expected db unset field")?;
                ops.push(DbChangeOp::Unset { path });
            } else if self.match_ident("add") || self.match_ident("addToSet") {
                let path = self.parse_db_field_path("expected db add field")?;
                let (value_expr, value_spans) = self.parse_expression()?.into_parts();
                children.push(value_spans);
                ops.push(DbChangeOp::AddToSet {
                    path,
                    value: value_expr,
                });
            } else if self.match_ident("remove") {
                let path = self.parse_db_field_path("expected db remove field")?;
                let (value_expr, value_spans) = self.parse_expression()?.into_parts();
                children.push(value_spans);
                ops.push(DbChangeOp::Remove {
                    path,
                    value: value_expr,
                });
            } else {
                let path = self.parse_db_field_path("expected db change field")?;
                if path.segments.len() > 1 {
                    return Err(CompileError::syntax(
                        "db change block assignments must use a top-level field",
                        self.previous().span.start,
                    ));
                }
                if self.match_symbol("=") {
                    let (value_expr, value_spans) = self.parse_expression()?.into_parts();
                    children.push(value_spans);
                    ops.push(DbChangeOp::Set {
                        path,
                        value: value_expr,
                    });
                } else if self.match_symbol("+") {
                    self.expect_symbol("=")?;
                    let (value_expr, value_spans) = self.parse_expression()?.into_parts();
                    children.push(value_spans);
                    ops.push(DbChangeOp::Inc {
                        path,
                        value: value_expr,
                    });
                } else if self.match_symbol("-") {
                    self.expect_symbol("=")?;
                    let (value_expr, value_spans) = self.parse_expression()?.into_parts();
                    children.push(value_spans);
                    ops.push(DbChangeOp::Inc {
                        path,
                        value: Expr::Binary {
                            op: BinaryOp::Sub,
                            left: Box::new(Expr::Literal(Literal::Number(0.0))),
                            right: Box::new(value_expr),
                        },
                    });
                } else {
                    return Err(CompileError::syntax(
                        "expected db change operator",
                        self.peek().span.start,
                    ));
                }
            }
            let _ = self.match_statement_terminator();
        }
        self.expect_symbol("}")?;
        Ok((DbChange { ops }, children))
    }

    pub(super) fn parse_db_field_path(&mut self, message: &str) -> Result<FieldPath> {
        let segments = self.parse_field_path(message)?;
        Ok(FieldPath {
            text: segments.join("."),
            segments,
        })
    }
}
