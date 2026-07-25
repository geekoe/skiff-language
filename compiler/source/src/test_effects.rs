use skiff_artifact_model::{PackageCallableSignature, PackageTypeRef};

use crate::shared::ast::{Expr, TestEffectDeclaration, TestEffectOutcome};

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTestEffectTarget {
    pub identity: String,
    pub signature: PackageCallableSignature,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TestEffectExpressionExpectation {
    RequestSubset(PackageTypeRef),
    Exact(PackageTypeRef),
    OneOf(Vec<PackageTypeRef>),
}

pub trait TestEffectPlanValidator {
    fn resolve_target(&mut self, source_target: &str) -> Result<ResolvedTestEffectTarget, String>;

    fn validate_expression(
        &mut self,
        expression: &Expr,
        expectation: &TestEffectExpressionExpectation,
    ) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedTestEffectPlan {
    pub case_identity: String,
    pub effects: Vec<TypedTestEffect>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedTestEffect {
    pub target: ResolvedTestEffectTarget,
    pub expect: Option<TypedTestEffectExpression>,
    pub outcomes: Vec<TypedTestEffectOutcome>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedTestEffectExpression {
    pub expression: Expr,
    pub expectation: TestEffectExpressionExpectation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypedTestEffectOutcome {
    Respond(TypedTestEffectExpression),
    Throw(TypedTestEffectExpression),
    Stream(Vec<TypedTestEffectExpression>),
}

pub fn validate_and_plan_test_effects(
    case_identity: impl Into<String>,
    declarations: &[TestEffectDeclaration],
    validator: &mut impl TestEffectPlanValidator,
) -> Result<TypedTestEffectPlan, String> {
    use std::collections::BTreeSet;

    let case_identity = case_identity.into();
    if case_identity.is_empty() {
        return Err("test effect case identity cannot be empty".to_string());
    }
    let mut targets = BTreeSet::new();
    let mut effects = Vec::with_capacity(declarations.len());
    for declaration in declarations {
        if !targets.insert(declaration.target.as_str()) {
            return Err(format!(
                "duplicate test effect target `{}`",
                declaration.target
            ));
        }
        effects.push(validate_effect(declaration, validator)?);
    }
    Ok(TypedTestEffectPlan {
        case_identity,
        effects,
    })
}

fn validate_effect(
    declaration: &TestEffectDeclaration,
    validator: &mut impl TestEffectPlanValidator,
) -> Result<TypedTestEffect, String> {
    let target = validator
        .resolve_target(&declaration.target)
        .map_err(|error| format!("test effect `{}`: {error}", declaration.target))?;
    if target.identity.is_empty() {
        return Err(format!(
            "test effect `{}` resolved to an empty target identity",
            declaration.target
        ));
    }

    let expect = declaration
        .expect
        .as_ref()
        .map(|expression| {
            let [parameter] = target.signature.parameters.as_slice() else {
                return Err(format!(
                    "test effect `{}` can use `expect` only when the target has exactly one request parameter",
                    declaration.target
                ));
            };
            checked_expression(
                expression,
                TestEffectExpressionExpectation::RequestSubset(parameter.ty.clone()),
                validator,
                &declaration.target,
                "expect",
            )
        })
        .transpose()?;

    let stream_item = stream_item_type(&target.signature.return_type);
    let mut outcomes = Vec::with_capacity(declaration.outcomes.len());
    for outcome in &declaration.outcomes {
        outcomes.push(match outcome {
            TestEffectOutcome::Respond { value } => {
                if stream_item.is_some() {
                    return Err(format!(
                        "test effect `{}` returns a stream; use `stream`",
                        declaration.target
                    ));
                }
                TypedTestEffectOutcome::Respond(checked_expression(
                    value,
                    TestEffectExpressionExpectation::Exact(target.signature.return_type.clone()),
                    validator,
                    &declaration.target,
                    "respond",
                )?)
            }
            TestEffectOutcome::Throw { value } => {
                if target.signature.throw_types.is_empty() {
                    return Err(format!(
                        "test effect `{}` does not declare a typed error",
                        declaration.target
                    ));
                }
                TypedTestEffectOutcome::Throw(checked_expression(
                    value,
                    TestEffectExpressionExpectation::OneOf(target.signature.throw_types.clone()),
                    validator,
                    &declaration.target,
                    "throw",
                )?)
            }
            TestEffectOutcome::Stream { events } => {
                let item = stream_item.clone().ok_or_else(|| {
                    format!(
                        "test effect `{}` is not a direct Stream<T> target",
                        declaration.target
                    )
                })?;
                let events = events
                    .iter()
                    .map(|event| {
                        checked_expression(
                            event,
                            TestEffectExpressionExpectation::Exact(item.clone()),
                            validator,
                            &declaration.target,
                            "stream event",
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                TypedTestEffectOutcome::Stream(events)
            }
        });
    }

    Ok(TypedTestEffect {
        target,
        expect,
        outcomes,
    })
}

fn checked_expression(
    expression: &Expr,
    expectation: TestEffectExpressionExpectation,
    validator: &mut impl TestEffectPlanValidator,
    target: &str,
    role: &str,
) -> Result<TypedTestEffectExpression, String> {
    validator
        .validate_expression(expression, &expectation)
        .map_err(|error| format!("test effect `{target}` {role}: {error}"))?;
    Ok(TypedTestEffectExpression {
        expression: expression.clone(),
        expectation,
    })
}

fn stream_item_type(ty: &PackageTypeRef) -> Option<PackageTypeRef> {
    match ty {
        PackageTypeRef::Container { name, arguments }
            if name == "Stream" && arguments.len() == 1 =>
        {
            arguments.first().cloned()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{PackageCallableParameter, TypeRefIr};

    use super::*;
    use crate::shared::ast::{Literal, TestEffectDeclaration};
    use crate::shared::error::SourceSpan;

    struct MatrixValidator {
        signatures: BTreeMap<String, PackageCallableSignature>,
        seen: Vec<TestEffectExpressionExpectation>,
        reject: Option<&'static str>,
    }

    impl TestEffectPlanValidator for MatrixValidator {
        fn resolve_target(
            &mut self,
            source_target: &str,
        ) -> Result<ResolvedTestEffectTarget, String> {
            self.signatures
                .get(source_target)
                .cloned()
                .map(|signature| ResolvedTestEffectTarget {
                    identity: format!("callable:{source_target}"),
                    signature,
                })
                .ok_or_else(|| "unresolved exact target".to_string())
        }

        fn validate_expression(
            &mut self,
            _expression: &Expr,
            expectation: &TestEffectExpressionExpectation,
        ) -> Result<(), String> {
            self.seen.push(expectation.clone());
            self.reject
                .map_or(Ok(()), |message| Err(message.to_string()))
        }
    }

    #[test]
    fn plans_unary_sequence_throw_and_stream_with_exact_expectations() {
        let request = builtin("Request");
        let response = builtin("Response");
        let failure = builtin("Failure");
        let item = builtin("string");
        let mut validator = MatrixValidator {
            signatures: BTreeMap::from([
                (
                    "std.http.request".to_string(),
                    signature(request.clone(), response.clone(), vec![failure.clone()]),
                ),
                (
                    "std.events".to_string(),
                    signature(
                        request.clone(),
                        PackageTypeRef::Container {
                            name: "Stream".to_string(),
                            arguments: vec![item.clone()],
                        },
                        Vec::new(),
                    ),
                ),
                (
                    "std.http.throwingRequest".to_string(),
                    signature(request.clone(), response.clone(), vec![failure.clone()]),
                ),
            ]),
            seen: Vec::new(),
            reject: None,
        };
        let declarations = vec![
            declaration(
                "std.http.throwingRequest",
                Some(literal("request")),
                vec![
                    TestEffectOutcome::Respond {
                        value: literal("one"),
                    },
                    TestEffectOutcome::Respond {
                        value: literal("two"),
                    },
                ],
            ),
            declaration(
                "std.http.request",
                None,
                vec![TestEffectOutcome::Throw {
                    value: literal("failure"),
                }],
            ),
            declaration(
                "std.events",
                None,
                vec![TestEffectOutcome::Stream {
                    events: vec![literal("a"), literal("b")],
                }],
            ),
        ];

        let plan = validate_and_plan_test_effects("module::test[0]", &declarations, &mut validator)
            .unwrap();
        assert_eq!(plan.case_identity, "module::test[0]");
        assert_eq!(plan.effects.len(), 3);
        assert!(matches!(
            validator.seen[0],
            TestEffectExpressionExpectation::RequestSubset(_)
        ));
        assert!(validator.seen.iter().any(|expectation| matches!(
            expectation,
            TestEffectExpressionExpectation::OneOf(types) if types == &vec![failure.clone()]
        )));
        assert_eq!(
            validator
                .seen
                .iter()
                .filter(|expectation| {
                    matches!(
                        expectation,
                        TestEffectExpressionExpectation::Exact(ty) if ty == &item
                    )
                })
                .count(),
            2
        );
    }

    #[test]
    fn rejects_unresolved_malformed_and_incompatible_plans() {
        let request = builtin("Request");
        let response = builtin("Response");
        let mut validator = MatrixValidator {
            signatures: BTreeMap::from([(
                "unary".to_string(),
                signature(request, response, Vec::new()),
            )]),
            seen: Vec::new(),
            reject: None,
        };
        let unresolved = declaration(
            "missing",
            None,
            vec![TestEffectOutcome::Respond {
                value: literal("x"),
            }],
        );
        assert!(
            validate_and_plan_test_effects("case", &[unresolved], &mut validator)
                .unwrap_err()
                .contains("unresolved exact target")
        );

        let invalid_throw = declaration(
            "unary",
            None,
            vec![TestEffectOutcome::Throw {
                value: literal("x"),
            }],
        );
        assert!(
            validate_and_plan_test_effects("case", &[invalid_throw], &mut validator)
                .unwrap_err()
                .contains("does not declare a typed error")
        );

        validator.reject = Some("incompatible expression");
        let invalid_response = declaration(
            "unary",
            None,
            vec![TestEffectOutcome::Respond {
                value: literal("x"),
            }],
        );
        assert!(
            validate_and_plan_test_effects("case", &[invalid_response], &mut validator)
                .unwrap_err()
                .contains("incompatible expression")
        );
    }

    fn declaration(
        target: &str,
        expect: Option<Expr>,
        outcomes: Vec<TestEffectOutcome>,
    ) -> TestEffectDeclaration {
        TestEffectDeclaration {
            target: target.to_string(),
            expect,
            outcomes,
            span: SourceSpan::synthetic(),
        }
    }

    fn literal(value: &str) -> Expr {
        Expr::Literal(Literal::String(value.to_string()))
    }

    fn signature(
        request: PackageTypeRef,
        response: PackageTypeRef,
        throw_types: Vec<PackageTypeRef>,
    ) -> PackageCallableSignature {
        PackageCallableSignature {
            parameters: vec![PackageCallableParameter {
                name: "request".to_string(),
                ty: request,
            }],
            return_type: response,
            throw_types,
            may_suspend: false,
        }
    }

    fn builtin(name: &str) -> PackageTypeRef {
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin {
                name: name.to_string(),
                args: Vec::new(),
            },
        }
    }
}
