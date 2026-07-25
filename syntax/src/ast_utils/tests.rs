use super::*;
use crate::ast::{
    BinaryOp, Block, Expr, Literal, MatchArm, ObjectLiteralEntry, ObjectLiteralKey, Pattern,
    PatternField, Stmt, TestEffectDeclaration, TestEffectOutcome, TestEffectSequenceStep,
    TestEffectStepOutcome, TypeRef,
};

#[test]
fn block_contains_expr_traverses_nested_blocks() {
    let expr = Expr::Binary {
        op: BinaryOp::Add,
        left: Box::new(Expr::Literal(Literal::Number(1.0))),
        right: Box::new(Expr::Field {
            object: Box::new(Expr::Identifier("connect".to_string())),
            field: "mongo".to_string(),
        }),
    };
    let block = Block {
        statements: vec![Stmt::If {
            condition: Expr::Identifier("flag".to_string()),
            then_block: Block {
                statements: vec![Stmt::Expr(expr)],
            },
            else_block: None,
        }],
    };

    let mut hit_field = false;
    let found = block_contains_expr(&block, &mut |expr| {
        hit_field = matches!(expr, Expr::Field { field, .. } if field == "mongo");
        hit_field
    });
    assert!(found);
    assert!(hit_field);
}

#[test]
fn pattern_contains_traverses_nested_patterns() {
    let pattern = Pattern::Nominal {
        name: "Root".to_string(),
        type_args: vec![TypeRef {
            name: "RootType".to_string(),
        }],
        fields: vec![PatternField {
            name: "field".to_string(),
            pattern: Some(Pattern::Record {
                fields: vec![PatternField {
                    name: "nested".to_string(),
                    pattern: Some(Pattern::Binding("value".to_string())),
                }],
            }),
        }],
    };

    let mut binding_name = None;
    let found = pattern_contains(&pattern, &mut |pattern| {
        if let Pattern::Binding(name) = pattern {
            binding_name = Some(name.clone());
            true
        } else {
            false
        }
    });

    assert!(found);
    assert_eq!(binding_name.as_deref(), Some("value"));
}

#[test]
fn read_only_walker_visits_catch_and_object_literal_value_in_order() {
    #[derive(Default)]
    struct LoggingVisitor {
        events: Vec<String>,
    }

    impl AstVisitor for LoggingVisitor {
        fn visit_expr(&mut self, expr: &Expr) {
            match expr {
                Expr::Catch { .. } => self.events.push("expr:catch".to_string()),
                Expr::ObjectLiteral { .. } => self.events.push("expr:object".to_string()),
                Expr::Identifier(name) => self.events.push(format!("expr:{name}")),
                _ => {}
            }
            walk_expr(self, expr);
        }

        fn visit_type_ref(&mut self, ty: &TypeRef) {
            self.events.push(format!("type:{}", ty.name));
        }
    }

    let expr = Expr::Catch {
        catch_type: TypeRef {
            name: "Error".to_string(),
        },
        try_expr: Box::new(Expr::ObjectLiteral {
            entries: vec![ObjectLiteralEntry {
                key: ObjectLiteralKey::Name("key".to_string()),
                key_span: None,
                value: Expr::Identifier("value".to_string()),
            }],
        }),
    };

    let mut visitor = LoggingVisitor::default();
    visitor.visit_expr(&expr);

    assert_eq!(
        visitor.events,
        vec!["expr:catch", "type:Error", "expr:object", "expr:value",]
    );
}

#[test]
fn mutable_walker_traverses_match_patterns_catch_and_object_literal_values() {
    struct RenamingVisitor;

    impl AstVisitorMut for RenamingVisitor {
        fn visit_expr(&mut self, expr: &mut Expr) {
            if let Expr::Identifier(name) = expr {
                name.push_str("_seen");
                return;
            }
            walk_expr_mut(self, expr);
        }

        fn visit_pattern(&mut self, pattern: &mut Pattern) {
            if let Pattern::Binding(name) = pattern {
                name.push_str("_seen");
                return;
            }
            walk_pattern_mut(self, pattern);
        }

        fn visit_type_ref(&mut self, ty: &mut TypeRef) {
            ty.name.push_str("_seen");
        }
    }

    let mut block = Block {
        statements: vec![Stmt::Match {
            value: Expr::Identifier("input".to_string()),
            arms: vec![MatchArm {
                pattern: Pattern::Nominal {
                    name: "Node".to_string(),
                    type_args: vec![TypeRef {
                        name: "Payload".to_string(),
                    }],
                    fields: vec![PatternField {
                        name: "field".to_string(),
                        pattern: Some(Pattern::Binding("binding".to_string())),
                    }],
                },
                body: Block {
                    statements: vec![Stmt::Expr(Expr::Catch {
                        catch_type: TypeRef {
                            name: "Error".to_string(),
                        },
                        try_expr: Box::new(Expr::ObjectLiteral {
                            entries: vec![ObjectLiteralEntry {
                                key: ObjectLiteralKey::Name("computed".to_string()),
                                key_span: None,
                                value: Expr::Identifier("value".to_string()),
                            }],
                        }),
                    })],
                },
            }],
        }],
    };

    let mut visitor = RenamingVisitor;
    visitor.visit_block(&mut block);

    let Stmt::Match { value, arms } = &block.statements[0] else {
        panic!("expected match statement");
    };
    assert!(matches!(value, Expr::Identifier(name) if name == "input_seen"));
    let Pattern::Nominal {
        type_args, fields, ..
    } = &arms[0].pattern
    else {
        panic!("expected nominal pattern");
    };
    assert_eq!(type_args[0].name, "Payload_seen");
    assert!(
        matches!(fields[0].pattern.as_ref(), Some(Pattern::Binding(name)) if name == "binding_seen")
    );
    let Stmt::Expr(Expr::Catch {
        catch_type,
        try_expr,
    }) = &arms[0].body.statements[0]
    else {
        panic!("expected catch expression");
    };
    assert_eq!(catch_type.name, "Error_seen");
    let Expr::ObjectLiteral { entries } = try_expr.as_ref() else {
        panic!("expected object literal");
    };
    assert!(matches!(&entries[0].key, ObjectLiteralKey::Name(name) if name == "computed"));
    assert!(matches!(&entries[0].value, Expr::Identifier(name) if name == "value_seen"));
}

#[test]
fn read_only_walker_can_override_match_arm_pattern_traversal() {
    #[derive(Default)]
    struct BodyOnlyVisitor {
        saw_pattern: bool,
        saw_body_expr: bool,
    }

    impl AstVisitor for BodyOnlyVisitor {
        fn visit_pattern(&mut self, _pattern: &Pattern) {
            self.saw_pattern = true;
        }

        fn visit_match_arm(&mut self, arm: &MatchArm) {
            self.visit_block(&arm.body);
        }

        fn visit_expr(&mut self, expr: &Expr) {
            if matches!(expr, Expr::Identifier(name) if name == "body") {
                self.saw_body_expr = true;
            }
            walk_expr(self, expr);
        }
    }

    let stmt = Stmt::Match {
        value: Expr::Identifier("input".to_string()),
        arms: vec![MatchArm {
            pattern: Pattern::Binding("ignored".to_string()),
            body: Block {
                statements: vec![Stmt::Expr(Expr::Identifier("body".to_string()))],
            },
        }],
    };

    let mut visitor = BodyOnlyVisitor::default();
    visitor.visit_stmt(&stmt);

    assert!(!visitor.saw_pattern);
    assert!(visitor.saw_body_expr);
}

#[test]
fn test_effect_walkers_visit_every_embedded_expression() {
    #[derive(Default)]
    struct Names(Vec<String>);

    impl AstVisitor for Names {
        fn visit_expr(&mut self, expr: &Expr) {
            if let Expr::Identifier(name) = expr {
                self.0.push(name.clone());
            }
            walk_expr(self, expr);
        }
    }

    let mut effects = vec![
        TestEffectDeclaration {
            target: "std.http.respond".to_string(),
            expect: Some(Expr::Identifier("respond_expect".to_string())),
            outcome: TestEffectOutcome::Respond {
                value: Expr::Identifier("respond".to_string()),
            },
            span: crate::error::SourceSpan::synthetic(),
        },
        TestEffectDeclaration {
            target: "std.http.throw".to_string(),
            expect: None,
            outcome: TestEffectOutcome::Throw {
                value: Expr::Identifier("throw".to_string()),
            },
            span: crate::error::SourceSpan::synthetic(),
        },
        TestEffectDeclaration {
            target: "std.http.stream".to_string(),
            expect: None,
            outcome: TestEffectOutcome::Stream {
                events: vec![Expr::Identifier("event".to_string())],
            },
            span: crate::error::SourceSpan::synthetic(),
        },
        TestEffectDeclaration {
            target: "std.http.sequence".to_string(),
            expect: Some(Expr::Identifier("sequence_expect".to_string())),
            outcome: TestEffectOutcome::Sequence {
                steps: vec![
                    TestEffectSequenceStep {
                        expect: Some(Expr::Identifier("step_expect".to_string())),
                        outcome: TestEffectStepOutcome::Respond {
                            value: Expr::Identifier("step_respond".to_string()),
                        },
                    },
                    TestEffectSequenceStep {
                        expect: None,
                        outcome: TestEffectStepOutcome::Throw {
                            value: Expr::Identifier("step_throw".to_string()),
                        },
                    },
                    TestEffectSequenceStep {
                        expect: None,
                        outcome: TestEffectStepOutcome::Stream {
                            events: vec![Expr::Identifier("step_event".to_string())],
                        },
                    },
                ],
            },
            span: crate::error::SourceSpan::synthetic(),
        },
    ];
    let mut names = Names::default();

    for effect in &effects {
        walk_test_effect(&mut names, effect);
    }

    assert_eq!(
        names.0,
        [
            "respond_expect",
            "respond",
            "throw",
            "event",
            "sequence_expect",
            "step_expect",
            "step_respond",
            "step_throw",
            "step_event",
        ]
    );

    struct PrefixIdentifiers;

    impl AstVisitorMut for PrefixIdentifiers {
        fn visit_expr(&mut self, expr: &mut Expr) {
            if let Expr::Identifier(name) = expr {
                name.insert_str(0, "renamed_");
            } else {
                walk_expr_mut(self, expr);
            }
        }
    }

    let mut prefixer = PrefixIdentifiers;
    for effect in &mut effects {
        walk_test_effect_mut(&mut prefixer, effect);
    }
    let mut renamed_names = Names::default();
    for effect in &effects {
        walk_test_effect(&mut renamed_names, effect);
    }
    assert!(
        renamed_names
            .0
            .iter()
            .all(|name| name.starts_with("renamed_")),
        "mutable walker missed an embedded expression: {:?}",
        renamed_names.0
    );
}

#[test]
fn compiler_test_effect_walkers_keep_probe_and_expectation_order() {
    #[derive(Default)]
    struct Names(Vec<String>);

    impl AstVisitor for Names {
        fn visit_expr(&mut self, expr: &Expr) {
            if let Expr::Identifier(name) = expr {
                self.0.push(name.clone());
            }
            walk_expr(self, expr);
        }
    }

    let mut stmt = Stmt::CompilerTestEffectRegister {
        target: "std.http.request".to_string(),
        target_probe: Expr::Identifier("target_probe".to_string()),
        expect: Some(Expr::Identifier("common_expect".to_string())),
        step_expect: Some(Expr::Identifier("step_expect".to_string())),
        outcome: TestEffectStepOutcome::Respond {
            value: Expr::Identifier("outcome".to_string()),
        },
    };
    let mut names = Names::default();
    names.visit_stmt(&stmt);
    assert_eq!(
        names.0,
        ["target_probe", "common_expect", "step_expect", "outcome"]
    );

    let expression_names = compiler_test_effect_expressions(&stmt)
        .unwrap()
        .into_iter()
        .map(|expr| match expr {
            Expr::Identifier(name) => name.as_str(),
            other => panic!("expected identifier, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        expression_names,
        ["target_probe", "common_expect", "step_expect", "outcome"]
    );

    struct PrefixIdentifiers;

    impl AstVisitorMut for PrefixIdentifiers {
        fn visit_expr(&mut self, expr: &mut Expr) {
            if let Expr::Identifier(name) = expr {
                name.insert_str(0, "renamed_");
            } else {
                walk_expr_mut(self, expr);
            }
        }
    }

    let mut prefixer = PrefixIdentifiers;
    prefixer.visit_stmt(&mut stmt);
    let mut renamed_names = Names::default();
    renamed_names.visit_stmt(&stmt);
    assert_eq!(
        renamed_names.0,
        [
            "renamed_target_probe",
            "renamed_common_expect",
            "renamed_step_expect",
            "renamed_outcome",
        ]
    );
}
