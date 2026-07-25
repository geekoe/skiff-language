//! Test-only source overlay compilation.
//!
//! Production source is compiled first and retained verbatim. Test declarations
//! are converted to ordinary private functions in a second package build; the
//! canonical production package reference is never overwritten or retagged.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use skiff_artifact_identity::package_artifact_ref;
use skiff_artifact_model::{PackageArtifactRef, PackageCallableId, PackageLocalAbiSymbol};
use skiff_compiler::{
    CompilerPlatformSources, PackageCompileError, PackageSourceInput, PublishedPackageArtifact,
};
use skiff_compiler_input::source_tree::SourceTreeFile;
use skiff_compiler_input::{
    package_config::{is_standard_package_id, PackageManifest},
    package_sources::{read_official_package_sources, read_package_sources},
    read_publication_resources, InputAssemblyError, ManifestOwner, PublicationApiEntry,
};
use skiff_compiler_source::{
    source_graph::{CompilerSourceFile, PublicationSourceGraph},
    SourceCompileError,
};
use skiff_deployment::storage::{CanonicalArtifactStore, EcosystemStorageError};
use skiff_syntax::ast::{
    Block, BlockSourceSpans, DependencySourceAddress, ExecutableSourceSpans, Expr, ExprSourceSpans,
    FunctionDecl, SourceFile, Stmt, StmtSourceSpans, TestEffectOutcome,
    TestEffectOutcomeSourceSpans, TestEffectStepOutcome, TestEffectStepOutcomeSourceSpans, TypeRef,
};
use thiserror::Error;

use crate::{
    canonical_fixture::PackageTestCase,
    canonical_package::{
        compile_package_artifact_with_store, package_aliases, read_compiled_dependency_closure,
        read_optional_platform_std, read_root_package_manifest, CanonicalPackageProject,
        CanonicalPackageProjectError,
    },
};

#[derive(Debug, Clone)]
pub struct PackageTestOverlayBinding {
    pub case: PackageTestCase,
    pub public_path: String,
    pub callable_id: PackageCallableId,
}

#[derive(Debug, Clone)]
pub struct PublishedPackageTestOverlay {
    pub production: PackageArtifactRef,
    pub overlay: PublishedPackageArtifact,
    pub dependency_packages: Vec<skiff_artifact_model::PackageArtifact>,
    pub bindings: Vec<PackageTestOverlayBinding>,
}

#[derive(Debug, Error)]
pub enum PackageTestOverlayError {
    #[error(transparent)]
    Project(#[from] CanonicalPackageProjectError),
    #[error(transparent)]
    Input(#[from] InputAssemblyError),
    #[error(transparent)]
    Source(#[from] SourceCompileError),
    #[error(transparent)]
    Compile(#[from] PackageCompileError),
    #[error(transparent)]
    Storage(#[from] EcosystemStorageError),
    #[error("invalid package-test overlay: {0}")]
    Invalid(String),
}

pub fn compile_package_test_overlay(
    platform_sources: &CompilerPlatformSources,
    package_root: &Path,
    artifact_root: &Path,
    project: &CanonicalPackageProject,
    cases: &[PackageTestCase],
) -> Result<PublishedPackageTestOverlay, PackageTestOverlayError> {
    if cases.is_empty() {
        return Err(PackageTestOverlayError::Invalid(
            "at least one package test case is required".to_string(),
        ));
    }
    let production = package_artifact_ref(&project.package.artifact)
        .map_err(|error| PackageTestOverlayError::Invalid(error.to_string()))?;
    let (source, manifest) =
        build_overlay_source(platform_sources, package_root, cases, production.clone())?;
    let store = CanonicalArtifactStore::open(artifact_root)?;
    let overlay = compile_overlay_artifact(platform_sources, project, &manifest, &source, &store)?;
    let dependency_packages = read_compiled_dependency_closure(&store, &overlay.artifact)?;
    let bindings = overlay_bindings(cases, &overlay)?;
    if package_artifact_ref(&project.package.artifact)
        .map_err(|error| PackageTestOverlayError::Invalid(error.to_string()))?
        != production
    {
        return Err(PackageTestOverlayError::Invalid(
            "test overlay rewrote production PackageArtifact identity".to_string(),
        ));
    }
    Ok(PublishedPackageTestOverlay {
        production,
        overlay,
        dependency_packages,
        bindings,
    })
}

fn build_overlay_source(
    platform_sources: &CompilerPlatformSources,
    package_root: &Path,
    cases: &[PackageTestCase],
    production: PackageArtifactRef,
) -> Result<(PackageSourceInput, PackageManifest), PackageTestOverlayError> {
    let manifest = read_root_package_manifest(platform_sources, package_root)?;
    let raw_sources = match manifest.provenance.owner {
        ManifestOwner::CompilerStandardPackage => {
            read_official_package_sources(platform_sources, &manifest)?
        }
        ManifestOwner::UserOrBuiltinPackage => read_package_sources(&manifest, package_root)?,
    };
    let mut source_tree = raw_sources.source_tree();
    let raw_graph = raw_sources.into_source_graph();
    let parsed_graph = PublicationSourceGraph::parse_raw_publication_sources(&raw_graph)?;
    let compiler_sources = parsed_graph.files().to_vec();
    let mut private_test_sources = Vec::new();
    let mut overlay_manifest = manifest.publication.clone();
    let mut grouped = BTreeMap::<PathBuf, Vec<&PackageTestCase>>::new();
    for case in cases {
        grouped
            .entry(case.relative_path.clone())
            .or_default()
            .push(case);
    }

    for (relative_path, selected) in grouped {
        let module_path = overlay_module_path(
            overlay_manifest.id.as_str(),
            &relative_path,
            &source_tree.sources,
        )?;
        let transformed = package_test_ast_for_cases(
            &selected[0].source_ast,
            selected
                .iter()
                .map(|case| (case.test_index, case.function_name.as_str())),
        );
        private_test_sources.push(CompilerSourceFile::from_parsed_ast(
            relative_path.clone(),
            module_path.clone(),
            true,
            false,
            selected[0].source_text.clone(),
            transformed,
        ));
        source_tree.sources.push(SourceTreeFile {
            module_path: module_path.clone(),
            file_path: relative_path,
            is_test_file: false,
            byte_len: selected[0].source_text.len() as u64,
        });
        let api_module_path = if is_standard_package_id(overlay_manifest.id.as_str()) {
            module_path
                .strip_prefix("std.")
                .unwrap_or(&module_path)
                .to_string()
        } else {
            module_path.clone()
        };
        overlay_manifest
            .api
            .entries
            .extend(selected.iter().map(|case| {
                PublicationApiEntry::for_source(
                    manifest_public_path(case),
                    api_module_path.clone(),
                    case.function_name.clone(),
                )
            }));
    }
    source_tree.sources.sort_by(|left, right| {
        left.module_path
            .cmp(&right.module_path)
            .then_with(|| left.file_path.cmp(&right.file_path))
    });
    let resources = read_publication_resources(package_root, &overlay_manifest.resources)?;
    let source = PackageSourceInput::new(
        overlay_manifest,
        source_tree,
        PublicationSourceGraph::from_compiler_sources(compiler_sources),
        resources,
    )
    .with_test_overlay(production, private_test_sources);
    Ok((source, manifest))
}

fn compile_overlay_artifact(
    platform_sources: &CompilerPlatformSources,
    project: &CanonicalPackageProject,
    manifest: &PackageManifest,
    source: &PackageSourceInput,
    store: &CanonicalArtifactStore,
) -> Result<PublishedPackageArtifact, PackageTestOverlayError> {
    let aliases = package_aliases(manifest, &project.dependency_packages);
    let dependencies = manifest
        .dependencies
        .iter()
        .map(|dependency| {
            project
                .artifact(&dependency.id, &dependency.version)
                .cloned()
                .ok_or_else(|| {
                    PackageTestOverlayError::Invalid(format!(
                        "canonical dependency {}@{} is absent from compiled project",
                        dependency.id, dependency.version
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut available = project.artifacts().cloned().collect::<Vec<_>>();
    read_optional_platform_std(store, &mut available)?;
    Ok(compile_package_artifact_with_store(
        platform_sources,
        source,
        &aliases,
        &dependencies,
        &available,
        &project.contract_dependencies,
        Some(store),
        project.test_service_profile.is_some(),
    )?)
}

fn overlay_bindings(
    cases: &[PackageTestCase],
    overlay: &PublishedPackageArtifact,
) -> Result<Vec<PackageTestOverlayBinding>, PackageTestOverlayError> {
    cases
        .iter()
        .map(|case| {
            let public_path = artifact_public_path(&overlay.artifact.package_id, case);
            let symbol = overlay
                .artifact
                .package_local_abi
                .public_symbols
                .get(&public_path)
                .ok_or_else(|| {
                    PackageTestOverlayError::Invalid(format!(
                        "overlay public path {public_path} was not emitted"
                    ))
                })?;
            let PackageLocalAbiSymbol::Callable { callable_id, .. } = symbol else {
                return Err(PackageTestOverlayError::Invalid(format!(
                    "overlay public path {public_path} is not callable"
                )));
            };
            Ok(PackageTestOverlayBinding {
                case: case.clone(),
                public_path,
                callable_id: callable_id.clone(),
            })
        })
        .collect()
}

fn manifest_public_path(case: &PackageTestCase) -> String {
    format!(
        "testCases.case{}",
        case.function_name.trim_start_matches("skiffTestCase")
    )
}

fn artifact_public_path(package_id: &str, case: &PackageTestCase) -> String {
    let public_path = manifest_public_path(case);
    if is_standard_package_id(package_id) {
        format!("std.{public_path}")
    } else {
        public_path
    }
}

fn overlay_module_path(
    package_id: &str,
    test_path: &Path,
    production_sources: &[SourceTreeFile],
) -> Result<String, PackageTestOverlayError> {
    let name = test_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            PackageTestOverlayError::Invalid(format!(
                "test path {} is not valid UTF-8",
                test_path.display()
            ))
        })?;
    let production_name = name.strip_suffix(".test.skiff").ok_or_else(|| {
        PackageTestOverlayError::Invalid(format!(
            "test path {} must end with .test.skiff",
            test_path.display()
        ))
    })?;
    let production_path = test_path.with_file_name(format!("{production_name}.skiff"));
    let base = production_sources
        .iter()
        .find(|source| source.file_path == production_path)
        .map(|source| source.module_path.clone())
        .unwrap_or_else(|| {
            let mut relative = test_path.to_path_buf();
            relative.set_file_name(production_name);
            let path = relative
                .components()
                .filter_map(|part| part.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join(".");
            if package_id == "skiff.run/std" {
                format!("std.{path}")
            } else {
                path
            }
        });
    Ok(format!("{base}.__test"))
}

fn package_test_ast_for_cases<'a>(
    ast: &SourceFile,
    tests: impl IntoIterator<Item = (usize, &'a str)>,
) -> SourceFile {
    let functions = tests
        .into_iter()
        .map(|(test_index, function_name)| {
            let test = ast
                .tests
                .get(test_index)
                .expect("discovered package test case belongs to this AST");
            let source_spans = ast.source_spans.tests.get(test_index).cloned();
            let setup_name = format!("{function_name}Setup");
            let mut setup_statements = Vec::new();
            let mut setup_statement_spans = Vec::new();
            for (effect_index, effect) in test.effects.iter().enumerate() {
                let effect_spans = source_spans
                    .as_ref()
                    .and_then(|spans| spans.effects.get(effect_index));
                let steps = flattened_test_effect_steps(&effect.outcome);
                let step_spans = effect_spans
                    .map(|spans| flattened_test_effect_step_spans(&spans.outcome))
                    .unwrap_or_default();
                for (step_index, (step_expect, outcome)) in steps.into_iter().enumerate() {
                    let target_probe = Expr::Call {
                        callee: Box::new(match effect.target.split_once('/') {
                            Some((dependency_ref, public_path)) => {
                                Expr::DependencySourceAddress(DependencySourceAddress {
                                    dependency_ref: dependency_ref.to_string(),
                                    public_path: public_path.to_string(),
                                })
                            }
                            None => Expr::Identifier(effect.target.clone()),
                        }),
                        args: Vec::new(),
                    };
                    setup_statements.push(Stmt::CompilerTestEffectRegister {
                        target: effect.target.clone(),
                        target_probe,
                        declaration_start: step_index == 0,
                        // The target-level expression is evaluated once. The
                        // runtime registry snapshots it on the first
                        // registration and applies that snapshot to every
                        // later step in the same sequence.
                        expect: (step_index == 0).then(|| effect.expect.clone()).flatten(),
                        step_expect,
                        outcome,
                    });
                    if let Some(effect_spans) = effect_spans {
                        let mut expressions = Vec::new();
                        expressions.push(ExprSourceSpans {
                            span: effect.span,
                            children: vec![ExprSourceSpans {
                                span: effect.span,
                                children: Vec::new(),
                                blocks: Vec::new(),
                                record_fields: Vec::new(),
                            }],
                            blocks: Vec::new(),
                            record_fields: Vec::new(),
                        });
                        if step_index == 0 {
                            if let Some(expect) = &effect_spans.expect {
                                expressions.push(expect.clone());
                            }
                        }
                        let step_spans = step_spans
                            .get(step_index)
                            .expect("effect AST and source span steps stay aligned");
                        if let Some(step_expect) = &step_spans.0 {
                            expressions.push(step_expect.clone());
                        }
                        match &step_spans.1 {
                            TestEffectStepOutcomeSourceSpans::Respond(span)
                            | TestEffectStepOutcomeSourceSpans::Throw(span) => {
                                expressions.push(span.clone())
                            }
                            TestEffectStepOutcomeSourceSpans::Stream(spans) => {
                                expressions.extend(spans.iter().cloned())
                            }
                        }
                        setup_statement_spans.push(StmtSourceSpans {
                            span: effect.span,
                            expressions,
                            blocks: Vec::new(),
                        });
                    }
                }
            }
            let setup = FunctionDecl {
                exported: false,
                name: setup_name.clone(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRef {
                    name: "void".to_string(),
                },
                body: Block {
                    statements: setup_statements,
                },
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
                span: test.span,
            };
            let setup_spans = source_spans.as_ref().map(|_| ExecutableSourceSpans {
                effects: Vec::new(),
                body: BlockSourceSpans {
                    span: test.span,
                    statements: setup_statement_spans,
                },
            });
            let setup_call = Expr::Call {
                callee: Box::new(Expr::Identifier(setup_name)),
                args: Vec::new(),
            };
            let mut body_statements = Vec::with_capacity(test.body.statements.len() + 1);
            body_statements.push(Stmt::Expr(setup_call));
            body_statements.extend(test.body.statements.clone());
            let body = FunctionDecl {
                exported: false,
                name: function_name.to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRef {
                    name: "void".to_string(),
                },
                body: Block {
                    statements: body_statements,
                },
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
                span: test.span,
            };
            let body_spans = source_spans.map(|spans| {
                let call_span = ExprSourceSpans {
                    span: test.span,
                    children: vec![ExprSourceSpans {
                        span: test.span,
                        children: Vec::new(),
                        blocks: Vec::new(),
                        record_fields: Vec::new(),
                    }],
                    blocks: Vec::new(),
                    record_fields: Vec::new(),
                };
                let mut statements = Vec::with_capacity(spans.body.statements.len() + 1);
                statements.push(StmtSourceSpans {
                    span: test.span,
                    expressions: vec![call_span],
                    blocks: Vec::new(),
                });
                statements.extend(spans.body.statements);
                ExecutableSourceSpans {
                    effects: Vec::new(),
                    body: BlockSourceSpans {
                        span: spans.body.span,
                        statements,
                    },
                }
            });
            ((setup_spans, setup), (body_spans, body))
        })
        .collect::<Vec<_>>();
    let mut overlay = ast.clone();
    overlay.tests.clear();
    overlay.test_default_run = None;
    overlay.source_spans.tests.clear();
    for ((setup_spans, setup), (body_spans, body)) in functions {
        if let Some(spans) = setup_spans {
            overlay.source_spans.functions.push(spans);
        }
        overlay.functions.push(setup);
        if let Some(spans) = body_spans {
            overlay.source_spans.functions.push(spans);
        }
        overlay.functions.push(body);
    }
    overlay
}

fn flattened_test_effect_steps(
    outcome: &TestEffectOutcome,
) -> Vec<(Option<Expr>, TestEffectStepOutcome)> {
    match outcome {
        TestEffectOutcome::Respond { value } => vec![(
            None,
            TestEffectStepOutcome::Respond {
                value: value.clone(),
            },
        )],
        TestEffectOutcome::Throw { value } => vec![(
            None,
            TestEffectStepOutcome::Throw {
                value: value.clone(),
            },
        )],
        TestEffectOutcome::Stream { events } => vec![(
            None,
            TestEffectStepOutcome::Stream {
                events: events.clone(),
            },
        )],
        TestEffectOutcome::Sequence { steps } => steps
            .iter()
            .map(|step| (step.expect.clone(), step.outcome.clone()))
            .collect(),
    }
}

fn flattened_test_effect_step_spans(
    outcome: &TestEffectOutcomeSourceSpans,
) -> Vec<(Option<ExprSourceSpans>, TestEffectStepOutcomeSourceSpans)> {
    match outcome {
        TestEffectOutcomeSourceSpans::Respond(value) => vec![(
            None,
            TestEffectStepOutcomeSourceSpans::Respond(value.clone()),
        )],
        TestEffectOutcomeSourceSpans::Throw(value) => {
            vec![(None, TestEffectStepOutcomeSourceSpans::Throw(value.clone()))]
        }
        TestEffectOutcomeSourceSpans::Stream(events) => vec![(
            None,
            TestEffectStepOutcomeSourceSpans::Stream(events.clone()),
        )],
        TestEffectOutcomeSourceSpans::Sequence { steps } => steps
            .iter()
            .map(|step| (step.expect.clone(), step.outcome.clone()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use skiff_syntax::{ast::TestEffectStepOutcome, parser::parse_source};

    use super::*;

    #[test]
    fn inline_effects_become_hidden_setup_called_before_original_body() {
        let ast = parse_source(
            r#"
test "uses dependency" effects {
  dep/run {
    expect: { id: "one" },
    respond: { ok: true },
  }
} {
  assert true;
}
"#,
        )
        .expect("test source parses");
        let overlay = package_test_ast_for_cases(&ast, [(0, "skiffTestCase0")]);
        assert!(overlay.tests.is_empty());
        assert_eq!(overlay.functions.len(), 2);
        assert_eq!(overlay.functions[0].name, "skiffTestCase0Setup");
        let [Stmt::CompilerTestEffectRegister {
            target,
            outcome: TestEffectStepOutcome::Respond { .. },
            ..
        }] = overlay.functions[0].body.statements.as_slice()
        else {
            panic!("setup must contain one compiler-owned registration");
        };
        assert_eq!(target, "dep/run");
        let Stmt::Expr(Expr::Call { callee, args }) = &overlay.functions[1].body.statements[0]
        else {
            panic!("test body must call setup first");
        };
        assert!(args.is_empty());
        assert_eq!(
            callee.as_ref(),
            &Expr::Identifier("skiffTestCase0Setup".into())
        );
        assert_eq!(overlay.source_spans.functions.len(), 2);
    }

    #[test]
    fn sequence_steps_become_ordered_registrations_with_separate_expectations() {
        let ast = parse_source(
            r#"
test "uses sequence" effects {
  dep/run {
    expect: { method: "POST" },
    sequence: [
      {
        expect: { url: "/first" },
        respond: { value: "first" },
      },
      {
        expect: { url: "/second" },
        throw: Failure { message: "second" },
      },
    ],
  }
} {
  assert true;
}
"#,
        )
        .expect("test source parses");
        let overlay = package_test_ast_for_cases(&ast, [(0, "skiffTestCase0")]);
        let [Stmt::CompilerTestEffectRegister {
            expect: Some(_),
            step_expect: Some(_),
            outcome: TestEffectStepOutcome::Respond { .. },
            ..
        }, Stmt::CompilerTestEffectRegister {
            expect: None,
            step_expect: Some(_),
            outcome: TestEffectStepOutcome::Throw { .. },
            ..
        }] = overlay.functions[0].body.statements.as_slice()
        else {
            panic!("sequence must flatten to two ordered compiler-owned registrations");
        };
        assert_eq!(
            overlay.source_spans.functions[0]
                .body
                .statements
                .iter()
                .map(|statement| statement.expressions.len())
                .collect::<Vec<_>>(),
            [4, 3],
            "the common expect is evaluated once; each step keeps its own expect and outcome spans"
        );
    }
}
