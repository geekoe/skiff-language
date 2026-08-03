use super::*;

pub(super) struct ParsedBlock {
    pub(super) block: Block,
    pub(super) spans: BlockSourceSpans,
}

pub(super) struct ParsedStmt {
    pub(super) stmt: Stmt,
    pub(super) spans: StmtSourceSpans,
}

pub(super) struct ParsedExpr {
    pub(super) expr: Expr,
    pub(super) spans: ExprSourceSpans,
}

pub(super) fn parsed_leaf_expr(expr: Expr, span: SourceSpan) -> ParsedExpr {
    ParsedExpr::new(expr, span, Vec::new())
}

impl ParsedExpr {
    pub(super) fn new(expr: Expr, span: SourceSpan, children: Vec<ExprSourceSpans>) -> ParsedExpr {
        ParsedExpr {
            expr,
            spans: expr_source_spans(span, children),
        }
    }

    pub(super) fn into_parts(self) -> (Expr, ExprSourceSpans) {
        (self.expr, self.spans)
    }

    pub(super) fn with_children_and_parts(
        expr: Expr,
        span: SourceSpan,
        children: Vec<ExprSourceSpans>,
        blocks: Vec<BlockSourceSpans>,
        record_fields: Vec<RecordFieldSourceSpans>,
    ) -> ParsedExpr {
        ParsedExpr {
            expr,
            spans: ExprSourceSpans {
                span,
                children,
                blocks,
                record_fields,
            },
        }
    }
}

impl ParsedBlock {
    pub(super) fn from_stmt(stmt: ParsedStmt) -> ParsedBlock {
        let ParsedStmt { stmt, spans } = stmt;
        ParsedBlock {
            spans: BlockSourceSpans {
                span: spans.span,
                statements: vec![spans],
            },
            block: Block {
                statements: vec![stmt],
            },
        }
    }

    pub(super) fn into_parts(self) -> (Block, BlockSourceSpans) {
        (self.block, self.spans)
    }
}

impl ParsedStmt {
    pub(super) fn new(
        stmt: Stmt,
        span: SourceSpan,
        expressions: Vec<ExprSourceSpans>,
        blocks: Vec<BlockSourceSpans>,
    ) -> ParsedStmt {
        ParsedStmt {
            stmt,
            spans: StmtSourceSpans {
                span,
                expressions,
                blocks,
            },
        }
    }

    pub(super) fn expr(expression: ParsedExpr) -> ParsedStmt {
        Self::new(
            Stmt::Expr(expression.expr),
            expression.spans.span,
            vec![expression.spans],
            Vec::new(),
        )
    }

    pub(super) fn leaf(stmt: Stmt, span: SourceSpan) -> ParsedStmt {
        Self::new(stmt, span, Vec::new(), Vec::new())
    }

    pub(super) fn with_expression(
        stmt: Stmt,
        span: SourceSpan,
        expression: ExprSourceSpans,
    ) -> ParsedStmt {
        Self::new(stmt, span, vec![expression], Vec::new())
    }

    pub(super) fn with_block(stmt: Stmt, span: SourceSpan, block: BlockSourceSpans) -> ParsedStmt {
        Self::new(stmt, span, Vec::new(), vec![block])
    }

    pub(super) fn with_expression_and_block(
        stmt: Stmt,
        span: SourceSpan,
        expression: ExprSourceSpans,
        block: BlockSourceSpans,
    ) -> ParsedStmt {
        Self::new(stmt, span, vec![expression], vec![block])
    }
}

pub(super) fn expr_source_spans(
    span: SourceSpan,
    children: Vec<ExprSourceSpans>,
) -> ExprSourceSpans {
    ExprSourceSpans {
        span,
        children,
        blocks: Vec::new(),
        record_fields: Vec::new(),
    }
}
