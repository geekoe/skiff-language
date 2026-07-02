use std::{collections::BTreeMap, sync::Arc};

use serde_json::json;

use super::*;
use crate::eval::program::{
    anonymous_type_decl, FileDeclarations, FileLinkTargets, LinkOverlay, PackageUnit,
    RuntimeTypeContext,
};
use crate::type_descriptor::ProgramTypeView;

#[test]
fn module_specific_decode_targets_are_catchable() {
    for (target, catch_type) in [
        ("std.json.encode", "std.json.DecodeError"),
        ("config.require", "config.DecodeError"),
        ("number.parse", "std.number.DecodeError"),
        ("Date.requireParse", "std.time.DecodeError"),
    ] {
        let envelope = exception_envelope_for_catch(
            &RuntimeError::decode_target(target, "decode failed"),
            &[TypeIdentity::builtin(catch_type)],
        )
        .expect("native decode target should translate")
        .unwrap_or_else(|| panic!("{catch_type} should catch {target}"));

        assert_eq!(
            envelope["__skiffActualPayloadType"],
            json!({
                "kind": "builtin",
                "name": catch_type
            })
        );
        assert_eq!(envelope["error"]["target"], target);
    }
}

#[test]
fn linked_std_error_address_catches_native_std_error_identity() {
    let (program, addr) = program_with_std_error_type("std.json", "DecodeError");
    let leaves = catch_type_leaves(
        &LinkedTypeRef::Address { addr: addr.clone() },
        program.view(),
    )
    .expect("std error address catch type should collect leaves");

    assert!(leaves.contains(&TypeIdentity::builtin("std.json.DecodeError")));
    assert!(leaves.contains(&TypeIdentity::address(addr.clone())));
    assert_eq!(
        throw_payload_actual_type(&LinkedTypeRef::Address { addr }, program.view())
            .expect("std error throw payload type should resolve"),
        TypeIdentity::builtin("std.json.DecodeError")
    );

    let envelope = exception_envelope_for_catch(
        &RuntimeError::decode_target("std.json.decode", "decode failed"),
        &leaves,
    )
    .expect("native std json decode should translate")
    .expect("linked std json catch type should match native std json decode");

    assert_eq!(
        envelope["__skiffActualPayloadType"],
        json!({
            "kind": "builtin",
            "name": "std.json.DecodeError"
        })
    );
}

struct TestProgramTypeView {
    service_files: Vec<Arc<LinkedFileUnit>>,
    packages: Vec<Arc<PackageUnit>>,
    package_files: Vec<Vec<Arc<LinkedFileUnit>>>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
}

impl TestProgramTypeView {
    fn view(&self) -> ProgramTypeView<'_> {
        ProgramTypeView::new(
            &self.service_files,
            &self.packages,
            &self.package_files,
            &self.link_overlay,
            &self.types,
        )
    }
}

fn program_with_std_error_type(
    module_path: &str,
    type_name: &str,
) -> (TestProgramTypeView, TypeAddr) {
    let addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let type_decl = anonymous_type_decl(
        type_name,
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    let mut types = RuntimeTypeContext::default();
    types.descriptors.insert(addr.clone(), type_decl.clone());
    let file = LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: format!("file:{module_path}"),
        source_ast_hash: format!("source:{module_path}"),
        module_path: module_path.to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        types: vec![type_decl],
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: Default::default(),
    };

    (
        TestProgramTypeView {
            service_files: Vec::new(),
            packages: vec![Arc::new(PackageUnit::empty(
                "skiff.run/std",
                "1.0.0",
                "build:std",
                "abi:std",
            ))],
            package_files: vec![vec![Arc::new(file)]],
            link_overlay: LinkOverlay::default(),
            types,
        },
        addr,
    )
}
