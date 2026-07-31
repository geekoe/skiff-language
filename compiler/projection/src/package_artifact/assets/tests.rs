use std::path::PathBuf;

use skiff_artifact_model::FileIrUnit;

use super::{file_ir_refs_from_units, resource_refs_from_projected, ProjectedPackageResource};

#[test]
fn artifact_asset_refs_are_normalized_independently_of_input_order() {
    let mut later_file = FileIrUnit::empty("later", "later-source");
    later_file.file_ir_identity = "file-b".to_string();
    let mut earlier_file = FileIrUnit::empty("earlier", "earlier-source");
    earlier_file.file_ir_identity = "file-a".to_string();
    let files = file_ir_refs_from_units(&[later_file, earlier_file]);
    assert_eq!(files[0].file_ir_identity, "file-a");
    assert_eq!(files[1].file_ir_identity, "file-b");

    let resource = |path: &str| ProjectedPackageResource {
        path: path.to_string(),
        absolute_path: PathBuf::from(path),
        byte_len: 0,
        sha256: String::new(),
        content_type: None,
    };
    let resources = resource_refs_from_projected(&[resource("z"), resource("a")]);
    assert_eq!(resources[0].path, "a");
    assert_eq!(resources[1].path, "z");
}
