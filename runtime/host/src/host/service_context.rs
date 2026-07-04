use std::{collections::HashMap, sync::Arc};

use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_eval::{EvalRuntimeProgram, EvalRuntimeProgramSource};
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkOverlay, LinkedFileUnit, LinkedProgramImage, PackageUnit,
    RuntimeProgramIdentity, RuntimeTypeContext,
};

use crate::{capability_context::DbCapabilitySource, config_view::RuntimeConfigView};
use skiff_runtime_request::{RequestOperationContext, RequestServiceMetadata, RuntimeOperation};

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct ServiceRuntimeContext {
    pub(crate) service_id: String,
    pub(crate) http_response_max_bytes: usize,
    pub(crate) activation_identity: Option<String>,
    pub(crate) resolved_config_identity: Option<String>,
    pub(crate) linked_image: Arc<LinkedProgramImage>,
    pub(crate) runtime_program_identity: RuntimeProgramIdentity,
    pub(crate) runtime_activation: Arc<RuntimeActivation>,
    pub(crate) revision_id: String,
    pub(crate) runtime_id: String,
    pub(crate) contract_identity: String,
    pub(crate) implementation_identity: String,
    pub(crate) artifact_identity: String,
    pub(crate) build_id: String,
    pub(crate) config: RuntimeConfigView,
    pub(crate) package_configs: Vec<RuntimeConfigView>,
    pub(crate) service_db: DbCapabilitySource,
}

impl ServiceRuntimeContext {
    pub(crate) fn new(
        service_id: String,
        http_response_max_bytes: usize,
        activation_identity: Option<String>,
        resolved_config_identity: Option<String>,
        linked_image: Arc<LinkedProgramImage>,
        runtime_program_identity: RuntimeProgramIdentity,
        runtime_activation: Arc<RuntimeActivation>,
        revision_id: String,
        runtime_id: String,
        contract_identity: String,
        implementation_identity: String,
        artifact_identity: String,
        build_id: String,
        config: RuntimeConfigView,
        package_configs: Vec<RuntimeConfigView>,
        service_db: DbCapabilitySource,
    ) -> Self {
        Self {
            service_id,
            revision_id,
            http_response_max_bytes,
            activation_identity,
            resolved_config_identity,
            linked_image,
            runtime_program_identity,
            runtime_activation,
            runtime_id,
            contract_identity,
            implementation_identity,
            artifact_identity,
            build_id,
            config,
            package_configs,
            service_db,
        }
    }

    pub(crate) fn service_version(&self) -> &str {
        self.runtime_activation.version.as_str()
    }

    pub(crate) fn request_metadata(&self) -> RequestServiceMetadata {
        RequestServiceMetadata {
            service_id: self.service_id.clone(),
            service_version: self.service_version().to_string(),
            runtime_id: self.runtime_id.clone(),
            build_id: self.build_id.clone(),
            http_response_max_bytes: self.http_response_max_bytes,
        }
    }
}

#[derive(Clone)]
pub(crate) struct ServiceOperationContext {
    pub(crate) service: Arc<ServiceRuntimeContext>,
    pub(crate) eval_program: Arc<EvalRuntimeProgram>,
    pub(crate) operation: RuntimeOperation,
    pub(crate) addr: ExecutableAddr,
}

impl ServiceOperationContext {
    pub(crate) fn new(
        service: Arc<ServiceRuntimeContext>,
        operation: RuntimeOperation,
        addr: ExecutableAddr,
    ) -> Self {
        let eval_program = Arc::new(EvalRuntimeProgram::from_source(
            &ServiceEvalRuntimeProgramSource::new(service.as_ref()),
        ));
        Self {
            service,
            eval_program,
            operation,
            addr,
        }
    }

    pub(crate) fn request_operation_context(&self) -> RequestOperationContext {
        RequestOperationContext::new(
            self.service.request_metadata(),
            self.eval_program.clone(),
            self.operation.clone(),
            self.addr.clone(),
        )
    }
}

struct ServiceEvalRuntimeProgramSource<'a> {
    service: &'a ServiceRuntimeContext,
}

impl<'a> ServiceEvalRuntimeProgramSource<'a> {
    fn new(service: &'a ServiceRuntimeContext) -> Self {
        Self { service }
    }
}

impl EvalRuntimeProgramSource for ServiceEvalRuntimeProgramSource<'_> {
    fn service_id(&self) -> &str {
        &self.service.service_id
    }

    fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        self.service.linked_image.service_files.as_slice()
    }

    fn packages(&self) -> &[Arc<PackageUnit>] {
        self.service.linked_image.packages.as_slice()
    }

    fn package_files(&self) -> &[Vec<Arc<LinkedFileUnit>>] {
        self.service.linked_image.package_files.as_slice()
    }

    fn spawn_routes(&self) -> &HashMap<String, ExecutableAddr> {
        &self.service.linked_image.spawn_routes
    }

    fn link_overlay(&self) -> &LinkOverlay {
        &self.service.linked_image.link_overlay
    }

    fn types(&self) -> &RuntimeTypeContext {
        &self.service.linked_image.types
    }
}
