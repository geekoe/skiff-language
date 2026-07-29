use std::{
    collections::{hash_map::Entry, HashMap},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde_json::{json, Value};
use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_eval::TestEffectCaseContext;
use url::Url;

use crate::error::{Result, RuntimeError};

const RESERVED_SELF_INGRESS_HEADERS: &[&str] = &[
    "x-skiff-service",
    "x-skiff-version",
    "host",
    "content-length",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Clone, Default)]
pub(crate) struct TestHttpEntryRegistry {
    entries: Arc<Mutex<HashMap<String, Arc<TestHttpEntryState>>>>,
}

impl TestHttpEntryRegistry {
    pub(crate) fn begin_parent(
        &self,
        activation_id: String,
        ingress_url: &str,
        deployment: ServiceDeploymentRef,
    ) -> Result<TestHttpEntryExecution> {
        let origin = canonical_http_origin(ingress_url)?;
        let state = Arc::new(TestHttpEntryState {
            activation_id: activation_id.clone(),
            origin,
            deployment,
            effects: TestEffectCaseContext::default(),
            active_self_ingress: AtomicBool::new(false),
        });
        let mut entries = self
            .entries
            .lock()
            .expect("test HTTP entry registry lock poisoned");
        match entries.entry(activation_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(Arc::clone(&state));
            }
            Entry::Occupied(_) => {
                return Err(RuntimeError::Unsupported(format!(
                    "test case already has an active parent request for activation {activation_id}"
                )));
            }
        }
        Ok(TestHttpEntryExecution {
            effects: state.effects.clone(),
            finalize: true,
            _lease: TestHttpEntryExecutionLease::Parent {
                registry: self.clone(),
                activation_id,
                state,
            },
        })
    }

    pub(crate) fn borrow_child(&self, activation_id: &str) -> Option<TestHttpEntryExecution> {
        let state = self
            .entries
            .lock()
            .expect("test HTTP entry registry lock poisoned")
            .get(activation_id)
            .cloned()?;
        if !state.active_self_ingress.load(Ordering::Acquire) {
            return None;
        }
        Some(TestHttpEntryExecution {
            effects: state.effects.clone(),
            finalize: false,
            _lease: TestHttpEntryExecutionLease::Child { _state: state },
        })
    }

    pub(crate) fn self_ingress_for_execution(
        &self,
        activation_id: &str,
        parent: bool,
    ) -> Option<TestHttpSelfIngressContext> {
        let state = self
            .entries
            .lock()
            .expect("test HTTP entry registry lock poisoned")
            .get(activation_id)
            .cloned()?;
        if !parent && !state.active_self_ingress.load(Ordering::Acquire) {
            return None;
        }
        Some(TestHttpSelfIngressContext { state })
    }

    fn remove_parent(&self, activation_id: &str, state: &Arc<TestHttpEntryState>) {
        let mut entries = self
            .entries
            .lock()
            .expect("test HTTP entry registry lock poisoned");
        if entries
            .get(activation_id)
            .is_some_and(|active| Arc::ptr_eq(active, state))
        {
            entries.remove(activation_id);
        }
    }
}

pub(crate) struct TestHttpEntryExecution {
    effects: TestEffectCaseContext,
    finalize: bool,
    _lease: TestHttpEntryExecutionLease,
}

impl TestHttpEntryExecution {
    pub(crate) fn effects(&self) -> TestEffectCaseContext {
        self.effects.clone()
    }

    pub(crate) fn finalize(&self) -> bool {
        self.finalize
    }
}

enum TestHttpEntryExecutionLease {
    Parent {
        registry: TestHttpEntryRegistry,
        activation_id: String,
        state: Arc<TestHttpEntryState>,
    },
    Child {
        _state: Arc<TestHttpEntryState>,
    },
}

impl Drop for TestHttpEntryExecutionLease {
    fn drop(&mut self) {
        match self {
            Self::Parent {
                registry,
                activation_id,
                state,
            } => registry.remove_parent(activation_id, state),
            Self::Child { .. } => {}
        }
    }
}

struct TestHttpEntryState {
    activation_id: String,
    origin: String,
    deployment: ServiceDeploymentRef,
    effects: TestEffectCaseContext,
    active_self_ingress: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct TestHttpSelfIngressContext {
    state: Arc<TestHttpEntryState>,
}

impl TestHttpSelfIngressContext {
    pub(crate) fn matches(&self, input: &Value) -> bool {
        input
            .get("url")
            .and_then(Value::as_str)
            .and_then(|raw_url| Url::parse(raw_url).ok())
            .and_then(|url| canonical_origin(&url))
            .as_deref()
            == Some(self.state.origin.as_str())
    }

    pub(crate) fn prepare(&self, input: &Value) -> Result<Option<PreparedTestHttpSelfIngress>> {
        let Some(raw_url) = input.get("url").and_then(Value::as_str) else {
            return Ok(None);
        };
        let Ok(url) = Url::parse(raw_url) else {
            return Ok(None);
        };
        if canonical_origin(&url).as_deref() != Some(self.state.origin.as_str()) {
            return Ok(None);
        }
        let lease = self.acquire()?;
        let mut input = input.clone();
        let object = input.as_object_mut().ok_or_else(|| {
            RuntimeError::http_error("std.http.request input must be an object".to_string(), None)
        })?;
        let headers = object
            .entry("headers".to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| {
                RuntimeError::http_error(
                    "std.http.request.headers must be an array".to_string(),
                    None,
                )
            })?;
        for header in headers.iter() {
            let name = header
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if RESERVED_SELF_INGRESS_HEADERS
                .iter()
                .any(|reserved| name.eq_ignore_ascii_case(reserved))
            {
                return Err(RuntimeError::http_error(
                    format!("self-ingress HTTP request must not set runtime-owned header {name}"),
                    None,
                ));
            }
        }
        headers.push(json!({
            "name": "x-skiff-service",
            "value": self.state.deployment.service_id,
        }));
        headers.push(json!({
            "name": "x-skiff-version",
            "value": self.state.deployment.contract_version,
        }));
        Ok(Some(PreparedTestHttpSelfIngress {
            input,
            _lease: lease,
        }))
    }

    fn acquire(&self) -> Result<TestHttpSelfIngressLease> {
        self.state
            .active_self_ingress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                RuntimeError::Unsupported(format!(
                    "test case already has an active self-ingress request for activation {}",
                    self.state.activation_id
                ))
            })?;
        Ok(TestHttpSelfIngressLease {
            state: Arc::clone(&self.state),
        })
    }
}

pub(crate) struct PreparedTestHttpSelfIngress {
    input: Value,
    _lease: TestHttpSelfIngressLease,
}

impl PreparedTestHttpSelfIngress {
    #[cfg(test)]
    pub(crate) fn input(&self) -> &Value {
        &self.input
    }

    pub(crate) fn into_parts(self) -> (Value, TestHttpSelfIngressLease) {
        (self.input, self._lease)
    }
}

pub(crate) struct TestHttpSelfIngressLease {
    state: Arc<TestHttpEntryState>,
}

impl Drop for TestHttpSelfIngressLease {
    fn drop(&mut self) {
        self.state
            .active_self_ingress
            .store(false, Ordering::Release);
    }
}

fn canonical_http_origin(raw: &str) -> Result<String> {
    let url = Url::parse(raw).map_err(|_| {
        RuntimeError::Unsupported("test HTTP entry ingress URL is invalid".to_string())
    })?;
    canonical_origin(&url).ok_or_else(|| {
        RuntimeError::Unsupported(
            "test HTTP entry ingress URL must use absolute http or https".to_string(),
        )
    })
}

fn canonical_origin(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef,
    };

    use super::*;

    #[test]
    fn self_ingress_injects_exact_selector_and_releases_sequential_slot() {
        let registry = TestHttpEntryRegistry::default();
        let parent = registry
            .begin_parent(
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        assert!(parent.finalize());
        let context = registry
            .self_ingress_for_execution("activation-a", true)
            .unwrap();
        assert!(registry
            .self_ingress_for_execution("activation-a", false)
            .is_none());
        let first = context
            .prepare(&json!({
                "method": "POST",
                "url": "http://127.0.0.1:44100/entry",
                "headers": [{"name": "content-type", "value": "application/json"}],
            }))
            .unwrap()
            .unwrap();
        let child = registry.borrow_child("activation-a").unwrap();
        assert!(!child.finalize());
        assert!(registry
            .self_ingress_for_execution("activation-a", false)
            .is_some());
        let concurrent_error = context
            .prepare(&json!({
                "method": "GET",
                "url": "http://127.0.0.1:44100/other",
            }))
            .err()
            .expect("second active self-ingress must fail");
        assert!(concurrent_error
            .to_string()
            .contains("already has an active self-ingress"));
        let headers = first.input().get("headers").unwrap().as_array().unwrap();
        assert!(headers.iter().any(|header| {
            header == &json!({"name": "x-skiff-service", "value": "test.service"})
        }));
        assert!(headers
            .iter()
            .any(|header| { header == &json!({"name": "x-skiff-version", "value": "1.0.0"}) }));
        drop(first);
        assert!(context
            .prepare(&json!({
                "method": "GET",
                "url": "http://127.0.0.1:44100/other",
            }))
            .unwrap()
            .is_some());
        drop(parent);
        assert!(registry
            .self_ingress_for_execution("activation-a", true)
            .is_none());
    }

    #[test]
    fn non_self_origin_is_not_claimed_and_reserved_headers_fail_case_insensitively() {
        let registry = TestHttpEntryRegistry::default();
        let _parent = registry
            .begin_parent(
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let context = registry
            .self_ingress_for_execution("activation-a", true)
            .unwrap();
        assert!(context
            .prepare(&json!({
                "method": "GET",
                "url": "https://example.test/entry",
            }))
            .unwrap()
            .is_none());

        for name in [
            "X-SKIFF-SERVICE",
            " x-skiff-service ",
            "x-skiff-Version",
            "Host",
            "Content-Length",
            "Transfer-Encoding",
            "Connection",
            "Keep-Alive",
            "TE",
            "Trailer",
            "Upgrade",
        ] {
            let error = context
                .prepare(&json!({
                    "method": "GET",
                    "url": "http://127.0.0.1:44100/entry",
                    "headers": [{"name": name, "value": "owned"}],
                }))
                .err()
                .expect("reserved self-ingress header must fail");
            assert!(error.to_string().contains("runtime-owned header"));
        }
    }

    #[test]
    fn exact_activation_keeps_one_parent_without_replacing_it_on_duplicate_begin() {
        let registry = TestHttpEntryRegistry::default();
        let parent = registry
            .begin_parent(
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let duplicate = registry
            .begin_parent(
                "activation-a".to_string(),
                "http://127.0.0.1:44101/test-case",
                deployment(),
            )
            .err()
            .expect("duplicate parent must fail");
        assert!(duplicate.to_string().contains("active parent request"));
        assert!(registry.borrow_child("activation-a").is_none());
        assert!(registry.borrow_child("activation-b").is_none());
        assert!(registry
            .self_ingress_for_execution("activation-a", true)
            .unwrap()
            .matches(&json!({"url": "http://127.0.0.1:44100/entry"})));
        drop(parent);
        assert!(registry
            .begin_parent(
                "activation-a".to_string(),
                "http://127.0.0.1:44101/test-case",
                deployment(),
            )
            .is_ok());
    }

    fn deployment() -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: "test.service".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new("revision-1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                "skiff-deployment-artifact-v4:sha256:{}",
                "a".repeat(64)
            )),
        }
    }
}
