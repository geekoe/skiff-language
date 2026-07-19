use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;

use self::fixtures::{
    actor_file_ir, projected_exports, projected_exports_with_source_module,
    projected_public_instance, public_instance_file_ir, public_instance_method,
};

mod fixtures;

#[test]
fn public_paths_remain_map_keys_while_payload_symbols_name_exact_declarations() {
    let exports = projected_exports(
        "example.com/actors",
        "api.ActorAlias",
        "api.defaultActorAlias",
        "api.runAlias",
        actor_file_ir(),
    )
    .expect("fixture exports must project");

    assert_eq!(
        exports
            .types
            .get("api.ActorAlias")
            .expect("type export at its public path")
            .symbol,
        "Actor"
    );
    assert_eq!(
        exports
            .constants
            .get("api.defaultActorAlias")
            .expect("const export at its public path")
            .symbol,
        "DEFAULT_ACTOR"
    );
    assert_eq!(
        exports
            .functions
            .get("api.runAlias")
            .expect("function export at its public path")
            .symbol,
        "run"
    );
    assert_eq!(exports.types.len(), 1);
    assert_eq!(exports.constants.len(), 1);
    assert_eq!(exports.functions.len(), 1);
}

#[test]
fn std_public_paths_keep_std_prefix_and_exact_payload_symbols() {
    let exports = projected_exports(
        SKIFF_STD_PUBLICATION_ID,
        "actor.Actor",
        "actor.defaultActor",
        "actor.run",
        actor_file_ir(),
    )
    .expect("fixture exports must project");

    assert_eq!(
        exports
            .types
            .get("std.actor.Actor")
            .expect("std type export at its package-scoped public path")
            .symbol,
        "Actor"
    );
    assert_eq!(
        exports
            .constants
            .get("std.actor.defaultActor")
            .expect("std const export at its package-scoped public path")
            .symbol,
        "DEFAULT_ACTOR"
    );
    assert_eq!(
        exports
            .functions
            .get("std.actor.run")
            .expect("std function export at its package-scoped public path")
            .symbol,
        "run"
    );
}

#[test]
fn invalid_file_index_and_symbol_targets_fail_closed() {
    let missing_file = projected_exports_with_source_module(
        "example.com/actors",
        "missing",
        "Actor",
        actor_file_ir(),
    )
    .expect_err("a missing source module must fail closed");
    assert_error_contains(missing_file, "missing module missing");

    let mut bad_index_file = actor_file_ir();
    bad_index_file
        .link_targets
        .types
        .get_mut("Actor")
        .expect("type target")
        .type_index = 9;
    let bad_index = projected_exports(
        "example.com/actors",
        "api.ActorAlias",
        "api.defaultActorAlias",
        "api.runAlias",
        bad_index_file,
    )
    .expect_err("an out-of-bounds declaration index must fail closed");
    assert_error_contains(bad_index, "type export index 9 is out of bounds");

    let missing_symbol = projected_exports_with_source_module(
        "example.com/actors",
        "actor",
        "ActorSuffix",
        actor_file_ir(),
    )
    .expect_err("a suffix-only declaration match must fail closed");
    assert_error_contains(missing_symbol, "missing symbol ActorSuffix in module actor");
}

#[test]
fn public_instance_projection_keeps_only_typed_execution_links() {
    let links = projected_public_instance(
        public_instance_file_ir(),
        vec![public_instance_method("handle")],
    )
    .expect("public instance execution link must project");

    assert!(links.exports.public_instances.is_empty());
    assert!(links.exports.impl_methods.contains_key("Worker.handle"));
    let instance = &links.public_instances[0];
    assert_eq!(instance.public_path, "worker");
    assert_eq!(instance.receiver.const_index, 0);
    assert_eq!(instance.methods[0].name, "handle");
    assert_eq!(instance.methods[0].public_path, "worker.handle");
    assert_eq!(instance.methods[0].executable.executable_index, 0);
}

#[test]
fn public_instance_missing_method_and_index_fail_closed() {
    let mut missing_method = public_instance_file_ir();
    missing_method
        .link_targets
        .executables
        .remove("Worker.handle");
    missing_method
        .declarations
        .executables
        .remove("Worker.handle");
    let error = projected_public_instance(missing_method, vec![public_instance_method("handle")])
        .expect_err("missing implementation method must fail closed");
    assert_error_contains(error, "missing implementation method handle");

    let mut missing_index = public_instance_file_ir();
    missing_index
        .link_targets
        .executables
        .get_mut("Worker.handle")
        .expect("method target")
        .executable_index = 9;
    let error = projected_public_instance(missing_index, vec![public_instance_method("handle")])
        .expect_err("missing executable index must fail closed");
    assert_error_contains(error, "points to missing executable index 9");
}

#[test]
fn duplicate_source_validated_interface_method_fails_closed() {
    let method = public_instance_method("handle");
    let error = projected_public_instance(public_instance_file_ir(), vec![method.clone(), method])
        .expect_err("duplicate validated interface method must fail closed");
    assert_error_contains(error, "contains duplicate validated method handle");
}

fn assert_error_contains(error: crate::error::ProjectionError, expected: &str) {
    let actual = error.to_string();
    assert!(
        actual.contains(expected),
        "expected {expected:?} in {actual:?}"
    );
}
