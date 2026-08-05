use std::{
    collections::{BTreeMap, HashMap},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde_json::{json, Value};

use skiff_runtime_boundary::date_value;

use skiff_runtime_boundary::json::RuntimeBoundaryCodec;

use skiff_runtime_boundary::plan::BoundaryUse;

use skiff_runtime_boundary::stream::STREAM_ID_KEY;

use skiff_runtime_boundary::type_descriptor::{
    RuntimeTypeNode, RuntimeTypePlan, RuntimeTypePlanDescriptorExt,
};

use skiff_runtime_boundary::{
    binary::{decode_payload, encode_payload, encode_payload_plan},
    payload::PayloadBoundary,
};

use skiff_runtime_host::eval_capability_adapter;

use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{
        HeapHandle, HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier,
    },
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, PlatformBuiltinErrorIdentity, RequestException,
    },
};

use skiff_runtime_request::cancellation::CancellationToken;

use tokio::time::sleep;

use super::super::*;
use super::*;

use crate::eval::InterpreterEnv as Env;

use skiff_artifact_model::{
    builtin_receiver_op_by_name, DbMetadataIr, FileIrRef, PackageArtifactRef, PackageBuildId,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PublicationResourceRef, TypeDescriptorIr,
    TypeExport,
};

use skiff_runtime_capability_context::{
    DbCapabilityTarget, DbCapabilityTargetId, DbProviderTargetMetadata,
};

use skiff_runtime_linked_program::{
    linked::{DbDeclarationIr, DbObjectKeyIr, DbObjectKindIr, TypeDeclarationIr},
    DbObjectTargetId, LinkedNamedUnionBranch, LoadedPublicationResource, PublicationResourceTable,
    RuntimeExecutionPackage,
};

use crate::{
    eval::error::{unwrap_diagnostic_source_context, RuntimeError},
    eval::exceptions::request_exception_for_rethrow,
    eval::program::{
        anonymous_type_decl, types::PackageSymbolKey, CallIr, ConstAddr, ConstIr, ExecutableAddr,
        ExecutableKind, ExprRefIr, FileAddr, FileDeclarations, FileLinkTargets, GatewayConfig,
        LinkOverlay, LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedExprIr,
        LinkedFileUnit, LinkedStmtIr, LinkedTypeDescriptor, LinkedTypeRef, LiteralIr,
        MetadataValue, NativeTarget, ParamIr, ResolvedSymbol, RuntimeProgram, RuntimeTypeContext,
        ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, StmtRefIr, TypeAddr, TypeDeclIr,
        UnitAddr,
    },
    eval::{
        capabilities::{StreamPoll, StreamRuntime, TypedStreamSink},
        native_capability::project_runtime_native_capability_context,
        native_invocation::resolve_runtime_native_invocation,
        program_execution::{
            executable_type_param_names, OwnedProgramExecutionContext, ProgramExecutionInput,
        },
        program_invocation::{ProgramInvocationContext, ProgramInvocationInput},
        TestEffectDouble,
    },
    type_descriptor::{PlanContext, RuntimeTypePlanLinkedExt},
};

use super::executables::*;
use super::runtime::*;
use super::stream_executables::*;
use skiff_runtime_native::dispatch::NativeDispatch;

pub(crate) const STD_HTTP_HEADER_TYPE_INDEX: usize = 0;
pub(crate) const STD_HTTP_QUERY_PARAM_TYPE_INDEX: usize = 1;
pub(crate) const STD_HTTP_REQUEST_TYPE_INDEX: usize = 2;
pub(crate) const STD_HTTP_RESPONSE_TYPE_INDEX: usize = 3;
pub(crate) const STD_HTTP_RESPONSE_STREAM_EVENT_TYPE_INDEX: usize = 4;
pub(crate) const STD_HTTP_CLIENT_REQUEST_TYPE_INDEX: usize = 5;
pub(crate) const STD_HTTP_CLIENT_RESPONSE_TYPE_INDEX: usize = 6;
pub(crate) const STD_HTTP_CLIENT_STREAM_HANDLE_TYPE_INDEX: usize = 7;
pub(crate) const STD_HTTP_SSE_EVENT_TYPE_INDEX: usize = 8;
pub(crate) const STD_DURATION_TYPE_INDEX: usize = 9;
pub(crate) const STD_FILE_IMMUTABLE_TYPE_INDEX: usize = 10;
pub(crate) const STD_FILE_CREATE_OPTIONS_TYPE_INDEX: usize = 11;
pub(crate) const STD_FILE_INFO_TYPE_INDEX: usize = 12;
pub(crate) const STD_RESOURCE_INFO_TYPE_INDEX: usize = 13;
pub(crate) const STD_RESOURCE_ERROR_TYPE_INDEX: usize = 14;

pub(crate) fn program_with_executables(executables: Vec<LinkedExecutable>) -> RuntimeProgram {
    let addr = ExecutableAddr::service(0, 0);
    RuntimeProgram {
        service: ServiceMeta {
            id: "svc".to_string(),
            display_name: Some("Service".to_string()),
            metadata: Default::default(),
        },
        version: "v1".to_string(),
        build_id: "build:program".to_string(),
        service_files: vec![Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:svc".to_string(),
            source_ast_hash: "source:svc".to_string(),
            module_path: "svc.main".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: Vec::new(),
            types: Vec::new(),
            constants: Vec::new(),
            executables,
            external_refs: Default::default(),
        })],
        packages: Vec::new(),
        service_resources: Default::default(),
        timeout: Default::default(),
        operation_route_bindings: Vec::new(),
        routes: HashMap::from([("svc.main.run".to_string(), addr.clone())]),
        task_routes: HashMap::new(),
        operations: HashMap::from([("run".to_string(), addr)]),
        operation_receivers: HashMap::new(),
        db: Vec::new(),
        actors: Vec::new(),
        link_overlay: LinkOverlay::default(),
        gateway: GatewayConfig::default(),
        types: RuntimeTypeContext::default(),
    }
}

pub(crate) fn program_with_executables_and_std_http_types(
    executables: Vec<LinkedExecutable>,
) -> RuntimeProgram {
    program_with_executables_and_std_builtins(executables)
}

pub(crate) fn program_with_executables_and_std_builtins(
    executables: Vec<LinkedExecutable>,
) -> RuntimeProgram {
    let mut program = program_with_executables(executables);
    install_std_builtin_package_types(&mut program);
    program
}

pub(crate) fn program_with_executable_and_std_http_types(
    executable: LinkedExecutable,
) -> RuntimeProgram {
    program_with_executables_and_std_http_types(vec![executable])
}

pub(crate) fn program_with_executable_and_std_builtins(
    executable: LinkedExecutable,
) -> RuntimeProgram {
    program_with_executables_and_std_builtins(vec![executable])
}

pub(crate) fn install_std_builtin_package_types(program: &mut RuntimeProgram) {
    let package_slot = program.packages.len();
    assert_eq!(
        package_slot, 0,
        "std HTTP test fixture currently expects an otherwise package-free program"
    );
    let declarations = std_builtin_type_declarations(package_slot);
    let std_file = Arc::new(std_builtin_file_unit(
        declarations
            .iter()
            .map(|(_, declaration)| declaration.clone())
            .collect(),
    ));
    let std_file_ref = FileIrRef {
        file_ir_identity: std_file.file_ir_identity.clone(),
        module_path: std_file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(std_file.source_ast_hash.clone()),
    };
    let resources = PublicationResourceTable::default();
    let mut std_package = crate::eval::test_support::runtime_execution_package_artifact_fixture(
        "skiff.run/std",
        "1.0.0",
        "skiff.run/std:build",
        "skiff.run/std:abi",
        &[Arc::clone(&std_file)],
        &resources,
    );
    std_package.package_local_abi.public_symbols.insert(
        "std.resource.ResourceError".to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: "type:std.resource.ResourceError".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            is_alias: false,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    std_package.implementation_links.types.insert(
        "std.resource.ResourceError".to_string(),
        TypeExport {
            file: std_file_ref,
            type_index: STD_RESOURCE_ERROR_TYPE_INDEX as u32,
            symbol: "ResourceError".to_string(),
            is_interface: false,
            descriptor: None,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    );
    program.packages.push(
        crate::eval::test_support::runtime_execution_package_from_artifact(
            package_slot,
            std_package,
            vec![Arc::clone(&std_file)],
            resources,
        ),
    );
    program
        .link_overlay
        .package_slots_by_id
        .insert("skiff.run/std".to_string(), package_slot);
    program
        .link_overlay
        .package_slots_by_dependency_ref
        .insert("std".to_string(), package_slot);
    for (index, (symbol_path, declaration)) in declarations.into_iter().enumerate() {
        let addr = std_http_type_addr_for_package(package_slot, index);
        program.types.descriptors.insert(addr.clone(), declaration);
        program.types.exported_types.insert_package(
            PackageSymbolKey::new(package_slot, symbol_path),
            addr.clone(),
        );
        if let Some(short_path) = symbol_path.strip_prefix("std.") {
            program
                .types
                .exported_types
                .insert_package(PackageSymbolKey::new(package_slot, short_path), addr);
        }
    }
    program.link_overlay.symbols.insert_package(
        PackageSymbolKey::new(package_slot, "std.resource.ResourceError"),
        ResolvedSymbol::Type {
            addr: std_http_type_addr_for_package(package_slot, STD_RESOURCE_ERROR_TYPE_INDEX),
        },
    );
}

pub(crate) fn replace_std_resource_error_type(
    program: &mut RuntimeProgram,
    declaration: TypeDeclIr,
) {
    let addr = std_http_type_addr(STD_RESOURCE_ERROR_TYPE_INDEX);
    let package = program.packages.first().expect("std package test fixture");
    let mut files = package.files().to_vec();
    let file = Arc::make_mut(
        files
            .first_mut()
            .expect("std package test fixture should have one file"),
    );
    file.types[STD_RESOURCE_ERROR_TYPE_INDEX] = declaration.clone();
    let artifact = package.artifact().clone();
    let resources = package.static_resources().clone();
    program.packages[0] = crate::eval::test_support::runtime_execution_package_from_artifact(
        0, artifact, files, resources,
    );
    program.types.descriptors.insert(addr, declaration);
}

pub(crate) fn program_with_executables_and_local_error_type(
    executables: Vec<LinkedExecutable>,
    error_type_name: &str,
) -> RuntimeProgram {
    let mut program = program_with_executables(executables);
    let file = Arc::make_mut(
        program
            .service_files
            .get_mut(0)
            .expect("test program should have a service file"),
    );
    file.types.push(crate::eval::program::TypeDeclIr {
        name: error_type_name.to_string(),
        descriptor: LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([(
                "message".to_string(),
                LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            )]),
        },
        ..crate::eval::program::TypeDeclIr::default()
    });
    program.types.descriptors.insert(
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        },
        file.types[0].clone(),
    );
    program
}

pub(crate) fn program_with_two_same_named_error_types(
    executables: Vec<LinkedExecutable>,
) -> RuntimeProgram {
    let mut program = program_with_executables(executables);
    let file = Arc::make_mut(
        program
            .service_files
            .get_mut(0)
            .expect("test program should have a service file"),
    );
    for _ in 0..2 {
        file.types.push(crate::eval::program::TypeDeclIr {
            name: "AuthError".to_string(),
            descriptor: local_error_descriptor(),
            ..crate::eval::program::TypeDeclIr::default()
        });
    }
    for type_index in 0..2 {
        program.types.descriptors.insert(
            service_type_addr(type_index),
            file.types[type_index].clone(),
        );
    }
    program
}

pub(crate) fn local_error_descriptor() -> LinkedTypeDescriptor {
    LinkedTypeDescriptor::Record {
        fields: BTreeMap::from([(
            "message".to_string(),
            LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        )]),
    }
}

pub(crate) fn service_type_addr(type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

pub(crate) fn std_http_type_addr(type_index: usize) -> TypeAddr {
    std_http_type_addr_for_package(0, type_index)
}

pub(crate) fn std_http_type_addr_for_package(package_slot: usize, type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Package(package_slot),
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

pub(crate) fn std_http_type_ref(type_index: usize) -> LinkedTypeRef {
    LinkedTypeRef::Address {
        addr: std_http_type_addr(type_index),
    }
}

pub(crate) fn std_http_type_plan_for_test(
    program: &RuntimeProgram,
    addr: &ExecutableAddr,
    type_index: usize,
) -> RuntimeTypePlan {
    let image = program.linked_image();
    RuntimeTypePlan::from_linked(
        &std_http_type_ref(type_index),
        &PlanContext::new(&image, addr),
    )
    .expect("std HTTP fixture type plan should build")
}

pub(crate) fn std_builtin_file_unit(types: Vec<TypeDeclIr>) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:std-http".to_string(),
        source_ast_hash: "source:std-http".to_string(),
        module_path: "std.http".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types,
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: Default::default(),
    }
}

pub(crate) fn std_builtin_type_declarations(
    package_slot: usize,
) -> Vec<(&'static str, TypeDeclIr)> {
    let header = LinkedTypeRef::Address {
        addr: std_http_type_addr_for_package(package_slot, STD_HTTP_HEADER_TYPE_INDEX),
    };
    let query_param = LinkedTypeRef::Address {
        addr: std_http_type_addr_for_package(package_slot, STD_HTTP_QUERY_PARAM_TYPE_INDEX),
    };
    let mut declarations = vec![
        (
            "std.http.HttpHeader",
            anonymous_type_decl(
                "std.http.HttpHeader",
                linked_record_descriptor(vec![
                    ("name", linked_builtin_type("string")),
                    ("value", linked_builtin_type("string")),
                ]),
            ),
        ),
        (
            "std.http.HttpQueryParam",
            anonymous_type_decl(
                "std.http.HttpQueryParam",
                linked_record_descriptor(vec![
                    ("name", linked_builtin_type("string")),
                    ("value", linked_builtin_type("string")),
                ]),
            ),
        ),
        (
            "std.http.HttpRequest",
            anonymous_type_decl(
                "std.http.HttpRequest",
                linked_record_descriptor(vec![
                    ("method", linked_builtin_type("string")),
                    ("url", linked_builtin_type("string")),
                    ("path", linked_builtin_type("string")),
                    ("query", linked_array_type(query_param.clone())),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_builtin_type("bytes")),
                ]),
            ),
        ),
        (
            "std.http.HttpResponse",
            anonymous_type_decl(
                "std.http.HttpResponse",
                linked_record_descriptor(vec![
                    ("status", linked_builtin_type("integer")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_builtin_type("bytes")),
                ]),
            ),
        ),
        (
            "std.http.HttpResponseStreamEvent",
            anonymous_type_decl(
                "std.http.HttpResponseStreamEvent",
                LinkedTypeDescriptor::Union {
                    branches: vec![
                        linked_discriminated_union_branch(
                            "tag",
                            "start",
                            linked_record_type(vec![
                                ("tag", linked_literal_string("start")),
                                ("status", linked_builtin_type("integer")),
                                ("headers", linked_array_type(header.clone())),
                            ]),
                        ),
                        linked_discriminated_union_branch(
                            "tag",
                            "chunk",
                            linked_record_type(vec![
                                ("tag", linked_literal_string("chunk")),
                                ("value", linked_builtin_type("bytes")),
                            ]),
                        ),
                        linked_discriminated_union_branch(
                            "tag",
                            "end",
                            linked_record_type(vec![("tag", linked_literal_string("end"))]),
                        ),
                    ],
                },
            ),
        ),
        (
            "std.http.HttpClientRequest",
            anonymous_type_decl(
                "std.http.HttpClientRequest",
                linked_record_descriptor(vec![
                    ("method", linked_builtin_type("string")),
                    ("url", linked_builtin_type("string")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_nullable_type(linked_builtin_type("bytes"))),
                    (
                        "timeoutMs",
                        linked_nullable_type(linked_builtin_type("integer")),
                    ),
                ]),
            ),
        ),
        (
            "std.http.HttpClientResponse",
            anonymous_type_decl(
                "std.http.HttpClientResponse",
                linked_record_descriptor(vec![
                    ("status", linked_builtin_type("integer")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_builtin_type("bytes")),
                ]),
            ),
        ),
        (
            "std.http.HttpClientStreamHandle",
            anonymous_type_decl(
                "std.http.HttpClientStreamHandle",
                linked_record_descriptor(vec![
                    ("status", linked_builtin_type("integer")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_stream_type(linked_builtin_type("bytes"))),
                ]),
            ),
        ),
        (
            "std.http.HttpSseEvent",
            anonymous_type_decl(
                "std.http.HttpSseEvent",
                LinkedTypeDescriptor::Union {
                    branches: vec![
                        linked_discriminated_union_branch(
                            "tag",
                            "response",
                            linked_record_type(vec![
                                ("tag", linked_literal_string("response")),
                                ("status", linked_builtin_type("integer")),
                                ("headers", linked_array_type(header)),
                            ]),
                        ),
                        linked_discriminated_union_branch(
                            "tag",
                            "body",
                            linked_record_type(vec![
                                ("tag", linked_literal_string("body")),
                                ("value", linked_builtin_type("bytes")),
                            ]),
                        ),
                        linked_discriminated_union_branch(
                            "tag",
                            "event",
                            linked_record_type(vec![
                                ("tag", linked_literal_string("event")),
                                ("event", linked_nullable_type(linked_builtin_type("string"))),
                                ("id", linked_nullable_type(linked_builtin_type("string"))),
                                ("data", linked_builtin_type("string")),
                            ]),
                        ),
                    ],
                },
            ),
        ),
    ];
    declarations.extend([
        (
            "std.time.Duration",
            anonymous_type_decl(
                "std.time.Duration",
                LinkedTypeDescriptor::Alias {
                    target: linked_builtin_type("integer"),
                },
            ),
        ),
        (
            "std.file.ImmutableFile",
            anonymous_type_decl(
                "std.file.ImmutableFile",
                linked_record_descriptor(vec![
                    ("id", linked_builtin_type("string")),
                    ("size", linked_builtin_type("integer")),
                    ("sha256", linked_builtin_type("string")),
                    (
                        "contentType",
                        LinkedTypeRef::Nullable {
                            inner: Box::new(linked_builtin_type("string")),
                        },
                    ),
                ]),
            ),
        ),
        (
            "std.file.CreateOptions",
            anonymous_type_decl(
                "std.file.CreateOptions",
                linked_record_descriptor(vec![
                    (
                        "contentType",
                        LinkedTypeRef::Nullable {
                            inner: Box::new(linked_builtin_type("string")),
                        },
                    ),
                    (
                        "purpose",
                        LinkedTypeRef::Nullable {
                            inner: Box::new(linked_builtin_type("string")),
                        },
                    ),
                ]),
            ),
        ),
        (
            "std.file.FileInfo",
            anonymous_type_decl(
                "std.file.FileInfo",
                linked_record_descriptor(vec![
                    ("id", linked_builtin_type("string")),
                    ("size", linked_builtin_type("integer")),
                    ("sha256", linked_builtin_type("string")),
                    (
                        "contentType",
                        LinkedTypeRef::Nullable {
                            inner: Box::new(linked_builtin_type("string")),
                        },
                    ),
                    (
                        "purpose",
                        LinkedTypeRef::Nullable {
                            inner: Box::new(linked_builtin_type("string")),
                        },
                    ),
                    ("createdAt", linked_builtin_type("string")),
                ]),
            ),
        ),
        (
            "std.resource.ResourceInfo",
            anonymous_type_decl(
                "std.resource.ResourceInfo",
                linked_record_descriptor(vec![
                    ("path", linked_builtin_type("string")),
                    ("size", linked_builtin_type("integer")),
                    ("sha256", linked_builtin_type("string")),
                    (
                        "contentType",
                        LinkedTypeRef::Nullable {
                            inner: Box::new(linked_builtin_type("string")),
                        },
                    ),
                ]),
            ),
        ),
        (
            "std.resource.ResourceError",
            anonymous_type_decl(
                "ResourceError",
                linked_record_descriptor(vec![
                    ("path", linked_builtin_type("string")),
                    ("message", linked_builtin_type("string")),
                ]),
            ),
        ),
    ]);
    declarations
}

pub(crate) fn linked_record_descriptor(fields: Vec<(&str, LinkedTypeRef)>) -> LinkedTypeDescriptor {
    LinkedTypeDescriptor::Record {
        fields: linked_field_map(fields),
    }
}

pub(crate) fn linked_record_type(fields: Vec<(&str, LinkedTypeRef)>) -> LinkedTypeRef {
    LinkedTypeRef::Record {
        fields: linked_field_map(fields),
    }
}

pub(crate) fn linked_discriminated_union_branch(
    discriminator_field: &str,
    discriminator_value: &str,
    payload_type: LinkedTypeRef,
) -> LinkedNamedUnionBranch {
    LinkedNamedUnionBranch::SyntheticDiscriminator {
        payload_type,
        discriminator_field: discriminator_field.to_string(),
        discriminator_value: discriminator_value.to_string(),
    }
}

pub(crate) fn linked_field_map(
    fields: Vec<(&str, LinkedTypeRef)>,
) -> BTreeMap<String, LinkedTypeRef> {
    fields
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty))
        .collect()
}

pub(crate) fn linked_array_type(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Array".to_string(),
        args: vec![item],
    }
}

pub(crate) fn linked_nullable_type(inner: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Nullable {
        inner: Box::new(inner),
    }
}

pub(crate) fn linked_stream_type(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Stream".to_string(),
        args: vec![item],
    }
}

pub(crate) fn linked_literal_string(value: &str) -> LinkedTypeRef {
    LinkedTypeRef::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

pub(crate) fn runtime_error_leaf(error: &RuntimeError) -> &RuntimeError {
    unwrap_diagnostic_source_context(error)
}

pub(crate) fn assert_unsupported_foreground_wait_error(error: &RuntimeError) {
    assert!(
        error
            .to_string()
            .contains("foreground/activate wait until parking is unsupported in this runtime path"),
        "unexpected error: {error}"
    );
}

pub(crate) fn program_with_service_and_package_executables(
    service_executable: LinkedExecutable,
    package_executable: LinkedExecutable,
) -> RuntimeProgram {
    let mut program = program_with_executable(service_executable);
    let linked_file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:pkg".to_string(),
        source_ast_hash: "source:pkg".to_string(),
        module_path: "pkg.main".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![package_executable],
        external_refs: Default::default(),
    });
    program.packages = vec![runtime_package(
        "skiff.test/package-placeholder",
        0,
        vec![linked_file],
        Default::default(),
    )];
    program
}

pub(crate) fn runtime_package(
    package_id: &str,
    code_slot: usize,
    files: Vec<Arc<LinkedFileUnit>>,
    static_resources: PublicationResourceTable,
) -> Arc<RuntimeExecutionPackage> {
    crate::eval::test_support::runtime_execution_package_fixture(
        package_id,
        code_slot,
        files,
        static_resources,
    )
}

pub(crate) fn replace_single_package(
    program: &mut RuntimeProgram,
    package_id: &str,
    static_resources: PublicationResourceTable,
) {
    let files = program
        .packages
        .first()
        .expect("single-package fixture must install linked package code")
        .files()
        .to_vec();
    program.packages = vec![runtime_package(package_id, 0, files, static_resources)];
}

pub(crate) fn program_with_executable(executable: LinkedExecutable) -> RuntimeProgram {
    program_with_executables(vec![executable])
}

pub(crate) fn install_run_result_type(program: &mut RuntimeProgram) {
    let declaration = anonymous_type_decl(
        "RunResult",
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("label".to_string(), linked_builtin_type("string")),
                ("copy".to_string(), linked_builtin_type("string")),
            ]),
        },
    );
    Arc::make_mut(
        program
            .service_files
            .get_mut(0)
            .expect("run fixture should have one service file"),
    )
    .types
    .push(declaration.clone());
    program
        .types
        .descriptors
        .insert(service_type_addr(0), declaration);
}

pub(crate) fn program_with_thread_db_target(executable: LinkedExecutable) -> RuntimeProgram {
    let target_id = thread_db_object_target_id(0);
    let mut file = package_file_unit(
        &target_id.file_ir_ref.file_ir_identity,
        &target_id.file_ir_ref.module_path,
        run_executable(),
    );
    file.executables.clear();
    file.declarations.types.insert(
        "Thread".to_string(),
        TypeDeclarationIr {
            type_index: target_id.type_index,
            symbol: "Thread".to_string(),
            source_span: None,
        },
    );
    let thread_type = LinkedTypeRef::DbObjectSymbol {
        symbol: ServiceSymbolRef {
            module_path: "svc.main".to_string(),
            symbol: "Thread".to_string(),
        },
    };
    file.declarations.db.insert(
        "Thread".to_string(),
        DbDeclarationIr {
            type_ref: thread_type,
            type_name: "Thread".to_string(),
            collection_name: Some("Thread".to_string()),
            kind: DbObjectKindIr::Object,
            key: DbObjectKeyIr {
                name: "id".to_string(),
                ty: linked_builtin_type("string"),
            },
            fields: Vec::new(),
            leases: Vec::new(),
            indexes: Vec::new(),
            source_span: None,
        },
    );
    let declaration = anonymous_type_decl(
        "Thread",
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::new(),
        },
    );
    file.types.push(declaration.clone());

    let mut program = program_with_executable(executable);
    program.packages.push(
        crate::eval::test_support::runtime_execution_package_fixture_with_identity(
            &target_id.package_artifact_ref.package_id,
            &target_id.package_artifact_ref.package_version,
            target_id.package_artifact_ref.package_build_id.as_str(),
            target_id
                .package_artifact_ref
                .package_local_abi_identity
                .as_str(),
            0,
            vec![Arc::new(file)],
            Default::default(),
        ),
    );
    let addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::FileIrIdentity(target_id.file_ir_ref.file_ir_identity.clone()),
        type_index: target_id.type_index,
    };
    program.types.descriptors.insert(addr.clone(), declaration);
    program
        .types
        .exported_types
        .insert_package(PackageSymbolKey::new(0, "svc.main.Thread"), addr);
    program
}
