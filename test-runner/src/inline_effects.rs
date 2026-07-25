use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

use serde::Serialize;
use serde_json::{Map, Number, Value};
use skiff_artifact_model::{PackageArtifact, PackageLocalAbiSymbol, PackageTypeRef};
use skiff_compiler_input::{package_config::PackageManifest, PackageDependencyAccess};
use skiff_compiler_source::{
    validate_and_plan_test_effects, ResolvedTestEffectTarget, TestEffectExpressionExpectation,
    TestEffectPlanValidator, TypedTestEffectOutcome, TypedTestEffectPlan,
};
use skiff_syntax::ast::{Expr, Literal, ObjectLiteralKey};

use crate::{
    canonical_fixture::CanonicalFixtureError, canonical_package::CanonicalPackageProject,
    test_discovery::PackageTestCase,
};

const LEGACY_MANIFEST: &str = "skiff.test-doubles.json";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeTestEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expect_request: Option<Value>,
    pub(crate) response: Value,
}

pub(crate) type RuntimeTestEffectPlan = HashMap<String, Vec<RuntimeTestEffect>>;

/// F267's compiler-owned boundary. The runner never constructs runtime doubles
/// from untyped source declarations: every declaration first crosses
/// `TypedTestEffectPlan`.
pub(crate) struct AssembledTestEffectPlanValidator<'a> {
    artifacts: Vec<&'a PackageArtifact>,
    aliases: BTreeMap<String, (&'a PackageArtifact, bool)>,
}

impl<'a> AssembledTestEffectPlanValidator<'a> {
    pub(crate) fn new(project: &'a CanonicalPackageProject, manifest: &'a PackageManifest) -> Self {
        let artifacts = project.artifacts().collect::<Vec<_>>();
        let aliases = manifest
            .dependencies
            .iter()
            .filter_map(|dependency| {
                let artifact = artifacts.iter().copied().find(|artifact| {
                    artifact.package_id == dependency.id
                        && artifact.package_version == dependency.version
                })?;
                Some((
                    dependency
                        .alias
                        .clone()
                        .unwrap_or_else(|| dependency.id.clone()),
                    (
                        artifact,
                        dependency.access == PackageDependencyAccess::TopLevel,
                    ),
                ))
            })
            .collect();
        Self { artifacts, aliases }
    }

    fn symbol(
        &self,
        source_target: &str,
    ) -> Option<(&PackageArtifact, String, &PackageLocalAbiSymbol)> {
        if let Some((alias, path)) = source_target.split_once('/') {
            let (artifact, top_level) = self.aliases.get(alias)?;
            let symbols = if *top_level {
                &artifact.package_local_abi.implementation_symbols
            } else {
                &artifact.package_local_abi.public_symbols
            };
            return symbols
                .get(path)
                .map(|symbol| (*artifact, path.to_string(), symbol));
        }

        self.artifacts.iter().find_map(|artifact| {
            artifact
                .package_local_abi
                .public_symbols
                .get(source_target)
                .map(|symbol| (*artifact, source_target.to_string(), symbol))
        })
    }
}

impl TestEffectPlanValidator for AssembledTestEffectPlanValidator<'_> {
    fn resolve_target(&mut self, source_target: &str) -> Result<ResolvedTestEffectTarget, String> {
        let (_artifact, resolved_path, symbol) = self.symbol(source_target).ok_or_else(|| {
            format!("unresolved exact callable target `{source_target}` in test-service graph")
        })?;
        let PackageLocalAbiSymbol::Callable {
            callable_id: _,
            signature,
        } = symbol
        else {
            return Err(format!("exact target `{source_target}` is not callable"));
        };
        Ok(ResolvedTestEffectTarget {
            // Runtime dispatch uses the exact linked public/implementation path.
            // Resolution above additionally proves the owning immutable artifact
            // and callable id, so this string is never an unresolved source token.
            identity: resolved_path,
            signature: signature.clone(),
        })
    }

    fn validate_expression(
        &mut self,
        expression: &Expr,
        expectation: &TestEffectExpressionExpectation,
    ) -> Result<(), String> {
        validate_constant_expression(expression)?;
        validate_literal_shape(expression, expectation)
    }
}

pub(crate) fn reject_legacy_manifest(package_root: &Path) -> Result<(), CanonicalFixtureError> {
    let mut candidates = vec![package_root.join(LEGACY_MANIFEST)];
    if let Some(parent) = package_root.parent() {
        candidates.push(parent.join(LEGACY_MANIFEST));
    }
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) {
        return Err(CanonicalFixtureError::InvalidInput(format!(
            "{} is unsupported; move effect doubles into `test ... effects {{ ... }}` and config into the test service profile",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn compile_case_plans(
    project: &CanonicalPackageProject,
    manifest: &PackageManifest,
    cases: &[PackageTestCase],
) -> Result<HashMap<String, RuntimeTestEffectPlan>, CanonicalFixtureError> {
    let mut plans = HashMap::new();
    for case in cases {
        let mut validator = AssembledTestEffectPlanValidator::new(project, manifest);
        let typed = validate_and_plan_test_effects(
            case.effect_plan.case_identity.clone(),
            &case.effect_plan.effects,
            &mut validator,
        )
        .map_err(|message| {
            CanonicalFixtureError::InvalidInput(format!(
                "{} inline effects are invalid: {message}",
                case.case_identity
            ))
        })?;
        let runtime = runtime_plan(&typed)?;
        plans.insert(case.case_identity.clone(), runtime);
    }
    Ok(plans)
}

fn runtime_plan(
    plan: &TypedTestEffectPlan,
) -> Result<RuntimeTestEffectPlan, CanonicalFixtureError> {
    let mut runtime = HashMap::new();
    for effect in &plan.effects {
        let expect_request = effect
            .expect
            .as_ref()
            .map(|expression| materialize_expression(&expression.expression))
            .transpose()?;
        let mut outcomes = Vec::with_capacity(effect.outcomes.len());
        for outcome in &effect.outcomes {
            let response = match outcome {
                TypedTestEffectOutcome::Respond(expression) => {
                    materialize_expression(&expression.expression)?
                }
                TypedTestEffectOutcome::Stream(events) => serde_json::json!({
                    "__skiffTestEffectOutcome": "stream",
                    "events": events
                        .iter()
                        .map(|event| materialize_expression(&event.expression))
                        .collect::<Result<Vec<_>, _>>()?,
                }),
                TypedTestEffectOutcome::Throw(expression) => serde_json::json!({
                    "__skiffTestEffectOutcome": "throw",
                    "payload": materialize_expression(&expression.expression)?,
                    "declaredTypes": effect.target.signature.throw_types,
                }),
            };
            outcomes.push(RuntimeTestEffect {
                expect_request: expect_request.clone(),
                response,
            });
        }
        runtime.insert(effect.target.identity.clone(), outcomes);
    }
    Ok(runtime)
}

fn validate_constant_expression(expression: &Expr) -> Result<(), String> {
    match expression {
        Expr::Literal(_) => Ok(()),
        Expr::Record { fields, .. } => fields
            .iter()
            .try_for_each(|(_, value)| validate_constant_expression(value)),
        Expr::ObjectLiteral { entries } => entries
            .iter()
            .try_for_each(|entry| validate_constant_expression(&entry.value)),
        Expr::Call { callee, args } if constant_constructor_name(callee).is_some() => args
            .iter()
            .try_for_each(validate_constant_expression),
        _ => Err(
            "inline effect values must be deterministic literals, records, objects, or supported pure constructors"
                .to_string(),
        ),
    }
}

fn validate_literal_shape(
    expression: &Expr,
    expectation: &TestEffectExpressionExpectation,
) -> Result<(), String> {
    let expected = match expectation {
        TestEffectExpressionExpectation::RequestSubset(ty)
        | TestEffectExpressionExpectation::Exact(ty) => std::slice::from_ref(ty),
        TestEffectExpressionExpectation::OneOf(types) => types.as_slice(),
    };
    if expected
        .iter()
        .any(|ty| expression_may_match_type(expression, ty))
    {
        Ok(())
    } else {
        Err(format!(
            "constant expression is incompatible with expected type(s): {expected:?}"
        ))
    }
}

fn expression_may_match_type(expression: &Expr, ty: &PackageTypeRef) -> bool {
    match ty {
        PackageTypeRef::Nullable { inner } => {
            matches!(expression, Expr::Literal(Literal::Null))
                || expression_may_match_type(expression, inner)
        }
        PackageTypeRef::Container { name, .. } => match name.as_str() {
            "string" | "String" => matches!(expression, Expr::Literal(Literal::String(_))),
            "bool" | "Bool" | "boolean" => {
                matches!(expression, Expr::Literal(Literal::Bool(_)))
            }
            "number" | "Number" | "integer" | "Integer" | "float" | "Float" => {
                matches!(expression, Expr::Literal(Literal::Number(_)))
            }
            _ => true,
        },
        PackageTypeRef::Local { .. }
        | PackageTypeRef::PackageSchema { .. }
        | PackageTypeRef::AnyInterface { .. } => {
            matches!(
                expression,
                Expr::Record { .. } | Expr::ObjectLiteral { .. } | Expr::Call { .. }
            )
        }
    }
}

fn materialize_expression(expression: &Expr) -> Result<Value, CanonicalFixtureError> {
    match expression {
        Expr::Literal(Literal::Null) => Ok(Value::Null),
        Expr::Literal(Literal::Bool(value)) => Ok(Value::Bool(*value)),
        Expr::Literal(Literal::String(value)) => Ok(Value::String(value.clone())),
        Expr::Literal(Literal::Number(value)) => {
            Number::from_f64(*value).map(Value::Number).ok_or_else(|| {
                CanonicalFixtureError::InvalidInput(
                    "inline effect number must be finite".to_string(),
                )
            })
        }
        Expr::Record { fields, .. } => materialize_fields(fields),
        Expr::ObjectLiteral { entries } => {
            let fields = entries
                .iter()
                .map(|entry| {
                    let ObjectLiteralKey::Name(name) = &entry.key;
                    (name.clone(), entry.value.clone())
                })
                .collect::<Vec<_>>();
            materialize_fields(&fields)
        }
        Expr::Call { callee, args } => materialize_constructor(callee, args),
        _ => Err(CanonicalFixtureError::InvalidInput(
            "inline effect expression passed typing but has no deterministic materializer"
                .to_string(),
        )),
    }
}

fn materialize_fields(fields: &[(String, Expr)]) -> Result<Value, CanonicalFixtureError> {
    fields
        .iter()
        .map(|(name, value)| Ok((name.clone(), materialize_expression(value)?)))
        .collect::<Result<Map<_, _>, _>>()
        .map(Value::Object)
}

fn materialize_constructor(callee: &Expr, args: &[Expr]) -> Result<Value, CanonicalFixtureError> {
    let name = constant_constructor_name(callee).ok_or_else(|| {
        CanonicalFixtureError::InvalidInput(
            "inline effect call is not a supported pure constructor".to_string(),
        )
    })?;
    match (name.as_str(), args) {
        ("bytes.fromUtf8", [arg]) | ("std.bytes.fromUtf8", [arg]) => {
            let Value::String(text) = materialize_expression(arg)? else {
                return Err(CanonicalFixtureError::InvalidInput(
                    "bytes.fromUtf8 inline effect argument must be a string".to_string(),
                ));
            };
            Ok(serde_json::json!({
                "__skiffBytesBase64": base64_encode(text.as_bytes())
            }))
        }
        (name, []) if name.starts_with("Array.empty") => Ok(Value::Array(Vec::new())),
        _ => Err(CanonicalFixtureError::InvalidInput(format!(
            "unsupported inline effect pure constructor `{name}`"
        ))),
    }
}

fn constant_constructor_name(expression: &Expr) -> Option<String> {
    match expression {
        Expr::Identifier(name) => Some(name.clone()),
        Expr::Field { object, field } => {
            Some(format!("{}.{}", constant_constructor_name(object)?, field))
        }
        Expr::Generic { callee, .. } => constant_constructor_name(callee),
        _ => None,
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn utf8_bytes_constructor_materializes_canonical_wire_value() {
        let expression = Expr::Call {
            callee: Box::new(Expr::Field {
                object: Box::new(Expr::Identifier("bytes".to_string())),
                field: "fromUtf8".to_string(),
            }),
            args: vec![Expr::Literal(Literal::String("hello".to_string()))],
        };
        assert_eq!(
            materialize_expression(&expression).unwrap(),
            serde_json::json!({"__skiffBytesBase64": "aGVsbG8="})
        );
    }

    #[test]
    fn legacy_manifest_is_rejected_without_parsing_it() {
        let root =
            std::env::temp_dir().join(format!("skiff-inline-effects-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(LEGACY_MANIFEST), "{}").unwrap();
        let error = reject_legacy_manifest(&root).unwrap_err();
        assert!(error.to_string().contains("is unsupported"));
        fs::remove_dir_all(root).unwrap();
    }
}
