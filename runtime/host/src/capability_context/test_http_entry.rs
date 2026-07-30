use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde_json::{json, Value};
use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_eval::TestEffectCaseContext;
use tokio::sync::oneshot;
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TestCaseCapability(String);

impl TestCaseCapability {
    fn from_router(value: &str) -> Result<Self> {
        if value.is_empty() {
            return Err(RuntimeError::Unsupported(
                "test case capability must not be empty".to_string(),
            ));
        }
        Ok(Self(value.to_string()))
    }
}

#[derive(Default)]
struct TestCaseRegistryOwner {
    cases: HashMap<TestCaseCapability, Arc<TestCaseState>>,
    requests: HashMap<String, TestCaseCapability>,
}

struct TestCaseState {
    capability: TestCaseCapability,
    root_request_id: String,
    http: Arc<TestHttpEntryState>,
    lifecycle: Mutex<TestCaseLifecycle>,
    finalization_sender: Mutex<Option<oneshot::Sender<skiff_runtime_eval::error::Result<()>>>>,
    #[cfg(test)]
    finalization_count: std::sync::atomic::AtomicUsize,
}

#[derive(Default)]
struct TestCaseLifecycle {
    root_closing: bool,
    derived_request_ids: HashSet<String>,
    finalization_started: bool,
}

/// Runtime-local owner for Router-issued opaque test case capabilities.
///
/// The wire token is converted to `TestCaseCapability` immediately and never
/// becomes a business-visible value. Request ids are bound exactly once so a
/// duplicate admission cannot acquire a second lease.
#[derive(Clone, Default)]
pub(crate) struct TestCaseRegistry {
    owner: Arc<Mutex<TestCaseRegistryOwner>>,
}

impl TestCaseRegistry {
    pub(crate) fn begin_root(
        &self,
        capability: &str,
        request_id: String,
        activation_id: String,
        ingress_url: &str,
        deployment: ServiceDeploymentRef,
    ) -> Result<TestCaseRootLease> {
        let capability = TestCaseCapability::from_router(capability)?;
        if request_id.is_empty() {
            return Err(RuntimeError::Unsupported(
                "test case root request id must not be empty".to_string(),
            ));
        }
        let origin = canonical_http_origin(ingress_url)?;
        let (finalization_sender, finalization_receiver) = oneshot::channel();
        let state = Arc::new(TestCaseState {
            capability: capability.clone(),
            root_request_id: request_id.clone(),
            http: Arc::new(TestHttpEntryState {
                activation_id,
                origin,
                deployment,
                effects: TestEffectCaseContext::default(),
                active_self_ingress: AtomicBool::new(false),
            }),
            lifecycle: Mutex::new(TestCaseLifecycle::default()),
            finalization_sender: Mutex::new(Some(finalization_sender)),
            #[cfg(test)]
            finalization_count: std::sync::atomic::AtomicUsize::new(0),
        });
        let mut owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        if owner.cases.contains_key(&capability) {
            return Err(RuntimeError::Unsupported(
                "test case capability was already registered".to_string(),
            ));
        }
        if owner.requests.contains_key(&request_id) {
            return Err(RuntimeError::Unsupported(format!(
                "test request id {request_id} was already registered"
            )));
        }
        owner.requests.insert(request_id, capability.clone());
        owner.cases.insert(capability, Arc::clone(&state));
        drop(owner);
        Ok(TestCaseRootLease {
            registry: self.clone(),
            state,
            finalization_receiver: Some(finalization_receiver),
            closed: false,
        })
    }

    /// Registers one already-established derived request before Eval begins.
    ///
    /// Closing does not reject recursive derivation while another derived
    /// request is still live. Once the last derived reference starts
    /// finalization, the capability is removed and all later borrows fail
    /// closed.
    pub(crate) fn begin_derived(
        &self,
        capability: &str,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        let capability = TestCaseCapability::from_router(capability)?;
        if request_id.is_empty() {
            return Err(RuntimeError::Unsupported(
                "derived test request id must not be empty".to_string(),
            ));
        }
        let mut owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        if owner.requests.contains_key(&request_id) {
            return Err(RuntimeError::Unsupported(format!(
                "test request id {request_id} was already registered"
            )));
        }
        let state = owner.cases.get(&capability).cloned().ok_or_else(|| {
            RuntimeError::Unsupported("unknown or finalized test case capability".to_string())
        })?;
        {
            let mut lifecycle = state
                .lifecycle
                .lock()
                .expect("test case lifecycle lock poisoned");
            if lifecycle.finalization_started {
                return Err(RuntimeError::Unsupported(
                    "test case capability is already finalizing".to_string(),
                ));
            }
            lifecycle.derived_request_ids.insert(request_id.clone());
        }
        owner.requests.insert(request_id.clone(), capability);
        drop(owner);
        Ok(TestCaseDerivedLease {
            registry: self.clone(),
            state,
            request_id,
            released: false,
        })
    }

    fn close_root(&self, state: &Arc<TestCaseState>) {
        let finalization = {
            let mut owner = self
                .owner
                .lock()
                .expect("test case registry owner lock poisoned");
            {
                let mut lifecycle = state
                    .lifecycle
                    .lock()
                    .expect("test case lifecycle lock poisoned");
                lifecycle.root_closing = true;
            }
            if owner
                .requests
                .get(&state.root_request_id)
                .is_some_and(|capability| capability == &state.capability)
            {
                owner.requests.remove(&state.root_request_id);
            }
            prepare_finalization(&mut owner, state)
        };
        finalize_case(finalization);
    }

    fn release_derived(&self, state: &Arc<TestCaseState>, request_id: &str) {
        let finalization = {
            let mut owner = self
                .owner
                .lock()
                .expect("test case registry owner lock poisoned");
            {
                let mut lifecycle = state
                    .lifecycle
                    .lock()
                    .expect("test case lifecycle lock poisoned");
                lifecycle.derived_request_ids.remove(request_id);
            }
            if owner
                .requests
                .get(request_id)
                .is_some_and(|capability| capability == &state.capability)
            {
                owner.requests.remove(request_id);
            }
            prepare_finalization(&mut owner, state)
        };
        finalize_case(finalization);
    }

    #[cfg(test)]
    fn contains_capability(&self, capability: &str) -> bool {
        let Ok(capability) = TestCaseCapability::from_router(capability) else {
            return false;
        };
        self.owner
            .lock()
            .expect("test case registry owner lock poisoned")
            .cases
            .contains_key(&capability)
    }

    #[cfg(test)]
    fn contains_request(&self, request_id: &str) -> bool {
        self.owner
            .lock()
            .expect("test case registry owner lock poisoned")
            .requests
            .contains_key(request_id)
    }

    #[cfg(test)]
    fn owner_counts(&self) -> (usize, usize) {
        let owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        (owner.cases.len(), owner.requests.len())
    }

    #[cfg(test)]
    fn owner_weak(&self) -> std::sync::Weak<Mutex<TestCaseRegistryOwner>> {
        Arc::downgrade(&self.owner)
    }

    pub(crate) fn self_ingress_for_request(
        &self,
        request_id: &str,
    ) -> Option<TestHttpSelfIngressContext> {
        let owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        let capability = owner.requests.get(request_id)?;
        let state = owner.cases.get(capability)?;
        Some(TestHttpSelfIngressContext {
            state: Arc::clone(&state.http),
        })
    }

    pub(crate) fn begin_nested_http(
        &self,
        activation_id: &str,
        request_id: String,
    ) -> Result<Option<TestCaseDerivedLease>> {
        let capability = {
            let owner = self
                .owner
                .lock()
                .expect("test case registry owner lock poisoned");
            let mut active = owner
                .cases
                .values()
                .filter(|state| {
                    state.http.activation_id == activation_id
                        && state.http.active_self_ingress.load(Ordering::Acquire)
                })
                .map(|state| state.capability.clone());
            let Some(capability) = active.next() else {
                return Ok(None);
            };
            if active.next().is_some() {
                return Err(RuntimeError::Unsupported(format!(
                    "multiple test cases have active self-ingress for activation {activation_id}"
                )));
            }
            capability
        };
        self.begin_derived(&capability.0, request_id).map(Some)
    }
}

type PendingCaseFinalization = Option<(
    TestEffectCaseContext,
    oneshot::Sender<skiff_runtime_eval::error::Result<()>>,
    Arc<TestCaseState>,
)>;

fn prepare_finalization(
    owner: &mut TestCaseRegistryOwner,
    state: &Arc<TestCaseState>,
) -> PendingCaseFinalization {
    let mut lifecycle = state
        .lifecycle
        .lock()
        .expect("test case lifecycle lock poisoned");
    if !lifecycle.root_closing
        || !lifecycle.derived_request_ids.is_empty()
        || lifecycle.finalization_started
    {
        return None;
    }
    lifecycle.finalization_started = true;
    if owner
        .cases
        .get(&state.capability)
        .is_some_and(|registered| Arc::ptr_eq(registered, state))
    {
        owner.cases.remove(&state.capability);
    }
    let sender = state
        .finalization_sender
        .lock()
        .expect("test case finalization sender lock poisoned")
        .take()
        .expect("test case finalization sender must exist before finalization");
    Some((state.http.effects.clone(), sender, Arc::clone(state)))
}

fn finalize_case(finalization: PendingCaseFinalization) {
    let Some((effects, sender, _state)) = finalization else {
        return;
    };
    #[cfg(test)]
    _state.finalization_count.fetch_add(1, Ordering::AcqRel);
    let _ = sender.send(effects.finalize());
}

pub(crate) struct TestCaseRootLease {
    registry: TestCaseRegistry,
    state: Arc<TestCaseState>,
    finalization_receiver: Option<oneshot::Receiver<skiff_runtime_eval::error::Result<()>>>,
    closed: bool,
}

impl TestCaseRootLease {
    pub(crate) fn effects(&self) -> TestEffectCaseContext {
        self.state.http.effects.clone()
    }

    pub(crate) async fn finalize(mut self) -> skiff_runtime_eval::error::Result<()> {
        self.close();
        self.finalization_receiver
            .take()
            .expect("test case root finalization receiver must exist")
            .await
            .map_err(|_| {
                skiff_runtime_eval::error::RuntimeError::Unsupported(
                    "test case finalization owner ended without a result".to_string(),
                )
            })?
    }

    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.registry.close_root(&self.state);
        }
    }

    #[cfg(test)]
    fn finalization_count(&self) -> usize {
        self.state.finalization_count.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn same_case_as(&self, derived: &TestCaseDerivedLease) -> bool {
        Arc::ptr_eq(&self.state, &derived.state)
    }
}

impl Drop for TestCaseRootLease {
    fn drop(&mut self) {
        self.close();
    }
}

pub(crate) struct TestCaseDerivedLease {
    registry: TestCaseRegistry,
    state: Arc<TestCaseState>,
    request_id: String,
    released: bool,
}

impl TestCaseDerivedLease {
    pub(crate) fn effects(&self) -> TestEffectCaseContext {
        self.state.http.effects.clone()
    }

    #[cfg(test)]
    fn same_case_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }
}

impl Drop for TestCaseDerivedLease {
    fn drop(&mut self) {
        if !self.released {
            self.released = true;
            self.registry.release_derived(&self.state, &self.request_id);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct TestHttpEntryRegistry {
    test_cases: TestCaseRegistry,
}

impl TestHttpEntryRegistry {
    pub(crate) fn begin_root_case(
        &self,
        capability: &str,
        request_id: String,
        activation_id: String,
        ingress_url: &str,
        deployment: ServiceDeploymentRef,
    ) -> Result<TestCaseRootLease> {
        self.test_cases.begin_root(
            capability,
            request_id,
            activation_id,
            ingress_url,
            deployment,
        )
    }

    pub(crate) fn begin_derived(
        &self,
        capability: &str,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        self.test_cases.begin_derived(capability, request_id)
    }

    pub(crate) fn self_ingress_for_request(
        &self,
        request_id: &str,
    ) -> Option<TestHttpSelfIngressContext> {
        self.test_cases.self_ingress_for_request(request_id)
    }

    pub(crate) fn begin_nested_http(
        &self,
        activation_id: &str,
        request_id: String,
    ) -> Result<Option<TestCaseDerivedLease>> {
        self.test_cases.begin_nested_http(activation_id, request_id)
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

    #[tokio::test]
    async fn self_ingress_injects_exact_selector_and_releases_sequential_slot() {
        let registry = TestHttpEntryRegistry::default();
        let root = registry
            .begin_root_case(
                "case-a",
                "root-a".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let context = registry.self_ingress_for_request("root-a").unwrap();
        assert!(registry
            .begin_nested_http("activation-a", "before-active".to_string())
            .unwrap()
            .is_none());
        let first = context
            .prepare(&json!({
                "method": "POST",
                "url": "http://127.0.0.1:44100/entry",
                "headers": [{"name": "content-type", "value": "application/json"}],
            }))
            .unwrap()
            .unwrap();
        let child = registry
            .begin_nested_http("activation-a", "child-a".to_string())
            .unwrap()
            .unwrap();
        assert!(root.same_case_as(&child));
        assert!(registry.self_ingress_for_request("child-a").is_some());
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
        drop(child);
        let second = context
            .prepare(&json!({
                "method": "GET",
                "url": "http://127.0.0.1:44100/other",
            }))
            .unwrap()
            .unwrap();
        drop(second);
        root.finalize().await.unwrap();
        assert_eq!(registry.test_cases.owner_counts(), (0, 0));
    }

    #[test]
    fn non_self_origin_is_not_claimed_and_reserved_headers_fail_case_insensitively() {
        let registry = TestHttpEntryRegistry::default();
        let _root = registry
            .begin_root_case(
                "case-a",
                "root-a".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let context = registry.self_ingress_for_request("root-a").unwrap();
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
    fn exact_capability_rejects_duplicate_but_allows_parallel_activation_cases() {
        let registry = TestHttpEntryRegistry::default();
        let root = registry
            .begin_root_case(
                "case-a",
                "root-a".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let duplicate = registry
            .begin_root_case(
                "case-a",
                "root-b".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44101/test-case",
                deployment(),
            )
            .err()
            .expect("duplicate parent must fail");
        assert!(duplicate.to_string().contains("already registered"));
        let parallel = registry
            .begin_root_case(
                "case-b",
                "root-b".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44101/test-case",
                deployment(),
            )
            .unwrap();
        assert!(registry
            .self_ingress_for_request("root-a")
            .unwrap()
            .matches(&json!({"url": "http://127.0.0.1:44100/entry"})));
        assert!(registry
            .self_ingress_for_request("root-b")
            .unwrap()
            .matches(&json!({"url": "http://127.0.0.1:44101/entry"})));
        drop(root);
        drop(parallel);
        assert_eq!(registry.test_cases.owner_counts(), (0, 0));
    }

    #[tokio::test]
    async fn opaque_capability_isolates_concurrent_cases_on_one_activation() {
        let registry = TestHttpEntryRegistry::default();
        let root_a = registry
            .begin_root_case(
                "case-a",
                "root-a".to_string(),
                "activation-shared".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let root_b = registry
            .begin_root_case(
                "case-b",
                "root-b".to_string(),
                "activation-shared".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let child_a = registry
            .begin_derived("case-a", "child-a".to_string())
            .unwrap();
        let child_b = registry
            .begin_derived("case-b", "child-b".to_string())
            .unwrap();

        assert!(root_a.same_case_as(&child_a));
        assert!(root_b.same_case_as(&child_b));
        assert!(!child_a.same_case_as(&child_b));
        assert!(registry
            .self_ingress_for_request("root-a")
            .unwrap()
            .matches(&json!({"url": "http://127.0.0.1:44100/a"})));
        assert!(registry
            .self_ingress_for_request("child-b")
            .unwrap()
            .matches(&json!({"url": "http://127.0.0.1:44100/b"})));

        drop(child_a);
        drop(child_b);
        root_a.finalize().await.unwrap();
        root_b.finalize().await.unwrap();
        assert!(!registry.test_cases.contains_capability("case-a"));
        assert!(!registry.test_cases.contains_capability("case-b"));
    }

    #[test]
    fn concurrent_self_ingress_on_one_activation_fails_closed_instead_of_crossing_cases() {
        let registry = TestHttpEntryRegistry::default();
        let root_a = registry
            .begin_root_case(
                "case-a",
                "root-a".to_string(),
                "activation-shared".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let root_b = registry
            .begin_root_case(
                "case-b",
                "root-b".to_string(),
                "activation-shared".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let active_a = registry
            .self_ingress_for_request("root-a")
            .unwrap()
            .prepare(&json!({
                "method": "GET",
                "url": "http://127.0.0.1:44100/a",
            }))
            .unwrap()
            .unwrap();
        let active_b = registry
            .self_ingress_for_request("root-b")
            .unwrap()
            .prepare(&json!({
                "method": "GET",
                "url": "http://127.0.0.1:44100/b",
            }))
            .unwrap()
            .unwrap();

        let error = registry
            .begin_nested_http("activation-shared", "ambiguous-child".to_string())
            .err()
            .expect("ambiguous nested ingress must fail closed");
        assert!(error
            .to_string()
            .contains("multiple test cases have active self-ingress"));
        assert_eq!(registry.test_cases.owner_counts(), (2, 2));

        drop(active_a);
        drop(active_b);
        drop(root_a);
        drop(root_b);
        assert_eq!(registry.test_cases.owner_counts(), (0, 0));
    }

    #[tokio::test]
    async fn root_finalization_waits_for_recursive_derived_requests() {
        let registry = TestHttpEntryRegistry::default();
        let root = registry
            .begin_root_case(
                "case-recursive",
                "root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let first = registry
            .begin_derived("case-recursive", "child-1".to_string())
            .unwrap();
        let state = Arc::clone(&root.state);
        let finalization = tokio::spawn(root.finalize());
        tokio::task::yield_now().await;
        assert!(registry.test_cases.contains_capability("case-recursive"));

        let recursive = registry
            .begin_derived("case-recursive", "child-2".to_string())
            .unwrap();
        assert!(first.same_case_as(&recursive));
        drop(first);
        assert!(registry.test_cases.contains_capability("case-recursive"));
        assert!(!finalization.is_finished());
        drop(recursive);

        finalization.await.unwrap().unwrap();
        assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
        assert!(!registry.test_cases.contains_capability("case-recursive"));
        assert!(!registry.test_cases.contains_request("child-1"));
        assert!(!registry.test_cases.contains_request("child-2"));
    }

    #[tokio::test]
    async fn dropped_root_waiter_does_not_cancel_child_and_child_cleans_up() {
        let registry = TestHttpEntryRegistry::default();
        let root = registry
            .begin_root_case(
                "case-cancelled-root",
                "root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let child = registry
            .begin_derived("case-cancelled-root", "child".to_string())
            .unwrap();
        let state = Arc::clone(&root.state);
        let finalization = tokio::spawn(root.finalize());
        tokio::task::yield_now().await;
        finalization.abort();
        let _ = finalization.await;

        assert!(registry
            .test_cases
            .contains_capability("case-cancelled-root"));
        assert!(registry.test_cases.contains_request("child"));
        drop(child);
        assert!(!registry
            .test_cases
            .contains_capability("case-cancelled-root"));
        assert!(!registry.test_cases.contains_request("child"));
        assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn unknown_and_active_duplicate_capabilities_fail_closed_without_leaks() {
        let registry = TestHttpEntryRegistry::default();
        let root = registry
            .begin_root_case(
                "case-once",
                "root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        assert!(registry
            .begin_root_case(
                "case-once",
                "other-root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .err()
            .expect("duplicate capability must fail")
            .to_string()
            .contains("already registered"));
        assert!(registry
            .begin_derived("missing", "missing-child".to_string())
            .err()
            .expect("unknown capability must fail")
            .to_string()
            .contains("unknown or finalized"));
        let child = registry
            .begin_derived("case-once", "child".to_string())
            .unwrap();
        assert!(registry
            .begin_derived("case-once", "child".to_string())
            .err()
            .expect("duplicate derived request id must fail")
            .to_string()
            .contains("request id child was already registered"));
        assert!(registry
            .begin_root_case(
                "other-case",
                "child".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .err()
            .expect("request id already bound to a derived request must fail")
            .to_string()
            .contains("request id child was already registered"));

        drop(child);
        root.finalize().await.unwrap();
        assert!(registry
            .begin_derived("case-once", "late-child".to_string())
            .err()
            .expect("finalized capability must be unknown to derived requests")
            .to_string()
            .contains("unknown or finalized"));
        assert_eq!(registry.test_cases.owner_counts(), (0, 0));

        // Router capabilities are random and not expected to be reissued.
        // The Runtime does not retain an unbounded tombstone merely to reject
        // a hypothetical root replay after the prior case is fully gone.
        registry
            .begin_root_case(
                "case-once",
                "fresh-root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap()
            .finalize()
            .await
            .unwrap();
        assert_eq!(registry.test_cases.owner_counts(), (0, 0));
    }

    #[test]
    fn root_drop_closes_case_but_keeps_derived_alive_until_terminal_drop() {
        let registry = TestHttpEntryRegistry::default();
        let root = registry
            .begin_root_case(
                "case-drop",
                "root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let child = registry
            .begin_derived("case-drop", "child".to_string())
            .unwrap();
        let state = Arc::clone(&root.state);
        assert_eq!(root.finalization_count(), 0);
        drop(root);
        assert!(registry.test_cases.contains_capability("case-drop"));
        let recursive = registry
            .begin_derived("case-drop", "recursive".to_string())
            .unwrap();
        drop(child);
        assert!(registry.test_cases.contains_capability("case-drop"));
        drop(recursive);
        assert!(!registry.test_cases.contains_capability("case-drop"));
        assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
        assert_eq!(registry.test_cases.owner_counts(), (0, 0));
    }

    #[test]
    fn dropping_host_registry_releases_owner_after_live_request_leases_end() {
        let registry = TestHttpEntryRegistry::default();
        let owner = registry.test_cases.owner_weak();
        let root = registry
            .begin_root_case(
                "case-host-drop",
                "root".to_string(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let child = registry
            .begin_derived("case-host-drop", "child".to_string())
            .unwrap();

        drop(registry);
        assert!(owner.upgrade().is_some());
        drop(root);
        assert!(owner.upgrade().is_some());
        drop(child);
        assert!(owner.upgrade().is_none());
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
