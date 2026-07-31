use super::*;

#[test]
fn builtin_receiver_return_types_come_from_artifact_table_for_date_integer_ops() {
    for method in ["toEpochMilliseconds", "diffMilliseconds", "compare"] {
        assert_eq!(
            builtin_receiver_call_return_type_for_root("Date", &TypeRefIr::builtin("Date"), method,),
            Some(TypeRefIr::builtin("integer")),
            "Date.{method} should infer integer from artifact-model table"
        );
    }
    assert_eq!(
        builtin_receiver_call_return_type_for_root(
            "Duration",
            &TypeRefIr::builtin("std.time.Duration"),
            "toMilliseconds"
        ),
        Some(TypeRefIr::builtin("integer"))
    );
}

#[test]
fn qualified_std_duration_package_symbol_is_a_builtin_receiver_root() {
    let duration = TypeRefIr::PackageSymbol {
        symbol: crate::file_ir::PackageSymbolRef {
            package: crate::file_ir::PackageRefIr::PackageId {
                package_id: skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID.to_string(),
            },
            symbol_path: "std.time.Duration".to_string(),
            abi_expectation: None,
        },
    };

    assert_eq!(
        runtime_receiver_root_from_type_ref(&duration),
        Some("Duration".to_string())
    );
}
