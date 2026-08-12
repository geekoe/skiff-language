#![allow(dead_code, unused_imports)]

use std::{env, path::PathBuf};

use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityFactory, DbCapabilityResult, DbCapabilitySource,
    DbProviderBuildInput, DbProviderFactory, DbProviderSource,
};

use crate::host::{RuntimeConfig, RuntimeHost};

mod fixture;
mod wire;

pub(super) use fixture::{
    BytecodeIdentityCorruption, PublishedFixture, TempRoot, FIXTURE_RELATIVE, PACKAGE_ID, PROFILE,
    VERSION,
};
pub(super) use wire::{
    decode_typed_response, receive_correlated_response, CanonicalSkbfRequest, CorrelatedResponse,
};

#[derive(Clone)]
pub(super) struct Correlation {
    pub(super) router_session_id: String,
    pub(super) request_id: String,
    pub(super) scenario_id: String,
}

impl Correlation {
    pub(super) fn new(scenario_id: &str) -> Self {
        Self {
            router_session_id: format!("skiff-router-session-v1:opaque:{scenario_id}"),
            request_id: format!("phase-0-{scenario_id}-request"),
            scenario_id: scenario_id.to_string(),
        }
    }

    pub(super) fn router_session_epoch(
        &self,
    ) -> crate::host::request_supervisor::RouterSessionEpoch {
        crate::host::request_supervisor::RouterSessionEpoch::from_connection_id(
            self.router_session_id.clone(),
        )
        .expect("proof correlation has a valid session epoch")
    }
}

pub(super) fn runtime_host(correlation: &Correlation) -> RuntimeHost {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::new(TestDbProviderFactory),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: format!("runtime-phase-0-{}", correlation.scenario_id),
        runtime_home: env::temp_dir().join(format!(
            "skiff-runtime-phase-0-proof-{}",
            correlation.scenario_id
        )),
        profile: PROFILE.to_string(),
        bytecode_only: true,
        http_response_max_bytes: 1024,
        http_egress_proxy: None,
    })
    .expect("construct bytecode-only production host composition");
    assert!(host
        .request_supervisor
        .start_session(correlation.router_session_epoch()));
    host
}

#[derive(Clone, Default)]
struct TestDbCapabilityFactory;

impl DbCapabilityFactory for TestDbCapabilityFactory {
    fn context_for_request(&self, _owner: String, _request_id: String) -> DbCapabilityContext {
        DbCapabilityContext::unavailable()
    }
}

#[derive(Clone, Default)]
struct TestDbProviderFactory;

impl DbProviderFactory for TestDbProviderFactory {
    fn build(&self, _input: DbProviderBuildInput) -> DbCapabilityResult<DbCapabilitySource> {
        Ok(DbCapabilitySource::new(Some(TestDbCapabilityFactory)))
    }
}
