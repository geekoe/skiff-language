use std::{collections::BTreeSet, path::PathBuf};

use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeRef, PackageLocalAbiSymbol, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler::CompilerPlatformSources;
use skiff_compiler_core::prelude_registry::{
    compiler_builtin_type, file_ir_builtin_source_spellings, CompilerBuiltinTypeKind,
};
use skiff_compiler_lowering::source_file_lowering::compile_source_file_ir_unit;
use skiff_compiler_source::prelude_registry::initialize_prelude_registry;

mod common;
use common::{
    artifacts::{module_artifact, source_artifact},
    package_project::compile_package_project,
    TestDir,
};

const CURRENT_STD_BUILD: &str =
    "skiff-package-build-v10:sha256:3604e31ffac0e1a12432e213fb895a51fef18355b365e1d897147a6c43924695";
const CURRENT_STD_LOCAL_ABI: &str =
    "skiff-package-local-abi-v7:sha256:a3923f5b29d9f1ac7373c679e6bcac4b13a1687ae29db4a98a1c73013509cc9e";
const CURRENT_STD_SCHEMA_INDEX: &str =
    "skiff-package-schema-index-v1:sha256:26b7640548d50a600c5e04e0b61851eb66d43b34bca65c26da99bacec2a7f577";
const CURRENT_CONFLICT_ERROR_SCHEMA: &str =
    "skiff-package-schema-type-v2:sha256:55e0f59a69a2facc339d89ba12be27a0aaec3e1a60b3211b43259d153b480a4d";
const CURRENT_DB_FILE_IR: &str =
    "skiff-file-ir-v8:sha256:e62485ea5dcd42c0e4552db0e4271bc8bd573ca7478a09bfa238bd2183976cf8";

#[test]
fn declared_source_aliases_emit_only_canonical_file_ir_builtin_names() {
    assert_direct_lowering_canonicalizes_qualified_aliases();

    let temp = TestDir::new("skiff-compiler", "canonical-builtin-spelling");
    temp.write(
        "package.yml",
        "id: example.com/canonical-builtin-spelling\nversion: 1.0.0\n",
    );
    temp.write("api.yml", "check: main.check\n");
    temp.write(
        "main.skiff",
        r#"import std

type CanonicalBuiltinProbe {
  boolBare: bool,
  boolAlias: boolean,
  arrayBare: Array<boolean>,
  mapNested: Map<string, Array<boolean?>>,
  callback: fn(flag: boolean) -> Stream<boolean>,
  choice: boolean | string,
  nested: { flag: boolean, payload: bytes },
  request: std.http.HttpRequest,
}

function check(flag: boolean) -> bool {
  return flag
}
"#,
    );

    let project =
        compile_package_project(temp.path()).expect("declared builtin aliases should compile");
    let main = module_artifact(&project.package, "main");
    let probe = main
        .unit
        .type_table
        .iter()
        .find(|ty| ty.name == "CanonicalBuiltinProbe")
        .expect("probe type should be emitted");
    let TypeDescriptorIr::Record { fields } = &probe.descriptor else {
        panic!("probe must be a record");
    };
    assert_eq!(fields["boolBare"], TypeRefIr::builtin("bool"));
    assert_eq!(fields["boolAlias"], fields["boolBare"]);

    let declared_aliases = file_ir_builtin_source_spellings()
        .filter(|builtin| builtin.source_spelling != builtin.canonical_name)
        .map(|builtin| builtin.source_spelling)
        .collect::<BTreeSet<_>>();
    let mut observed_names = BTreeSet::new();
    for package in project.artifacts() {
        assert_surface_is_canonical(
            &format!("{} PackageArtifact", package.artifact.package_id),
            &serde_json::to_value(&package.artifact).unwrap(),
            &declared_aliases,
            &mut observed_names,
        );
        assert_surface_is_canonical(
            &format!("{} PackageSchema index", package.artifact.package_id),
            &serde_json::to_value(&package.package_schema_index).unwrap(),
            &declared_aliases,
            &mut observed_names,
        );
        assert_surface_is_canonical(
            &format!("{} PackageSchema records", package.artifact.package_id),
            &serde_json::to_value(&package.package_schema_type_records).unwrap(),
            &declared_aliases,
            &mut observed_names,
        );
        for file in &package.file_ir_units {
            assert_surface_is_canonical(
                &format!(
                    "{} FileIR {}",
                    package.artifact.package_id, file.module_path
                ),
                &file.value(),
                &declared_aliases,
                &mut observed_names,
            );
        }
    }
    assert!(
        observed_names.contains("bool"),
        "canonical bool must remain present in emitted artifacts"
    );

    assert_fresh_std_conflict_error_is_canonical(&project);
}

#[test]
fn undeclared_builtin_spellings_are_not_implicit_source_aliases() {
    for spelling in ["String", "Bytes"] {
        let temp = TestDir::new(
            "skiff-compiler",
            &format!("noncanonical-builtin-{spelling}"),
        );
        temp.write(
            "package.yml",
            "id: example.com/noncanonical-builtin\nversion: 1.0.0\n",
        );
        temp.write("api.yml", "Bad: main.Bad\n");
        temp.write("main.skiff", format!("type Bad {{ value: {spelling} }}\n"));

        let error = compile_package_project(temp.path())
            .expect_err("undeclared spelling must not compile as a builtin alias");
        assert!(
            error.to_string().contains(spelling),
            "{spelling} failure should retain the rejected source spelling: {error}"
        );
    }
}

#[test]
fn compiler_builtin_registry_retires_cancel_error_and_keeps_timeout_error() {
    for spelling in ["CancelError", "std.error.CancelError"] {
        assert!(
            compiler_builtin_type(spelling).is_none(),
            "retired cancellation spelling {spelling} must have no compiler builtin owner"
        );
    }

    let timeout = compiler_builtin_type("TimeoutError")
        .expect("TimeoutError must retain its compiler builtin owner");
    assert_eq!(timeout.symbol, "std.error.TimeoutError");
    assert_eq!(timeout.arity, 0);
    assert_eq!(timeout.kind, CompilerBuiltinTypeKind::Error);
    assert_eq!(compiler_builtin_type(timeout.symbol), Some(timeout));
}

#[test]
fn cancel_error_short_and_qualified_type_spellings_are_rejected() {
    assert_cancel_error_spellings_are_rejected("type", |spelling| {
        format!("type Bad {{ value: {spelling} }}\n")
    });
}

#[test]
fn cancel_error_short_and_qualified_constructors_are_rejected() {
    assert_cancel_error_spellings_are_rejected("constructor", |spelling| {
        format!(
            r#"function bad() -> {spelling} {{
  return {spelling} {{}}
}}
"#
        )
    });
}

#[test]
fn cancel_error_short_and_qualified_throw_payloads_are_rejected() {
    assert_cancel_error_spellings_are_rejected("throw", |spelling| {
        format!(
            r#"function bad(value: {spelling}) -> void {{
  throw value
}}
"#
        )
    });
}

#[test]
fn cancel_error_short_and_qualified_catch_types_are_rejected() {
    assert_cancel_error_spellings_are_rejected("catch", |spelling| {
        format!(
            r#"function bad(value: TimeoutError) -> void {{
  const attempted = catch<{spelling}>(value)
}}
"#
        )
    });
}

#[test]
fn cancel_error_short_and_qualified_rethrow_envelopes_are_rejected() {
    assert_cancel_error_spellings_are_rejected("rethrow", |spelling| {
        format!(
            r#"function bad(exception: Exception<{spelling}>) -> void {{
  rethrow exception
}}
"#
        )
    });
}

#[test]
fn cancel_error_short_and_qualified_union_leaves_are_rejected() {
    assert_cancel_error_spellings_are_rejected("union-leaf", |spelling| {
        format!(
            r#"function bad(value: TimeoutError) -> void {{
  const attempted = catch<TimeoutError | {spelling}>(value)
}}
"#
        )
    });
}

fn assert_cancel_error_spellings_are_rejected(surface: &str, source: impl Fn(&str) -> String) {
    initialize_test_prelude_registry();
    for spelling in ["CancelError", "std.error.CancelError"] {
        let error = compile_source_file_ir_unit(
            &source(spelling),
            format!("retired-cancel-error-{surface}.skiff"),
            "retired",
            "package",
        )
        .expect_err("retired cancellation error spelling must fail source compilation");
        let diagnostic = error.to_string();
        assert!(
            diagnostic.contains(spelling),
            "{surface} failure should retain rejected spelling {spelling}: {error}"
        );
        assert!(
            diagnostic.contains("unresolved type")
                || diagnostic.contains("unknown compiler-owned type"),
            "{surface} must fail because {spelling} cannot resolve: {error}"
        );
    }
}

fn assert_direct_lowering_canonicalizes_qualified_aliases() {
    initialize_test_prelude_registry();
    let unit = compile_source_file_ir_unit(
        r#"type QualifiedBuiltinProbe {
  actor: std.actor.Actor<string>,
  bytesValue: std.bytes.bytes,
  arrayValue: std.collection.Array<boolean>,
  mapValue: std.collection.Map<string, boolean>,
  configValue: config.Config,
  streamValue: std.stream.Stream<boolean>,
  exception: std.error.Exception<boolean>,
  catchResult: std.error.CatchResult<boolean, std.error.TimeoutError>,
  sourceLocation: std.error.SourceLocation,
  stackTrace: std.error.StackTrace,
  stackFrame: std.error.StackFrame,
  timeout: std.error.TimeoutError,
  session: std.session.ClientSessionRef,
  capability: std.session.ClientCapability,
  callback: fn(flag: boolean) -> std.collection.Array<boolean?>,
  nested: { choice: boolean | string, map: std.collection.Map<string, boolean> },
}"#,
        "qualified.skiff",
        "qualified",
        "package",
    )
    .expect("standalone FileIR lowering should accept declared qualified builtin spellings");
    let value = serde_json::to_value(&unit).unwrap();
    let declared_aliases = file_ir_builtin_source_spellings()
        .filter(|builtin| builtin.source_spelling != builtin.canonical_name)
        .map(|builtin| builtin.source_spelling)
        .collect::<BTreeSet<_>>();
    let mut observed_names = BTreeSet::new();
    assert_surface_is_canonical(
        "direct qualified FileIR",
        &value,
        &declared_aliases,
        &mut observed_names,
    );
    for canonical in [
        "Actor",
        "bytes",
        "Array",
        "Map",
        "Config",
        "Stream",
        "Exception",
        "CatchResult",
        "SourceLocation",
        "StackTrace",
        "StackFrame",
        "TimeoutError",
        "ClientSessionRef",
        "ClientCapability",
        "bool",
    ] {
        assert!(
            observed_names.contains(canonical),
            "direct lowering should emit canonical builtin {canonical}: {value}"
        );
    }
}

fn initialize_test_prelude_registry() {
    let compiler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let platform_root = compiler_root
        .parent()
        .expect("compiler crate should be directly below the workspace root");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(platform_root).expect("platform sources should load"),
    )
    .expect("prelude registry should initialize");
}

fn assert_fresh_std_conflict_error_is_canonical(
    project: &common::package_project::PublishedPackageProject,
) {
    let std = project
        .dependency("skiff.run/std", "1.0.0")
        .expect("explicit std import should materialize fresh std");
    let db = source_artifact(std, "db.skiff");
    let conflict = db
        .unit
        .type_table
        .iter()
        .find(|ty| ty.name == "ConflictError")
        .expect("std.db.ConflictError should be emitted");
    assert_eq!(
        record_field(&conflict.descriptor, "retryable"),
        &TypeRefIr::builtin("bool")
    );

    let implementation = std.artifact.implementation_links.types["std.db.ConflictError"]
        .descriptor
        .as_ref()
        .expect("ConflictError implementation link should carry its descriptor");
    assert_eq!(
        record_field(implementation, "retryable"),
        &TypeRefIr::builtin("bool")
    );

    for symbols in [
        &std.artifact.package_local_abi.public_symbols,
        &std.artifact.package_local_abi.implementation_symbols,
    ] {
        let PackageLocalAbiSymbol::Type { descriptor, .. } = &symbols["std.db.ConflictError"]
        else {
            panic!("ConflictError should be a Local ABI type");
        };
        assert_eq!(
            record_field(descriptor, "retryable"),
            &TypeRefIr::builtin("bool")
        );
    }

    let schema_entry = &std.package_schema_index.types["std.db.ConflictError"];
    let schema_record = &std.package_schema_type_records[&schema_entry.package_schema_type_id];
    let ContractTypeDescriptor::Record { fields } = &schema_record.canonical_descriptor.descriptor
    else {
        panic!("ConflictError PackageSchema should be a record");
    };
    assert_eq!(fields["retryable"], ContractTypeRef::builtin("bool"));
    assert_eq!(
        schema_entry.package_schema_type_id.as_str(),
        CURRENT_CONFLICT_ERROR_SCHEMA
    );
    assert_eq!(
        std.package_schema_index
            .package_schema_index_identity
            .as_str(),
        CURRENT_STD_SCHEMA_INDEX
    );

    assert_eq!(db.identity, CURRENT_DB_FILE_IR);
    assert_eq!(
        std.artifact.package_local_abi.local_abi_identity.as_str(),
        CURRENT_STD_LOCAL_ABI
    );
    assert_eq!(std.artifact.package_build_id.as_str(), CURRENT_STD_BUILD);
}

fn record_field<'a>(descriptor: &'a TypeDescriptorIr, field: &str) -> &'a TypeRefIr {
    let TypeDescriptorIr::Record { fields } = descriptor else {
        panic!("expected record descriptor, found {descriptor:?}");
    };
    &fields[field]
}

fn assert_surface_is_canonical(
    surface: &str,
    value: &serde_json::Value,
    declared_aliases: &BTreeSet<&str>,
    observed_names: &mut BTreeSet<String>,
) {
    let mut names = Vec::new();
    collect_builtin_names(value, &mut names);
    for name in names {
        assert!(
            !declared_aliases.contains(name.as_str()),
            "{surface} emitted noncanonical builtin spelling {name}"
        );
        observed_names.insert(name);
    }
}

fn collect_builtin_names(value: &serde_json::Value, names: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_builtin_names(item, names);
            }
        }
        serde_json::Value::Object(fields) => {
            if fields.get("kind").and_then(serde_json::Value::as_str) == Some("builtin") {
                if let Some(name) = fields.get("name").and_then(serde_json::Value::as_str) {
                    names.push(name.to_string());
                }
            }
            for field in fields.values() {
                collect_builtin_names(field, names);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}
