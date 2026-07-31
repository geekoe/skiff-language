use std::path::PathBuf;

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
