use crate::prelude::PreludeProjection;
use crate::{
    error::ProjectionError,
    {
        contract::{
            project_contract_projection, validate_contract_projection_boundary,
            validate_contract_projection_conformance, ContractProjection, ContractProjectionError,
        },
        project_config_projection,
        runtime::{
            validate_runtime_operation_modes, validate_timeout_targets, validate_websocket_gateway,
        },
        ConfigProjection, TimeoutProjectionConfig, WebSocketGatewayProjectionConfig,
    },
};
use skiff_compiler_projection_input::ProjectionView;

#[derive(Debug)]
pub struct ContractProjectionBundle {
    pub contract_projection: ContractProjection,
    pub config_projection: ConfigProjection,
}

pub fn build_service_contract_projection(
    publication_api_has_entries: bool,
    timeout: &TimeoutProjectionConfig,
    websocket_gateway: Option<&WebSocketGatewayProjectionConfig>,
    input: ProjectionView<'_>,
    prelude: &PreludeProjection,
) -> Result<ContractProjectionBundle, ProjectionError> {
    let config_projection = project_config_projection(input.source().config_requirements())?;
    let contract_projection = project_contract_projection(input, prelude).map_err(|error| {
        ProjectionError::ContractValidation {
            message: contract_projection_error_message(error),
        }
    })?;
    let has_explicit_public_instances = !input
        .source()
        .export_bindings()
        .public_instances()
        .is_empty();
    if publication_api_has_entries
        && !has_explicit_public_instances
        && !contract_projection.has_exported_interfaces()
    {
        return Err(ProjectionError::NoExportedInterfaces);
    }
    validate_contract_projection_boundary(input, &contract_projection).map_err(|error| {
        ProjectionError::ContractValidation {
            message: error.message(),
        }
    })?;
    validate_contract_projection_conformance(input, &contract_projection).map_err(|error| {
        ProjectionError::ImplementationConformance {
            message: format!("IR contract conformance: {error:?}"),
        }
    })?;

    validate_runtime_operation_modes(&contract_projection)?;
    validate_timeout_targets(timeout, &contract_projection)?;
    validate_websocket_gateway(websocket_gateway, &contract_projection)?;

    Ok(ContractProjectionBundle {
        contract_projection,
        config_projection,
    })
}

fn contract_projection_error_message(error: ContractProjectionError) -> String {
    match error {
        ContractProjectionError::PrivateContractTypeReference {
            context,
            source_symbol,
        } => {
            let context = context
                .strip_prefix("interface ")
                .map(|rest| format!("api {rest}"))
                .unwrap_or(context);
            format!(
                "IR contract projection: {context}: type {source_symbol} must be exported to appear in a service API signature"
            )
        }
        error => format!("IR contract projection: {error:?}"),
    }
}
