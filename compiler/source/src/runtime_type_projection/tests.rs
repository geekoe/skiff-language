use std::{collections::BTreeMap, path::PathBuf};

use skiff_compiler_input::CompilerPlatformSources;

use super::*;
use crate::prelude_registry::initialize_prelude_registry;

fn initialize_test_prelude() {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves");
    let platform_sources =
        CompilerPlatformSources::new(&platform_root).expect("platform sources load");
    initialize_prelude_registry(&platform_sources).expect("prelude registry initializes");
}

#[test]
fn prelude_declarations_preserve_record_representation_and_named_union_kinds() {
    initialize_test_prelude();
    let registry = prelude_registry();

    let record = lower_prelude_type_decl(
        registry
            .type_decl("std.json.DecodeError")
            .expect("DecodeError declaration"),
    )
    .expect("record declaration lowers");
    assert!(matches!(
        record.descriptor,
        TypeDescriptorIr::Record { ref fields }
            if fields["target"] == TypeRefIr::builtin("string")
                && fields["message"] == TypeRefIr::builtin("string")
    ));

    let representation = lower_prelude_type_decl(
        registry
            .type_decl("std.time.Duration")
            .expect("Duration declaration"),
    )
    .expect("representation declaration lowers");
    assert!(matches!(
        representation.descriptor,
        TypeDescriptorIr::Representation { ref representation }
            if representation == &TypeRefIr::builtin("integer")
    ));

    let union = lower_prelude_type_decl(
        registry
            .type_decl("std.http.HttpResponseStreamEvent")
            .expect("HttpResponseStreamEvent declaration"),
    )
    .expect("named union declaration lowers");
    let TypeDescriptorIr::Union { branches } = union.descriptor else {
        panic!("prelude named union must remain a named union");
    };
    assert_eq!(branches.len(), 3);
    assert!(branches.iter().all(|branch| matches!(
        branch,
        NamedUnionBranchIr::SyntheticDiscriminator {
            discriminator_field,
            ..
        } if discriminator_field == "tag"
    )));
}

#[test]
fn platform_error_type_ir_fields_come_from_exact_source_declarations() {
    initialize_test_prelude();
    let registry = prelude_registry();

    for (symbol, expected_fields) in [
        (
            "std.collection.ArrayIndexOutOfBoundsError",
            vec![("index", "integer"), ("length", "integer")],
        ),
        ("std.collection.JsonObjectPropertyNotFoundError", vec![]),
        ("std.collection.MapKeyNotFoundError", vec![]),
        (
            "std.error.InstructionLimitExceededError",
            vec![("instructionCount", "integer"), ("limit", "integer")],
        ),
        ("std.error.TimeoutError", vec![("timeoutMs", "integer")]),
        (
            "std.http.RequestTimeoutError",
            vec![("timeoutMs", "integer")],
        ),
        (
            "std.actor.ActivationTimeoutError",
            vec![("timeoutMs", "integer")],
        ),
        (
            "std.actor.MethodInvocationTimeoutError",
            vec![("timeoutMs", "integer")],
        ),
    ] {
        let declaration = registry
            .type_decl(symbol)
            .unwrap_or_else(|| panic!("{symbol} declaration must resolve"));
        let lowered = lower_prelude_type_decl(declaration)
            .unwrap_or_else(|error| panic!("{symbol} must lower: {error}"));
        let expected_fields = expected_fields
            .into_iter()
            .map(|(name, ty)| (name.to_string(), TypeRefIr::builtin(ty)))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            lowered.descriptor,
            TypeDescriptorIr::Record {
                fields: expected_fields
            },
            "{symbol} must lower exactly from its parsed source fields"
        );
    }
}
