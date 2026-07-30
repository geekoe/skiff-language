use std::sync::Arc;

use serde_json::Value;
use skiff_runtime_capability_context::{
    DbCapabilityError, DbCapabilityFuture, DbCapabilityResult, DbCapabilitySource,
    DbProviderBuildInput, DbProviderConfig, DbProviderFactory,
};

use crate::{
    index::ServiceDbIndexProvisionPlan, DbEncryptionKeyring, ServiceDbConfig, ServiceDbRuntime,
};

#[derive(Clone, Default)]
pub struct MongoServiceDbProviderFactory {
    keyring: Option<Arc<DbEncryptionKeyring>>,
}

impl MongoServiceDbProviderFactory {
    pub fn new(keyring: Option<Arc<DbEncryptionKeyring>>) -> Self {
        Self { keyring }
    }
}

impl DbProviderFactory for MongoServiceDbProviderFactory {
    fn build(&self, input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        let runtime = self.runtime_from_input(input)?;
        Ok(DbCapabilitySource::new(Some(
            Arc::new(runtime).capability_factory(),
        )))
    }

    fn provision<'a>(&'a self, inputs: Vec<DbProviderBuildInput>) -> DbCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let runtimes = inputs
                .into_iter()
                .filter(|input| !input.runtime_program_db.is_empty())
                .map(|input| self.runtime_from_input(input))
                .collect::<DbCapabilityResult<Vec<_>>>()?;
            let plan = ServiceDbIndexProvisionPlan::from_runtimes(&runtimes)
                .map_err(DbCapabilityError::opaque)?;
            plan.reconcile().await.map_err(DbCapabilityError::opaque)
        })
    }
}

impl MongoServiceDbProviderFactory {
    fn runtime_from_input(
        &self,
        input: DbProviderBuildInput,
    ) -> DbCapabilityResult<ServiceDbRuntime> {
        let mut config = service_db_config_from_provider_config(input.config)?;
        config.encryption_cipher = self.keyring.as_ref().map(|keyring| keyring.cipher());
        ServiceDbRuntime::new_with_config(
            input.environment,
            input.service_id,
            config,
            &input.runtime_program_db,
        )
        .map_err(DbCapabilityError::opaque)
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
            encryption_cipher: None,
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
