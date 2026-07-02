use crate::error::ProjectionError;
use skiff_compiler_core::id::PublicationId;

pub fn validate_service_storage_projection_namespace(
    has_storage_metadata: bool,
    service_id: &str,
) -> Result<(), ProjectionError> {
    if !has_storage_metadata {
        return Ok(());
    }

    let mut violations = Vec::new();
    validate_service_storage_namespace(service_id, &mut violations);
    if violations.is_empty() {
        return Ok(());
    }
    Err(ProjectionError::ContractValidation {
        message: violations
            .into_iter()
            .map(|violation| format!("- {violation}"))
            .collect::<Vec<_>>()
            .join("\n"),
    })
}

fn validate_service_storage_namespace(service_id: &str, violations: &mut Vec<String>) {
    let service_id = match PublicationId::parse(service_id) {
        Ok(service_id) => service_id,
        Err(error) => {
            violations.push(format!(
                "service id {service_id} must be a publication id before Mongo database projection: {error}"
            ));
            return;
        }
    };
    let database_name = service_id.storage_safe_component();
    if let Err(error) = validate_mongo_database_name(&database_name) {
        violations.push(format!(
            "service id {service_id} cannot be projected to a Mongo database name for db/process metadata: {error}"
        ));
    }
}

fn validate_mongo_database_name(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() >= 64 {
        return Err(format!(
            "Mongo database name must be 1-63 bytes, got {}",
            value.len()
        ));
    }
    if matches!(value, "admin" | "local" | "config") {
        return Err(format!("Mongo database name `{value}` is reserved"));
    }
    if value
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || value.contains(['.', '/', '\\', '"', '$'])
    {
        return Err(format!(
            "Mongo database name `{value}` contains a forbidden character"
        ));
    }
    Ok(())
}
