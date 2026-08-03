use std::collections::{BTreeMap, BTreeSet};

use super::{
    canonical_file_ir_builtin, compiler_builtin_type, compiler_owned_type_symbol,
    config_prelude_type, file_ir_builtin_source_spellings, module_symbol_root,
    qualified_prelude_type, validate_package_api_public_path, validate_root_projection_metadata,
    CompilerBuiltinTypeKind, FileIrBuiltinTypeKind, COMPILER_BUILTIN_TYPES,
    LANGUAGE_PRIMITIVE_TYPES, PRELUDE_REGISTRY_ID,
};

#[test]
fn metadata_validation_requires_declared_prelude_root() {
    let error = validate_root_projection_metadata(
        &[String::from("config")],
        &BTreeMap::from([(
            String::from("std"),
            BTreeMap::from([(String::from("string"), String::from("std.string"))]),
        )]),
        &[String::from("std.string")],
    )
    .unwrap_err();

    assert_eq!(
        error,
        "rootProjections includes std, but std is not declared in prelude.roots"
    );
}

#[test]
fn metadata_validation_requires_backing_source_module() {
    let error = validate_root_projection_metadata(
        &[String::from("std")],
        &BTreeMap::from([(
            String::from("std"),
            BTreeMap::from([(String::from("string"), String::from("std.string"))]),
        )]),
        &[String::from("std.number")],
    )
    .unwrap_err();

    assert_eq!(
            error,
            "rootProjections.std.string points to std.string, but no standard_library source module provides it"
        );
}

#[test]
fn module_symbol_root_maps_prelude_modules() {
    assert_eq!(module_symbol_root(PRELUDE_REGISTRY_ID, "config"), "config");
    assert_eq!(
        module_symbol_root(PRELUDE_REGISTRY_ID, "collection"),
        "std.collection"
    );
    assert_eq!(
        module_symbol_root(PRELUDE_REGISTRY_ID, "std.string"),
        "std.string"
    );
    assert_eq!(
        module_symbol_root("example.com/pkg", "api"),
        "example.com/pkg.api"
    );
}

#[test]
fn prelude_type_helpers_parse_supported_forms() {
    assert_eq!(
        qualified_prelude_type("std.collection.Array"),
        Some(("std.collection", "Array"))
    );
    assert_eq!(qualified_prelude_type("std.collection.deep.Array"), None);
    assert_eq!(
        config_prelude_type("config.DecodeError"),
        Some(("config", "DecodeError"))
    );
    assert_eq!(config_prelude_type("config.deep.Type"), None);
    assert_eq!(compiler_owned_type_symbol("JsonObject"), Some("JsonObject"));
}

#[test]
fn compiler_builtin_registry_owns_identity_kind_and_arity() {
    let array = compiler_builtin_type("Array").unwrap();
    assert_eq!(array.symbol, "std.collection.Array");
    assert_eq!(array.arity, 1);
    assert_eq!(array.kind, CompilerBuiltinTypeKind::Container);
    assert_eq!(compiler_builtin_type(array.symbol), Some(array));

    let session = compiler_builtin_type("ClientSessionRef").unwrap();
    assert_eq!(session.symbol, "std.session.ClientSessionRef");
    assert_eq!(session.arity, 0);
    assert_eq!(session.kind, CompilerBuiltinTypeKind::OpaqueHandle);

    let task_ref = compiler_builtin_type("TaskRef").unwrap();
    assert_eq!(task_ref.symbol, "std.task.TaskRef");
    assert_eq!(task_ref.arity, 0);
    assert_eq!(task_ref.kind, CompilerBuiltinTypeKind::OpaqueHandle);
    assert_eq!(compiler_builtin_type(task_ref.symbol), Some(task_ref));

    let status = compiler_builtin_type("TaskStatus").unwrap();
    assert_eq!(status.symbol, "std.task.TaskStatus");
    assert_eq!(status.arity, 0);
    assert_eq!(status.kind, CompilerBuiltinTypeKind::Value);
    assert_eq!(compiler_builtin_type(status.symbol), Some(status));

    let cancel_result = compiler_builtin_type("TaskCancelResult").unwrap();
    assert_eq!(cancel_result.symbol, "std.task.TaskCancelResult");
    assert_eq!(cancel_result.arity, 0);
    assert_eq!(cancel_result.kind, CompilerBuiltinTypeKind::Value);
    assert_eq!(compiler_builtin_type(cancel_result.symbol), Some(cancel_result));

    assert!(compiler_builtin_type("ActorRef").is_none());
    assert!(compiler_builtin_type("NotABuiltin").is_none());
}

#[test]
fn canonical_file_ir_builtin_spelling_registry_is_complete_and_collision_free() {
    let spellings = file_ir_builtin_source_spellings().collect::<Vec<_>>();
    let mut source_spellings = BTreeMap::new();
    for spelling in &spellings {
        assert!(
            source_spellings
                .insert(spelling.source_spelling, spelling.canonical_name)
                .is_none(),
            "source builtin spelling {} must have exactly one canonical owner",
            spelling.source_spelling
        );
    }

    let mut canonical_compiler_names = BTreeSet::new();
    for builtin in COMPILER_BUILTIN_TYPES {
        assert!(
            canonical_compiler_names.insert(builtin.name),
            "compiler builtin canonical name {} must be unique",
            builtin.name
        );
        for source_spelling in [builtin.name, builtin.symbol] {
            let resolved = canonical_file_ir_builtin(source_spelling)
                .unwrap_or_else(|| panic!("{source_spelling} must resolve"));
            assert_eq!(resolved.canonical_name, builtin.name);
            assert_eq!(resolved.arity, builtin.arity);
            assert_eq!(resolved.kind, FileIrBuiltinTypeKind::Compiler(builtin.kind));
        }
    }

    for primitive in LANGUAGE_PRIMITIVE_TYPES {
        let resolved = canonical_file_ir_builtin(primitive.source_spelling)
            .unwrap_or_else(|| panic!("{} must resolve", primitive.source_spelling));
        assert_eq!(resolved.canonical_name, primitive.canonical_name);
        assert_eq!(resolved.arity, 0);
        assert_eq!(resolved.kind, FileIrBuiltinTypeKind::LanguagePrimitive);
    }
    assert_eq!(
        canonical_file_ir_builtin("boolean").map(|builtin| builtin.canonical_name),
        Some("bool")
    );
    assert_eq!(
        canonical_file_ir_builtin("bool").map(|builtin| builtin.canonical_name),
        Some("bool")
    );
    for unknown in ["String", "Bytes", "std.date.Date", "NotABuiltin"] {
        assert_eq!(canonical_file_ir_builtin(unknown), None);
    }
}

#[test]
fn package_api_public_path_validation_is_context_free() {
    let mut violations = Vec::new();
    validate_package_api_public_path("example.com/pkg.api", "example.com/pkg", &mut violations);
    validate_package_api_public_path("bad-path", "example.com/pkg", &mut violations);

    assert_eq!(
        violations,
        vec![
            "api key example.com/pkg.api must be a valid dotted public path or empty string",
            "api key example.com/pkg.api must not contain package or service id example.com/pkg",
            "api key bad-path must be a valid dotted public path or empty string",
        ]
    );
}
