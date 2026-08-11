use super::{canonical_file_ir_identity_value, file_ir_identity};
use crate::file_ir::{
    ConstIr, ExecutableBody, FileIrUnit, SourceMapSource, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
};

#[test]
fn identity_payload_omits_excluded_fields_by_type() {
    let mut unit = FileIrUnit::empty("internal.example", "source-ast-hash-a");
    unit.file_ir_identity = "stale-file-ir-identity".to_string();
    unit.source_map.sources.push(SourceMapSource {
        id: 0,
        path: "internal/example.skiff".to_string(),
        module_path: "internal.example".to_string(),
        source_ast_hash: Some("source-map-ast-hash-a".to_string()),
    });

    let value = canonical_file_ir_identity_value(&unit);

    assert!(value.get("fileIrIdentity").is_none());
    assert!(value.get("sourceAstHash").is_none());
    assert!(value
        .pointer("/sourceMap/sources/0/sourceAstHash")
        .is_none());
    assert_eq!(value["modulePath"], "internal.example");
    assert_eq!(
        value.pointer("/sourceMap/sources/0/path"),
        Some(&serde_json::json!("internal/example.skiff"))
    );
}

#[test]
fn nontrivial_file_ir_identity_matches_canonical_owner_and_golden() {
    let mut unit = FileIrUnit::empty("internal.identity_golden", "source-ast-hash");
    unit.source_map.sources.push(SourceMapSource {
        id: 7,
        path: "internal/identity_golden.skiff".to_string(),
        module_path: "internal.identity_golden".to_string(),
        source_ast_hash: Some("excluded-source-map-hash".to_string()),
    });
    unit.type_table.push(TypeDeclIr {
        name: "Payload".to_string(),
        descriptor: TypeDescriptorIr::Alias {
            target: TypeRefIr::builtin("string"),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    unit.constants.push(ConstIr {
        name: "greeting".to_string(),
        ty: TypeRefIr::builtin("string"),
        body: ExecutableBody::default(),
        source_span: None,
    });

    let adapter_identity = file_ir_identity(&unit);
    let canonical_identity =
        skiff_artifact_identity::file_ir_identity(&unit).expect("canonical File IR identity");

    assert_eq!(adapter_identity, canonical_identity);
    assert_eq!(
        adapter_identity,
        "skiff-file-ir-v14:sha256:c269c1a8f9540f4be4a87e4a3d4ece1038e264a99de29984c15562ae50e78898"
    );
}
