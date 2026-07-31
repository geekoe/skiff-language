use super::file_ir_artifact_hash;
use skiff_artifact_identity::assign_file_ir_identity;
use skiff_artifact_model::FileIrUnit;

#[test]
fn artifact_hash_includes_source_ast_hash_fields() {
    let mut left = FileIrUnit::empty("surface", "source-ast-hash-a");
    assign_file_ir_identity(&mut left).expect("left identity");
    let mut right = FileIrUnit::empty("surface", "source-ast-hash-b");
    assign_file_ir_identity(&mut right).expect("right identity");

    assert_eq!(left.file_ir_identity, right.file_ir_identity);
    assert_ne!(file_ir_artifact_hash(&left), file_ir_artifact_hash(&right));
}
