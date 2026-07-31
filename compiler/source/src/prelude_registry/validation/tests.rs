use std::collections::BTreeMap;

use super::{validate_root_projection_metadata, validate_type_name};

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
fn compiler_builtin_arity_and_unknown_types_fail_closed() {
    let known = vec![
        "Array".to_string(),
        "string".to_string(),
        "number".to_string(),
    ];
    assert!(validate_type_name("Array<string>", &known, &[], "std.test").is_ok());
    assert_eq!(
        validate_type_name("Array", &known, &[], "std.test").unwrap_err(),
        "compiler builtin type Array expects 1 type arguments, got 0 in module std.test"
    );
    assert_eq!(
        validate_type_name("Array<string, number>", &known, &[], "std.test").unwrap_err(),
        "compiler builtin type Array expects 1 type arguments, got 2 in module std.test"
    );
    assert_eq!(
        validate_type_name("Missing", &known, &[], "std.test").unwrap_err(),
        "unknown standard_library type reference Missing in module std.test"
    );
}
