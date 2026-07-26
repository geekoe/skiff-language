use std::{collections::BTreeSet, path::PathBuf};

use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeRef, PackageLocalAbiSymbol, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler::CompilerPlatformSources;
use skiff_compiler_core::prelude_registry::file_ir_builtin_source_spellings;
use skiff_compiler_lowering::source_file_lowering::compile_source_file_ir_unit;
use skiff_compiler_source::prelude_registry::initialize_prelude_registry;

mod common;
use common::{
    artifacts::{module_artifact, source_artifact},
    package_project::compile_package_project,
    TestDir,
};

const BASELINE_STD_BUILD: &str =
    "skiff-package-build-v9:sha256:8ac1d3ee235fb3f543df52430f1539610ca05c5631a09df22f7c4f4a7b6a8e17";
const BASELINE_STD_LOCAL_ABI: &str =
    "skiff-package-local-abi-v6:sha256:c8be1d04060489a28f827a5313da12ae26891b1d3b21d1085b6e72884c9ab0ea";
const BASELINE_STD_SCHEMA_INDEX: &str =
    "skiff-package-schema-index-v1:sha256:1f70d5626cddaab23d51d52db974a9292cf019cb0161d67ff560c599ed6fd7fe";
const BASELINE_CONFLICT_ERROR_SCHEMA: &str =
    "skiff-package-schema-type-v1:sha256:dd893e08035a093080419ff2c04beda67c1dab2e95ddcc23dec12f9ce6d8bdd0";
const BASELINE_DB_FILE_IR: &str =
    "skiff-file-ir-v8:sha256:bb39d35baa25cbfb50a1d146e21a18a2ad088940d34304b877e13e348543b069";

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

fn assert_direct_lowering_canonicalizes_qualified_aliases() {
    let compiler_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let platform_root = compiler_root
        .parent()
        .expect("compiler crate should be directly below the workspace root");
    initialize_prelude_registry(
        &CompilerPlatformSources::new(platform_root).expect("platform sources should load"),
    )
    .expect("prelude registry should initialize");
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
  cancel: std.error.CancelError,
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
        "CancelError",
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
        BASELINE_CONFLICT_ERROR_SCHEMA
    );
    assert_eq!(
        std.package_schema_index
            .package_schema_index_identity
            .as_str(),
        BASELINE_STD_SCHEMA_INDEX
    );

    assert_ne!(db.identity, BASELINE_DB_FILE_IR);
    assert_ne!(
        std.artifact.package_local_abi.local_abi_identity.as_str(),
        BASELINE_STD_LOCAL_ABI
    );
    assert_eq!(std.artifact.package_build_id.as_str(), BASELINE_STD_BUILD);
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
