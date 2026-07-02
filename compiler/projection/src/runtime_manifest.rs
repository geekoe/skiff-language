use crate::{
    error::ProjectionError,
    runtime_manifest_model::ArtifactOperation,
    runtime_manifest_model::{
        protocol_identity_from_canonical_schema, revision_id, RuntimeGatewayManifest,
        RuntimeHttpGatewayManifest, RuntimeServiceAccessManifest, RuntimeServiceManifest,
        RuntimeTimeoutManifest, SkiffRuntimeManifest, RUNTIME_MANIFEST_SCHEMA_VERSION,
    },
    typed_artifacts::PublicInstanceExport,
    ServiceAccessProjectionConfig,
    {
        contract::{
            canonical_contract_projection_schema_with_public_instances,
            CanonicalContractProjectionSchema, ContractProjection, ContractProjectionIndex,
        },
        runtime::{
            build_artifact_operations, build_entry_point_artifacts,
            build_public_instance_artifact_operations, build_public_instance_runtime_operations,
            build_runtime_operations, build_websocket_manifest, raw_http_gateway_operation,
            EntryOperationSpec, PackageGatewayProjection,
        },
        ProjectionContext,
    },
};
use skiff_compiler_projection_input::ProjectionView;

#[derive(Debug)]
pub struct RuntimeManifestProjection {
    pub canonical_contract_schema: CanonicalContractProjectionSchema,
    pub service_operations: Vec<ArtifactOperation>,
    pub entry_service_operations: Vec<EntryOperationSpec>,
    pub manifest: SkiffRuntimeManifest,
}

pub fn project_runtime_manifest_projection(
    input: ProjectionView<'_>,
    contract_projection: &ContractProjection,
    service_version: &str,
    public_instances: &[PublicInstanceExport],
    context: &ProjectionContext<'_>,
    package_gateway_projection: &PackageGatewayProjection,
) -> Result<RuntimeManifestProjection, ProjectionError> {
    let Some(service_context) = context.as_service() else {
        return Err(ProjectionError::ContractValidation {
            message: "runtime manifest projection requires service projection context".to_string(),
        });
    };
    let service_id = service_context.service_id();
    let service_target_component = service_context.service_target_component();
    let access = service_context.access();
    let timeout = service_context.timeout();
    let service_ingress =
        input
            .service_ingress()
            .ok_or_else(|| ProjectionError::ContractValidation {
                message: "runtime manifest projection requires service ingress projection input"
                    .to_string(),
            })?;
    let contract_service_operations =
        build_artifact_operations(service_target_component, contract_projection);
    let public_instance_service_operations = build_public_instance_artifact_operations(
        service_target_component,
        contract_projection,
        public_instances,
    )?;
    let mut service_operations = contract_service_operations.clone();
    service_operations.extend(public_instance_service_operations);
    let canonical_contract_schema = canonical_contract_projection_schema_with_public_instances(
        contract_projection,
        public_instances,
    );
    let canonical_contract_schema_json = canonical_contract_schema.canonical_json();
    let protocol_identity =
        protocol_identity_from_canonical_schema(&canonical_contract_schema_json);
    let contract_projection_index =
        ContractProjectionIndex::from_projection_input_with_prelude(input, Some(context.prelude()));

    let mut runtime_operations = build_runtime_operations(
        service_id,
        service_version,
        &contract_service_operations,
        contract_projection,
        &protocol_identity,
    );
    runtime_operations.extend(build_public_instance_runtime_operations(
        service_id,
        service_version,
        service_target_component,
        public_instances,
        contract_projection,
        Some(&contract_projection_index),
        &protocol_identity,
    )?);
    let service_operation_names = contract_projection.operation_names();
    let entry_points = build_entry_point_artifacts(
        service_id,
        service_version,
        service_target_component,
        service_ingress,
        input,
        contract_projection,
        &contract_projection_index,
        &protocol_identity,
        &service_operation_names,
        package_gateway_projection,
    )?;
    runtime_operations.extend(entry_points.runtime_operations.clone());
    let mut operations = service_operations.clone();
    operations.extend(entry_points.artifact_operations.clone());
    let raw_http = if entry_points.raw_http.is_some() {
        entry_points.raw_http.clone()
    } else {
        raw_http_gateway_operation(
            service_target_component,
            contract_projection,
            &runtime_operations,
        )?
    };
    let websocket = entry_points
        .websocket
        .as_ref()
        .map(|websocket| {
            build_websocket_manifest(
                service_id,
                websocket,
                contract_projection,
                &contract_projection_index,
                &runtime_operations,
            )
        })
        .transpose()?;
    let mut service = RuntimeServiceManifest {
        id: service_id.to_string(),
        revision_id: String::new(),
        protocol_identity: protocol_identity.clone(),
        access: Some(runtime_service_access(access)),
    };
    // P1b: revision input is descriptor-based instead of raw source text.
    //
    // 它由两部分组成,合起来才能既"对实现敏感"又"对排版/重排不敏感":
    //   1. canonical contract schema JSON —— 捕获 API 签名 / schema(L74 已算)。
    //   2. lowered File IR identities —— 每个 unit 的 file_ir_identity 是其 lowered IR 的
    //      sha256,**包含可执行体**;它是规范化内容派生,空白/重排不变,但实现体一变就变。
    //
    // 只用 (1) 会漏掉"签名不变、只改实现体"的改动(那不该改 protocol identity,但**必须**改
    // revision,让 runtime reload)。加 (2) 恢复这个语义;file_ir_identity 已排序去序,保证
    // 文件顺序无关。这修复了 P1b 初版只喂 schema 导致 body-only 改动不 bump revision 的回归
    // (service_conformance::implementation_body_changes_revision_without_protocol_identity_change)。
    let mut file_ir_identities: Vec<&str> = input
        .file_ir_units()
        .iter()
        .map(|unit| unit.file_ir_identity.as_str())
        .collect();
    file_ir_identities.sort_unstable();
    let revision_input = format!(
        "{}\0fileIr\0{}",
        canonical_contract_schema_json,
        file_ir_identities.join("\0")
    );
    service.revision_id = revision_id(&service, revision_input.as_str(), &operations);
    let manifest = SkiffRuntimeManifest {
        schema_version: RUNTIME_MANIFEST_SCHEMA_VERSION.to_string(),
        service,
        operations: runtime_operations,
        gateway: Some(RuntimeGatewayManifest {
            http: (raw_http.is_some() || !entry_points.http_routes.is_empty()).then(|| {
                RuntimeHttpGatewayManifest {
                    raw: raw_http.clone(),
                    routes: entry_points.http_routes.clone(),
                }
            }),
            websocket,
        }),
        timeout: Some(RuntimeTimeoutManifest {
            default_ms: timeout.default,
            methods: timeout.methods.clone(),
        }),
    };

    Ok(RuntimeManifestProjection {
        canonical_contract_schema,
        service_operations,
        entry_service_operations: entry_points.service_operations,
        manifest,
    })
}

fn runtime_service_access(access: &ServiceAccessProjectionConfig) -> RuntimeServiceAccessManifest {
    RuntimeServiceAccessManifest {
        visibility: access.visibility,
        organization_role: access.organization_role,
    }
}
