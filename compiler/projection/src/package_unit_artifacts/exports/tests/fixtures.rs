use std::collections::BTreeMap;

use skiff_artifact_model::{
    AbiIdentityFacts, ConfigShape, ConstIr, ConstLinkTargetIr, ExecutableBody, ExecutableIr,
    ExecutableKind, ExecutableLinkTargetIr, FileIrUnit, PackageExportIndex, SlotLayout, TypeDeclIr,
    TypeDescriptorIr, TypeLinkTargetIr, TypeRefIr,
};
use skiff_compiler_projection_input::ProjectionCallableEffectFacts;

use super::super::project_package_export_index;
use crate::{
    error::ProjectionError,
    package_exports::{PackageExportSymbol, PackageExports},
    package_unit_artifacts::{PackageFileIrProjection, PackageIrProjectionSource},
    ConfigActivation, ConfigProjection, ConfigRequirementsProjection,
};

pub(super) fn projected_exports(
    package_id: &str,
    type_public_path: &str,
    const_public_path: &str,
    function_public_path: &str,
    file: FileIrUnit,
) -> Result<PackageExportIndex, ProjectionError> {
    let exports = PackageExports {
        entries: Vec::new(),
        symbols: BTreeMap::from([
            export(type_public_path, "Actor"),
            export(const_public_path, "DEFAULT_ACTOR"),
            export(function_public_path, "run"),
        ]),
        public_instances: Vec::new(),
    };
    project_exports(package_id, &exports, file)
}

pub(super) fn projected_exports_with_source_module(
    package_id: &str,
    module: &str,
    symbol: &str,
    file: FileIrUnit,
) -> Result<PackageExportIndex, ProjectionError> {
    let exports = PackageExports {
        entries: Vec::new(),
        symbols: BTreeMap::from([(
            "api.ActorAlias".to_string(),
            PackageExportSymbol {
                module: module.to_string(),
                symbol: symbol.to_string(),
            },
        )]),
        public_instances: Vec::new(),
    };
    project_exports(package_id, &exports, file)
}

fn export(public_path: &str, declaration_symbol: &str) -> (String, PackageExportSymbol) {
    (
        public_path.to_string(),
        PackageExportSymbol {
            module: "actor".to_string(),
            symbol: declaration_symbol.to_string(),
        },
    )
}

fn project_exports(
    package_id: &str,
    exports: &PackageExports,
    file: FileIrUnit,
) -> Result<PackageExportIndex, ProjectionError> {
    let abi_identity_projection = AbiIdentityFacts::default();
    let config_projection = empty_config_projection();
    let callable_effects = ProjectionCallableEffectFacts::default();
    let package = PackageIrProjectionSource {
        package_id,
        version: "0.1.0",
        exports,
        abi_identity_projection: &abi_identity_projection,
        config_projection: &config_projection,
        callable_effects: &callable_effects,
        resources: &[],
        file_ir_units: vec![PackageFileIrProjection::from_unit(file)],
    };

    project_package_export_index(&package, &[])
}

pub(super) fn actor_file_ir() -> FileIrUnit {
    let mut file = FileIrUnit::empty("actor", "source-hash");
    file.file_ir_identity = "file-ir:actor".to_string();
    file.type_table.push(TypeDeclIr {
        name: "Actor".to_string(),
        descriptor: TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        },
        type_params: Vec::new(),
        discriminator: None,
        implements: Vec::new(),
        source_span: None,
    });
    file.constants.push(ConstIr {
        name: "DEFAULT_ACTOR".to_string(),
        ty: TypeRefIr::native("string"),
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::native("void"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.link_targets
        .types
        .insert("Actor".to_string(), TypeLinkTargetIr { type_index: 0 });
    file.link_targets.constants.insert(
        "DEFAULT_ACTOR".to_string(),
        ConstLinkTargetIr { const_index: 0 },
    );
    file.link_targets.executables.insert(
        "run".to_string(),
        ExecutableLinkTargetIr {
            executable_index: 0,
        },
    );
    file
}

fn empty_config_projection() -> ConfigProjection {
    ConfigProjection {
        shape: ConfigShape::empty(),
        uses: Vec::new(),
        activation: ConfigActivation {
            schema_version: "test-config-activation",
            has_paths: Vec::new(),
        },
        requirements: ConfigRequirementsProjection {
            own: Vec::new(),
            dependency: Vec::new(),
            effective: Vec::new(),
        },
    }
}
