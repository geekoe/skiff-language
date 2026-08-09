use std::collections::BTreeMap;

use skiff_artifact_model::{
    ConstDeclarationIr, ConstIr, ConstLinkTargetIr, ExecutableBody, ExecutableDeclarationIr,
    ExecutableIr, ExecutableKind, ExecutableLinkTargetIr, FileIrUnit, FunctionTypeParamIr,
    InterfaceDeclIr, InterfaceOperationIr, PackageExportIndex, ParamIr, ParamModeIr, SlotLayout, TypeDeclIr,
    TypeDeclarationIr, TypeDescriptorIr, TypeLinkTargetIr, TypeRefIr,
};

use super::super::project_package_export_links;
use crate::{
    error::ProjectionError,
    package_artifact::{
        api_exports::{
            PackageExportPublicInstance, PackageExportPublicInstanceInterface,
            PackageExportPublicInstanceMethod, PackageExportSymbol, PackageExports,
        },
        export_links::ProjectedPackageExportLinks,
        model::PackageExportLinkProjectionInput,
    },
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

pub(super) fn projected_public_instance(
    file: FileIrUnit,
    methods: Vec<PackageExportPublicInstanceMethod>,
) -> Result<ProjectedPackageExportLinks, ProjectionError> {
    let exports = PackageExports {
        entries: Vec::new(),
        symbols: BTreeMap::new(),
        public_instances: vec![PackageExportPublicInstance {
            public_path: "worker".to_string(),
            module: "api".to_string(),
            const_symbol: "worker".to_string(),
            receiver_module: "api".to_string(),
            receiver_symbol: "Worker".to_string(),
            interfaces: vec![PackageExportPublicInstanceInterface {
                module: "api".to_string(),
                symbol: "WorkerApi".to_string(),
                arguments: Vec::new(),
                methods,
            }],
        }],
    };
    let files = vec![file];
    project_package_export_links(
        &PackageExportLinkProjectionInput {
            package_id: "example.com/worker",
            exports: &exports,
            file_ir_units: &files,
        },
        &[],
    )
}

pub(super) fn public_instance_method(name: &str) -> PackageExportPublicInstanceMethod {
    PackageExportPublicInstanceMethod {
        name: name.to_string(),
        executable_module: "api".to_string(),
        executable_symbol: format!("Worker.{name}"),
    }
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
    let file_ir_units = vec![file];
    let package = PackageExportLinkProjectionInput {
        package_id,
        exports,
        file_ir_units: &file_ir_units,
    };

    project_package_export_links(&package, &[]).map(|links| links.exports)
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
        implements: Vec::new(),
        source_span: None,
    });
    file.constants.push(ConstIr {
        name: "DEFAULT_ACTOR".to_string(),
        ty: TypeRefIr::builtin("string"),
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
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

pub(super) fn public_instance_file_ir() -> FileIrUnit {
    let mut file = FileIrUnit::empty("api", "source-hash");
    file.file_ir_identity = "file-ir:api".to_string();
    file.type_table.extend([
        TypeDeclIr {
            name: "Worker".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "WorkerApi".to_string(),
            descriptor: TypeDescriptorIr::Interface,
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ]);
    file.declarations.types.extend([
        (
            "Worker".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "api.Worker".to_string(),
                source_span: None,
            },
        ),
        (
            "WorkerApi".to_string(),
            TypeDeclarationIr {
                type_index: 1,
                symbol: "api.WorkerApi".to_string(),
                source_span: None,
            },
        ),
    ]);
    file.declarations.interfaces.insert(
        "WorkerApi".to_string(),
        InterfaceDeclIr {
            name: "WorkerApi".to_string(),
            type_params: Vec::new(),
            operations: vec![InterfaceOperationIr {
                name: "handle".to_string(),
                type_params: Vec::new(),
                params: vec![FunctionTypeParamIr {
                    name: "value".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
                return_type: TypeRefIr::builtin("string"),
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: Some(TypeRefIr::builtin("Self")),
            }],
            source_span: None,
        },
    );
    let worker_type = TypeRefIr::LocalType { type_index: 0 };
    file.constants.push(ConstIr {
        name: "worker".to_string(),
        ty: worker_type.clone(),
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.declarations.constants.insert(
        "worker".to_string(),
        ConstDeclarationIr {
            const_index: 0,
            symbol: "api.worker".to_string(),
            ty: worker_type.clone(),
            source_span: None,
        },
    );
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::ImplMethod,
        symbol: "api.Worker.handle".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 1,
            ty: TypeRefIr::builtin("string"),
            mode: ParamModeIr::Value,
        }],
        return_type: TypeRefIr::builtin("string"),
        self_type: Some(worker_type),
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    });
    file.declarations.executables.insert(
        "Worker.handle".to_string(),
        ExecutableDeclarationIr {
            executable_index: 0,
            symbol: "api.Worker.handle".to_string(),
            source_span: None,
        },
    );
    file.link_targets.executables.insert(
        "Worker.handle".to_string(),
        ExecutableLinkTargetIr {
            executable_index: 0,
        },
    );
    file
}
