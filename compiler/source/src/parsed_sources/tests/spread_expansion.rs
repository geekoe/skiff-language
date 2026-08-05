//! Semantic expansion tests for record `spread` (design
//! `doc/implementation/record-spread-contract-storage.md` §3.3).
//!
//! Same-package cases compile through `build_package_from_parsed_sources` and
//! inspect the expanded AST; cross-package cases compile a provider package
//! first and pass it as dependency source facts.

use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::FileIrUnit;

use super::*;
use crate::{
    build_package_from_parsed_sources, package_dependency_facts::SourceCompilePackageFacts,
    parsed_sources::parse_publication_sources, source_graph::CompilerSourceFile,
    CompileParsedPackageSourcesInput, PackageCompilePolicy, PackageDependency, PackageSourceModel,
    PublicationError,
};

const PROVIDER_ID: &str = "example.com/spread-provider";

fn initialize_prelude() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    crate::prelude_registry::initialize_prelude_registry(
        &skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
            .expect("platform sources load"),
    )
    .expect("prelude registry initializes");
}

fn compile_sources(sources: &[(&str, &str, &str)]) -> Result<PackageSourceModel, PublicationError> {
    initialize_prelude();
    let root = PathBuf::from("/test/spread-expansion");
    let parsed_inputs = sources
        .iter()
        .map(|(path, module, text)| {
            CompilerSourceFile::parse(
                PathBuf::from(path),
                (*module).to_string(),
                false,
                false,
                (*text).to_string(),
                *path,
            )
            .expect("spread test source should parse")
        })
        .collect::<Vec<_>>();
    let parsed_sources = parse_publication_sources(&root, &parsed_inputs)
        .expect("spread test source facts should build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::<PackageDependency>::new();
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new("example.com/spread-expansion"),
    })
}

fn compile(source: &str) -> Result<PackageSourceModel, PublicationError> {
    compile_sources(&[("main.skiff", "main", source)])
}

fn assert_compile_error(source: &str, expected: &str) {
    let error = compile(source).expect_err(source);
    assert!(
        error.to_string().contains(expected),
        "expected {expected:?}, got {error}"
    );
}

fn expanded_fields(
    model: &PackageSourceModel,
    module: &str,
    type_name: &str,
) -> Vec<(String, String)> {
    model
        .sources()
        .parsed_sources()
        .iter()
        .find(|parsed| parsed.source().module_path == module)
        .unwrap_or_else(|| panic!("missing module {module}"))
        .ast()
        .types
        .iter()
        .find(|ty| ty.name == type_name)
        .unwrap_or_else(|| panic!("missing type {type_name}"))
        .fields
        .iter()
        .map(|field| (field.name.clone(), field.ty.name.clone()))
        .collect()
}

fn spreads_of(model: &PackageSourceModel, module: &str, type_name: &str) -> Vec<String> {
    model
        .sources()
        .parsed_sources()
        .iter()
        .find(|parsed| parsed.source().module_path == module)
        .unwrap_or_else(|| panic!("missing module {module}"))
        .ast()
        .types
        .iter()
        .find(|ty| ty.name == type_name)
        .unwrap_or_else(|| panic!("missing type {type_name}"))
        .spreads
        .iter()
        .map(|spread| spread.name.clone())
        .collect()
}

fn provider_model(source: &str) -> PackageSourceModel {
    initialize_prelude();
    let root = PathBuf::from("/test/spread-provider");
    let parsed_input = CompilerSourceFile::parse(
        PathBuf::from("model.skiff"),
        "model".to_string(),
        false,
        false,
        source.to_string(),
        "model.skiff",
    )
    .expect("provider source should parse");
    let parsed_sources = parse_publication_sources(&root, std::slice::from_ref(&parsed_input))
        .expect("provider source facts should build");
    let package_aliases = BTreeMap::new();
    let package_dependencies = Vec::<PackageDependency>::new();
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: None,
        package_artifacts: None,
        policy: PackageCompilePolicy::new(PROVIDER_ID),
    })
    .expect("provider package should compile")
}

fn compile_with_provider(
    source: &str,
    provider: &PackageSourceModel,
) -> Result<PackageSourceModel, PublicationError> {
    initialize_prelude();
    let root = PathBuf::from("/test/spread-host");
    let parsed_input = CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        source.to_string(),
        "main.skiff",
    )
    .expect("host source should parse");
    let parsed_sources = parse_publication_sources(&root, std::slice::from_ref(&parsed_input))
        .expect("host source facts should build");
    let package_facts = vec![SourceCompilePackageFacts::new(
        PROVIDER_ID,
        "1.0.0",
        Vec::new(),
        provider,
        &[] as &[FileIrUnit],
    )];
    let mut dependency = PackageDependency::id(PROVIDER_ID);
    dependency.alias = Some("provider".to_string());
    let package_aliases = BTreeMap::from([("provider".to_string(), vec![String::new()])]);
    let package_dependencies = vec![dependency];
    build_package_from_parsed_sources(CompileParsedPackageSourcesInput {
        parsed_sources,
        production_sources: Vec::new(),
        diagnostic_root: &root,
        publication_api: None,
        package_aliases: &package_aliases,
        package_dependencies: &package_dependencies,
        package_facts: Some(&package_facts),
        package_artifacts: None,
        policy: PackageCompilePolicy::new("example.com/spread-host"),
    })
}

#[test]
fn expands_basic_spread_into_target_fields() {
    let model = compile(
        r#"
        type Base {
          id: string,
          title: string?,
        }

        type Thread {
          spread Base,
          ownerUserId: string,
        }
        "#,
    )
    .expect("basic spread should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![
            ("ownerUserId".to_string(), "string".to_string()),
            ("id".to_string(), "string".to_string()),
            ("title".to_string(), "string?".to_string()),
        ]
    );
    assert!(spreads_of(&model, "main", "Thread").is_empty());
    assert_eq!(
        expanded_fields(&model, "main", "Base"),
        vec![
            ("id".to_string(), "string".to_string()),
            ("title".to_string(), "string?".to_string())
        ]
    );
}

#[test]
fn expands_multiple_spreads_in_order() {
    let model = compile(
        r#"
        type First {
          a: string,
        }

        type Second {
          b: number,
        }

        type Combined {
          spread First,
          spread Second,
          own: bool,
        }
        "#,
    )
    .expect("multiple spreads should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Combined"),
        vec![
            ("own".to_string(), "bool".to_string()),
            ("a".to_string(), "string".to_string()),
            ("b".to_string(), "number".to_string()),
        ]
    );
}

#[test]
fn qualifies_cross_module_field_types_with_source_module_root() {
    let model = compile_sources(&[
        (
            "model/base.skiff",
            "model",
            r#"
                type Metadata {
                  id: string,
                }

                type Base {
                  meta: Metadata,
                  title: string?,
                }
            "#,
        ),
        (
            "api/thread.skiff",
            "api.thread",
            r#"
                type Thread {
                  spread root.model.Base,
                  owner: string,
                }
            "#,
        ),
    ])
    .expect("cross-module spread should compile and resolve");
    assert_eq!(
        expanded_fields(&model, "api.thread", "Thread"),
        vec![
            ("owner".to_string(), "string".to_string()),
            ("meta".to_string(), "root.model.Metadata".to_string()),
            ("title".to_string(), "string?".to_string()),
        ]
    );
}

#[test]
fn keeps_already_qualified_field_type_texts() {
    let model = compile_sources(&[
        ("model/meta.skiff", "model", "type Metadata { id: string }"),
        (
            "api/thread.skiff",
            "api.thread",
            r#"
                type Base {
                  meta: root.model.Metadata,
                  title: string?,
                }

                type Thread {
                  spread Base,
                }
            "#,
        ),
    ])
    .expect("already-qualified field texts should keep resolving");
    assert_eq!(
        expanded_fields(&model, "api.thread", "Thread"),
        vec![
            ("meta".to_string(), "root.model.Metadata".to_string()),
            ("title".to_string(), "string?".to_string()),
        ]
    );
}

#[test]
fn rejects_duplicate_field_between_spreads() {
    assert_compile_error(
        r#"
        type First {
          id: string,
        }

        type Second {
          id: string,
        }

        type Combined {
          spread First,
          spread Second,
        }
        "#,
        "field `id` conflicts with another field on record `main.Combined`",
    );
}

#[test]
fn rejects_duplicate_field_between_spread_and_explicit_field() {
    assert_compile_error(
        r#"
        type Base {
          id: string,
        }

        type Thread {
          spread Base,
          id: string,
        }
        "#,
        "field `id` conflicts with another field on record `main.Thread`",
    );
}

#[test]
fn rejects_self_spread() {
    assert_compile_error(
        r#"
        type Node {
          spread Node,
          value: string,
        }
        "#,
        "spread cycle detected: main.Node -> main.Node",
    );
}

#[test]
fn rejects_cyclic_spread_chain() {
    assert_compile_error(
        r#"
        type First {
          spread Second,
          a: string,
        }

        type Second {
          spread First,
          b: string,
        }
        "#,
        "spread cycle detected: main.First -> main.Second -> main.First",
    );
}

#[test]
fn rejects_spread_cycle_through_third_type() {
    assert_compile_error(
        r#"
        type First {
          spread Second,
          a: string,
        }

        type Second {
          spread Third,
          b: string,
        }

        type Third {
          spread First,
          c: string,
        }
        "#,
        "spread cycle detected",
    );
}

#[test]
fn expands_generic_source_with_closed_arguments() {
    let model = compile(
        r#"
        type Box<T> {
          value: T,
          items: Array<T>,
        }

        type Holder {
          spread Box<number>,
          label: string,
        }
        "#,
    )
    .expect("generic spread with closed arguments should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Holder"),
        vec![
            ("label".to_string(), "string".to_string()),
            ("value".to_string(), "number".to_string()),
            ("items".to_string(), "Array<number>".to_string()),
        ]
    );
}

#[test]
fn rejects_bare_generic_spread_source() {
    assert_compile_error(
        r#"
        type Box<T> {
          value: T,
        }

        type Holder {
          spread Box,
        }
        "#,
        "spread source `main.Box` expects 1 type arguments, found 0",
    );
}

#[test]
fn rejects_generic_spread_arguments_referencing_target_type_params() {
    assert_compile_error(
        r#"
        type Box<T> {
          value: T,
        }

        type Holder<X> {
          spread Box<X>,
        }
        "#,
        "type argument `X` references the target type parameter `X`",
    );
}

#[test]
fn rejects_generic_spread_argument_arity_mismatch() {
    assert_compile_error(
        r#"
        type Pair<A, B> {
          first: A,
          second: B,
        }

        type Holder {
          spread Pair<int>,
        }
        "#,
        "expects 2 type arguments, found 1",
    );
}

#[test]
fn rejects_representation_spread_source() {
    assert_compile_error(
        r#"
        type Shape = string

        type Holder {
          spread Shape,
        }
        "#,
        "spread source `Shape` is not a record; representation, named union, actor, and interface declarations cannot be spread",
    );
}

#[test]
fn rejects_named_union_spread_source() {
    assert_compile_error(
        r#"
        type Either = A | B

        type Holder {
          spread Either,
        }
        "#,
        "is not a record; representation, named union, actor, and interface declarations cannot be spread",
    );
}

#[test]
fn rejects_interface_spread_source() {
    assert_compile_error(
        r#"
        interface Reader {
          function read() -> string
        }

        type Holder {
          spread Reader,
        }
        "#,
        "is not a record",
    );
}

#[test]
fn expands_transparent_alias_source_to_record_fields() {
    let model = compile(
        r#"
        type Base {
          id: string,
        }

        alias LocalBase = Base

        type Thread {
          spread LocalBase,
          owner: string,
        }
        "#,
    )
    .expect("alias spread source should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![
            ("owner".to_string(), "string".to_string()),
            ("id".to_string(), "string".to_string()),
        ]
    );
}

#[test]
fn expands_alias_chain_source_to_record_fields() {
    let model = compile(
        r#"
        type Base {
          id: string,
        }

        alias LocalBase = Base
        alias PublicBase = LocalBase

        type Thread {
          spread PublicBase,
        }
        "#,
    )
    .expect("alias chain spread source should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![("id".to_string(), "string".to_string())]
    );
}

#[test]
fn rejects_alias_source_to_representation() {
    assert_compile_error(
        r#"
        type Shape = string
        alias LocalShape = Shape

        type Holder {
          spread LocalShape,
        }
        "#,
        "is not a record",
    );
}

#[test]
fn rejects_unknown_spread_source() {
    assert_compile_error(
        r#"
        type Holder {
          spread Missing,
        }
        "#,
        "spread source `Missing` is not visible in module `main`",
    );
}

#[test]
fn rejects_unknown_cross_module_spread_source() {
    assert_compile_error(
        r#"
        type Holder {
          spread root.other.Missing,
        }
        "#,
        "spread source `root.other.Missing` is not visible in module `main`",
    );
}

#[test]
fn rejects_unknown_dependency_spread_source() {
    assert_compile_error(
        r#"
        type Holder {
          spread unknownpkg.model.Thing,
        }
        "#,
        "spread source `unknownpkg.model.Thing` is not a declared dependency alias or a module of the current package",
    );
}

#[test]
fn recursive_alias_spread_source_is_rejected_at_alias_cycle_detection() {
    let source = CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        r#"
        alias First = Second
        alias Second = First

        type Holder {
          spread First,
        }
        "#
        .to_string(),
        "main.skiff",
    )
    .expect("source parses");
    let error = parse_publication_sources(Path::new("/test/spread-expansion"), &[source])
        .expect_err("recursive aliases are rejected before expansion");
    assert!(
        error
            .to_string()
            .contains("recursive alias cycle main.First -> main.Second -> main.First"),
        "unexpected error: {error}"
    );
}

#[test]
fn spread_field_name_is_regular_field() {
    let model = compile(
        r#"
        type Base {
          id: string,
        }

        type Event {
          spread: Base,
          kind: string,
        }
        "#,
    )
    .expect("spread followed by colon is a regular field");
    assert_eq!(
        expanded_fields(&model, "main", "Event"),
        vec![
            ("spread".to_string(), "Base".to_string()),
            ("kind".to_string(), "string".to_string()),
        ]
    );
}

#[test]
fn spread_source_can_be_spread_again() {
    let model = compile(
        r#"
        type Base {
          id: string,
        }

        type Middle {
          spread Base,
          mid: string,
        }

        type Top {
          spread Middle,
          top: string,
        }
        "#,
    )
    .expect("chained same-package spreads should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Top"),
        vec![
            ("top".to_string(), "string".to_string()),
            ("mid".to_string(), "string".to_string()),
            ("id".to_string(), "string".to_string()),
        ]
    );
}

#[test]
fn cross_package_spread_copies_and_qualifies_provider_fields() {
    let provider = provider_model(
        r#"
        type Metadata {
          id: string,
        }

        type AgentThread {
          ownerUserId: string,
          meta: Metadata,
        }
        "#,
    );
    let model = compile_with_provider(
        r#"
        type Thread {
          spread provider/model.AgentThread,
          pinnedAt: string?,
        }
        "#,
        &provider,
    )
    .expect("cross-package spread should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![
            ("pinnedAt".to_string(), "string?".to_string()),
            ("ownerUserId".to_string(), "string".to_string()),
            ("meta".to_string(), "provider.model.Metadata".to_string()),
        ]
    );
}

#[test]
fn cross_package_spread_supports_dot_spelling() {
    let provider = provider_model(
        r#"
        type AgentThread {
          ownerUserId: string,
        }
        "#,
    );
    let model = compile_with_provider(
        r#"
        type Thread {
          spread provider.model.AgentThread,
        }
        "#,
        &provider,
    )
    .expect("dot-spelled cross-package spread should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![("ownerUserId".to_string(), "string".to_string())]
    );
}

#[test]
fn cross_package_spread_of_provider_record_that_itself_spreads() {
    let provider = provider_model(
        r#"
        type Base {
          id: string,
        }

        type AgentThread {
          spread Base,
          ownerUserId: string,
        }
        "#,
    );
    let model = compile_with_provider(
        r#"
        type Thread {
          spread provider/model.AgentThread,
        }
        "#,
        &provider,
    )
    .expect("cross-package spread of a spread-using provider record should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![
            ("ownerUserId".to_string(), "string".to_string()),
            ("id".to_string(), "string".to_string()),
        ]
    );
}

#[test]
fn cross_package_generic_spread_substitutes_provider_type_params() {
    let provider = provider_model(
        r#"
        type Box<T> {
          value: T,
          items: Array<T>,
        }
        "#,
    );
    let model = compile_with_provider(
        r#"
        type Holder {
          spread provider/model.Box<number>,
        }
        "#,
        &provider,
    )
    .expect("cross-package generic spread should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Holder"),
        vec![
            ("value".to_string(), "number".to_string()),
            ("items".to_string(), "Array<number>".to_string()),
        ]
    );
}

#[test]
fn cross_package_bare_generic_spread_rejected() {
    let provider = provider_model(
        r#"
        type Box<T> {
          value: T,
        }
        "#,
    );
    let error = compile_with_provider(
        r#"
        type Holder {
          spread provider/model.Box,
        }
        "#,
        &provider,
    )
    .expect_err("bare generic cross-package spread must fail");
    assert!(
        error
            .to_string()
            .contains("expects 1 type arguments, found 0"),
        "unexpected error: {error}"
    );
}

#[test]
fn cross_package_representation_spread_rejected() {
    let provider = provider_model(
        r#"
        type Shape = string
        "#,
    );
    let error = compile_with_provider(
        r#"
        type Holder {
          spread provider/model.Shape,
        }
        "#,
        &provider,
    )
    .expect_err("cross-package representation spread must fail");
    assert!(
        error
            .to_string()
            .contains("is a representation declaration, which cannot be spread"),
        "unexpected error: {error}"
    );
}

#[test]
fn cross_package_duplicate_fields_rejected() {
    let provider = provider_model(
        r#"
        type AgentThread {
          ownerUserId: string,
        }
        "#,
    );
    let error = compile_with_provider(
        r#"
        type Thread {
          spread provider/model.AgentThread,
          ownerUserId: string,
        }
        "#,
        &provider,
    )
    .expect_err("duplicate cross-package field must fail");
    assert!(
        error.to_string().contains("field `ownerUserId` conflicts"),
        "unexpected error: {error}"
    );
}

#[test]
fn cross_package_missing_symbol_rejected() {
    let provider = provider_model(
        r#"
        type AgentThread {
          ownerUserId: string,
        }
        "#,
    );
    let error = compile_with_provider(
        r#"
        type Thread {
          spread provider/model.Unknown,
        }
        "#,
        &provider,
    )
    .expect_err("unknown cross-package symbol must fail");
    assert!(
        error
            .to_string()
            .contains("does not resolve to a record in dependency"),
        "unexpected error: {error}"
    );
}

#[test]
fn cross_package_alias_source_resolves_to_record() {
    let provider = provider_model(
        r#"
        type AgentThread {
          ownerUserId: string,
        }

        alias PublicThread = AgentThread
        "#,
    );
    let model = compile_with_provider(
        r#"
        type Thread {
          spread provider/model.PublicThread,
        }
        "#,
        &provider,
    )
    .expect("cross-package alias spread source should compile");
    assert_eq!(
        expanded_fields(&model, "main", "Thread"),
        vec![("ownerUserId".to_string(), "string".to_string())]
    );
}
