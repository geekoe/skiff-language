use std::sync::Arc;

use serde_json::Value;
use skiff_runtime_capability_context::{
    DbCapabilityError, DbCapabilityResult, DbCapabilitySource, DbProviderBuildInput,
    DbProviderConfig, DbProviderFactory,
};

use crate::{ServiceDbConfig, ServiceDbRuntime};

#[derive(Clone, Default)]
pub struct MongoServiceDbProviderFactory;

impl DbProviderFactory for MongoServiceDbProviderFactory {
    fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        let config = service_db_config_from_provider_config(input.config)?;
        let runtime =
            ServiceDbRuntime::new_with_config(input.service_id, config, &input.runtime_program_db)
                .map_err(DbCapabilityError::opaque)?;
        Ok(DbCapabilitySource::new(Some(
            Arc::new(runtime).capability_factory(),
        )))
    }
}

fn service_db_config_from_provider_config(
    config: DbProviderConfig,
) -> DbCapabilityResult<ServiceDbConfig> {
    let value = config.into_value();
    let object = value.as_object().ok_or_else(|| {
        DbCapabilityError::decode("serviceDb provider config must be a JSON object")
    })?;
    if let Some(field) = object.keys().find(|field| field.as_str() != "mongoUrl") {
        return Err(DbCapabilityError::decode(format!(
            "serviceDb provider config field {field} is not supported"
        )));
    }
    match object.get("mongoUrl") {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(ServiceDbConfig {
            mongo_url: value.clone(),
        }),
        Some(Value::String(_)) => Err(DbCapabilityError::decode(
            "serviceDb provider config field mongoUrl must be a non-empty string",
        )),
        Some(_) => Err(DbCapabilityError::decode(
            "serviceDb provider config field mongoUrl must be a string",
        )),
        None => Err(DbCapabilityError::decode(
            "serviceDb provider config field mongoUrl is required",
        )),
    }
}
