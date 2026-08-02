use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde_json::{json, Value};
use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_capability_context::ConnectionRequestSession;
use skiff_runtime_eval::TestEffectCaseContext;
use tokio::sync::oneshot;
use url::Url;

use crate::error::{Result, RuntimeError};

const TEST_CASE_CAPABILITY_HEADER: &str = "x-skiff-test-case-capability";
const TEST_CASE_PARENT_REQUEST_ID_HEADER: &str = "x-skiff-test-case-parent-request-id";

const RESERVED_SELF_INGRESS_HEADERS: &[&str] = &[
    "x-skiff-service",
    "x-skiff-version",
    TEST_CASE_CAPABILITY_HEADER,
    TEST_CASE_PARENT_REQUEST_ID_HEADER,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestRequestAuthority {
    capability: TestCaseCapability,
    router_session: ConnectionRequestSession,
}

/// Runtime-local identity for one admission of a wire request id.
///
/// Request ids may be reused after their parenting authority is revoked while the old ownership
/// lease is still alive. The generation makes release and exact revocation ABA-safe in that
/// overlap.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TestRequestIdentity {
    request_id: String,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TestRequestRegistration {
    identity: TestRequestIdentity,
    authority: TestRequestAuthority,
}

#[derive(Default)]
struct TestCaseRegistryOwner {
    cases: HashMap<TestCaseCapability, Arc<TestCaseState>>,
    requests: HashMap<String, TestRequestRegistration>,
    next_request_generation: u64,
}

struct TestCaseState {
    capability: TestCaseCapability,
    router_session: ConnectionRequestSession,
    root_request_id: String,
    root_request_identity: TestRequestIdentity,
    http: Arc<TestHttpEntryState>,
    lifecycle: Mutex<TestCaseLifecycle>,
    finalization_sender: Mutex<Option<oneshot::Sender<skiff_runtime_eval::error::Result<()>>>>,
    #[cfg(test)]
    finalization_count: std::sync::atomic::AtomicUsize,
}

impl TestCaseState {
    fn admitted_context(&self, request_id: &str) -> TestHttpAdmittedContext {
        TestHttpAdmittedContext {
            capability: self.capability.0.clone(),
            router_session: self.router_session.clone(),
            request_id: request_id.to_string(),
            http: Arc::clone(&self.http),
        }
    }
}

#[derive(Default)]
struct TestCaseLifecycle {
    root_closing: bool,
    root_released: bool,
    session_disconnected: bool,
    derived_requests: HashSet<TestRequestIdentity>,
    finalization_started: bool,
}

/// Runtime-local owner for Router-issued opaque test case capabilities.
///
/// The wire token is converted to `TestCaseCapability` immediately and never becomes a
/// business-visible value. At most one parenting authority is active for a wire request id, while
/// every ownership lease is bound to its own opaque admission generation.
#[derive(Clone, Default)]
pub(crate) struct TestCaseRegistry {
    owner: Arc<Mutex<TestCaseRegistryOwner>>,
}

impl TestCaseRegistry {
    pub(crate) fn begin_root(
        &self,
        capability: &str,
        router_session_id: &str,
        request_id: String,
        activation_id: String,
        ingress_url: &str,
        deployment: ServiceDeploymentRef,
    ) -> Result<TestCaseRootLease> {
        let capability = TestCaseCapability::from_router(capability)?;
        let router_session = ConnectionRequestSession::new(router_session_id.to_string())
            .map_err(RuntimeError::Unsupported)?;
        if request_id.is_empty() {
            return Err(RuntimeError::Unsupported(
                "test case root request id must not be empty".to_string(),
            ));
        }
        let origin = canonical_http_origin(ingress_url)?;
        let (finalization_sender, finalization_receiver) = oneshot::channel();
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
        let root_request_identity = next_request_identity(&mut owner, &request_id)?;
        let state = Arc::new(TestCaseState {
            capability: capability.clone(),
            router_session: router_session.clone(),
            root_request_id: request_id.clone(),
            root_request_identity: root_request_identity.clone(),
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
        owner.requests.insert(
            request_id,
            TestRequestRegistration {
                identity: root_request_identity,
                authority: TestRequestAuthority {
                    capability: capability.clone(),
                    router_session,
                },
            },
        );
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
        router_session_id: &str,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        self.begin_derived_inner(capability, router_session_id, None, request_id)
    }

    /// Registers a derived HTTP, spawn, or Actor request only while its authenticated parent is
    /// still an active member of the same opaque test case. Capability, session, parent
    /// validation, and child insertion share one owner lock so concurrent parent release cannot
    /// admit late work.
    pub(crate) fn begin_derived_from_parent(
        &self,
        capability: &str,
        parent_request_id: &str,
        router_session_id: &str,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        self.begin_derived_inner(
            capability,
            router_session_id,
            Some(parent_request_id),
            request_id,
        )
    }

    fn begin_derived_inner(
        &self,
        capability: &str,
        router_session_id: &str,
        parent_request_id: Option<&str>,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        let capability = TestCaseCapability::from_router(capability)?;
        let router_session = ConnectionRequestSession::new(router_session_id.to_string())
            .map_err(RuntimeError::Unsupported)?;
        if parent_request_id.is_some_and(str::is_empty) {
            return Err(RuntimeError::Unsupported(
                "derived test parent request id must not be empty".to_string(),
            ));
        }
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
        if let Some(parent_request_id) = parent_request_id {
            if !owner
                .requests
                .get(parent_request_id)
                .is_some_and(|registration| {
                    registration.authority.capability == capability
                        && registration.authority.router_session == router_session
                })
            {
                return Err(RuntimeError::Unsupported(
                    "test case parent request is unknown, finalized, belongs to another case, or belongs to another router session".to_string(),
                ));
            }
        }
        let state = owner.cases.get(&capability).cloned().ok_or_else(|| {
            RuntimeError::Unsupported("unknown or finalized test case capability".to_string())
        })?;
        if state.router_session != router_session {
            return Err(RuntimeError::Unsupported(
                "test case capability belongs to another router session".to_string(),
            ));
        }
        {
            let lifecycle = state
                .lifecycle
                .lock()
                .expect("test case lifecycle lock poisoned");
            if lifecycle.finalization_started {
                return Err(RuntimeError::Unsupported(
                    "test case capability is already finalizing".to_string(),
                ));
            }
            if lifecycle.session_disconnected {
                return Err(RuntimeError::Unsupported(
                    "test case router session is disconnected".to_string(),
                ));
            }
        }
        let request_identity = next_request_identity(&mut owner, &request_id)?;
        state
            .lifecycle
            .lock()
            .expect("test case lifecycle lock poisoned")
            .derived_requests
            .insert(request_identity.clone());
        owner.requests.insert(
            request_id,
            TestRequestRegistration {
                identity: request_identity.clone(),
                authority: TestRequestAuthority {
                    capability,
                    router_session,
                },
            },
        );
        drop(owner);
        Ok(TestCaseDerivedLease {
            registry: self.clone(),
            state,
            identity: request_identity,
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
                lifecycle.root_released = true;
            }
            if owner
                .requests
                .get(&state.root_request_id)
                .is_some_and(|registration| registration.identity == state.root_request_identity)
            {
                owner.requests.remove(&state.root_request_id);
            }
            prepare_finalization(&mut owner, state)
        };
        finalize_case(finalization);
    }

    fn release_derived(&self, state: &Arc<TestCaseState>, identity: &TestRequestIdentity) {
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
                lifecycle.derived_requests.remove(identity);
            }
            if owner
                .requests
                .get(&identity.request_id)
                .is_some_and(|registration| registration.identity == *identity)
            {
                owner.requests.remove(&identity.request_id);
            }
            prepare_finalization(&mut owner, state)
        };
        finalize_case(finalization);
    }

    /// Closes every test authority issued on one Router connection.
    ///
    /// Live leases remain valid ownership guards and finish through their normal Drop path, but
    /// all request membership is removed atomically before this method returns. Therefore a
    /// reconnected Router cannot replay a capability or parent request id to derive more work.
    pub(crate) fn disconnect_session(&self, router_session_id: &str) -> Result<()> {
        let router_session = ConnectionRequestSession::new(router_session_id.to_string())
            .map_err(RuntimeError::Unsupported)?;
        let finalizations = {
            let mut owner = self
                .owner
                .lock()
                .expect("test case registry owner lock poisoned");
            let states = owner
                .cases
                .values()
                .filter(|state| state.router_session == router_session)
                .cloned()
                .collect::<Vec<_>>();
            owner
                .requests
                .retain(|_, registration| registration.authority.router_session != router_session);
            for state in &states {
                let mut lifecycle = state
                    .lifecycle
                    .lock()
                    .expect("test case lifecycle lock poisoned");
                lifecycle.root_closing = true;
                lifecycle.session_disconnected = true;
            }
            states
                .iter()
                .filter_map(|state| prepare_finalization(&mut owner, state))
                .collect::<Vec<_>>()
        };
        for finalization in finalizations {
            finalize_case(Some(finalization));
        }
        Ok(())
    }

    /// Revokes one request's ability to parent more derived work without releasing its lease.
    ///
    /// Cancellation, deadline, and completed-Eval winners call this as soon as their outcome is
    /// known. The owning lease continues to keep case finalization pending through terminal
    /// encode/send, while recursive admission fails immediately.
    #[cfg(test)]
    pub(crate) fn revoke_request(&self, router_session_id: &str, request_id: &str) -> bool {
        let Ok(router_session) = ConnectionRequestSession::new(router_session_id.to_string())
        else {
            return false;
        };
        let mut owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        if !owner
            .requests
            .get(request_id)
            .is_some_and(|registration| registration.authority.router_session == router_session)
        {
            return false;
        }
        owner.requests.remove(request_id);
        true
    }

    fn revoke_exact(&self, identity: &TestRequestIdentity) -> bool {
        let mut owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        if !owner
            .requests
            .get(&identity.request_id)
            .is_some_and(|registration| registration.identity == *identity)
        {
            return false;
        }
        owner.requests.remove(&identity.request_id);
        true
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

    #[cfg(test)]
    pub(crate) fn self_ingress_for_request(
        &self,
        router_session_id: &str,
        request_id: &str,
    ) -> Option<TestHttpSelfIngressContext> {
        let router_session = ConnectionRequestSession::new(router_session_id.to_string()).ok()?;
        let owner = self
            .owner
            .lock()
            .expect("test case registry owner lock poisoned");
        let registration = owner.requests.get(request_id)?;
        let state = owner.cases.get(&registration.authority.capability)?;
        if state.router_session != registration.authority.router_session
            || registration.authority.router_session != router_session
        {
            return None;
        }
        Some(TestHttpSelfIngressContext {
            state: Arc::clone(&state.http),
            test_case_capability: registration.authority.capability.0.clone(),
            parent_request_id: request_id.to_string(),
        })
    }

    pub(crate) fn begin_nested_http(
        &self,
        activation_id: &str,
        router_session_id: &str,
        request_id: String,
    ) -> Result<Option<TestCaseDerivedLease>> {
        let router_session = ConnectionRequestSession::new(router_session_id.to_string())
            .map_err(RuntimeError::Unsupported)?;
        let capability = {
            let owner = self
                .owner
                .lock()
                .expect("test case registry owner lock poisoned");
            let active = owner
                .cases
                .values()
                .filter(|state| {
                    state.http.activation_id == activation_id
                        && state.http.active_self_ingress.load(Ordering::Acquire)
                })
                .collect::<Vec<_>>();
            let matching = active
                .iter()
                .copied()
                .filter(|state| state.router_session == router_session)
                .collect::<Vec<_>>();
            if matching.len() > 1 {
                return Err(RuntimeError::Unsupported(format!(
                    "multiple test cases have active self-ingress for activation {activation_id}"
                )));
            }
            let Some(state) = matching.first() else {
                if active.is_empty() {
                    return Ok(None);
                }
                return Err(RuntimeError::Unsupported(
                    "active test self-ingress belongs to another router session".to_string(),
                ));
            };
            state.capability.clone()
        };
        self.begin_derived(&capability.0, router_session_id, request_id)
            .map(Some)
    }
}

fn next_request_identity(
    owner: &mut TestCaseRegistryOwner,
    request_id: &str,
) -> Result<TestRequestIdentity> {
    owner.next_request_generation =
        owner
            .next_request_generation
            .checked_add(1)
            .ok_or_else(|| {
                RuntimeError::Unsupported("test request generation exhausted".to_string())
            })?;
    Ok(TestRequestIdentity {
        request_id: request_id.to_string(),
        generation: owner.next_request_generation,
    })
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
        || !lifecycle.root_released
        || !lifecycle.derived_requests.is_empty()
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

    pub(crate) fn admitted_context(&self) -> TestHttpAdmittedContext {
        self.state.admitted_context(&self.state.root_request_id)
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
    identity: TestRequestIdentity,
    released: bool,
}

/// Cloneable exact revoker for one admitted test request.
///
/// A stale task may retain this after its wire request id has been reused. Exact identity matching
/// makes that late revocation a no-op instead of revoking the newer request.
#[derive(Clone)]
pub(crate) struct TestRequestRevoker {
    registry: TestCaseRegistry,
    identity: TestRequestIdentity,
}

impl TestRequestRevoker {
    pub(crate) fn revoke(&self) -> bool {
        self.registry.revoke_exact(&self.identity)
    }
}

/// Explicit authority bundle for an Eval admitted into one test case.
///
/// Keeping capability, Router connection identity, and self-ingress context together prevents
/// adapters from reconstructing authority later through an ambient request-id lookup.
#[derive(Clone)]
pub(crate) struct TestHttpAdmittedContext {
    capability: String,
    router_session: ConnectionRequestSession,
    request_id: String,
    http: Arc<TestHttpEntryState>,
}

impl TestHttpAdmittedContext {
    pub(crate) fn capability(&self) -> &str {
        &self.capability
    }

    pub(crate) fn router_session(&self) -> &ConnectionRequestSession {
        &self.router_session
    }

    #[cfg(test)]
    pub(crate) fn request_id(&self) -> &str {
        &self.request_id
    }

    pub(crate) fn self_ingress(&self) -> TestHttpSelfIngressContext {
        TestHttpSelfIngressContext {
            state: Arc::clone(&self.http),
            test_case_capability: self.capability.clone(),
            parent_request_id: self.request_id.clone(),
        }
    }
}

/// Cloneable Host-private view used to configure one Actor method's test-aware Eval context.
#[derive(Clone)]
pub(crate) struct ActorMethodTestEffectContext {
    admitted: TestHttpAdmittedContext,
    effects: TestEffectCaseContext,
}

impl ActorMethodTestEffectContext {
    pub(crate) fn effects(&self) -> TestEffectCaseContext {
        self.effects.clone()
    }

    pub(crate) fn admitted_context(&self) -> TestHttpAdmittedContext {
        self.admitted.clone()
    }
}

/// Host-private ownership for one Actor method admitted into an active test case.
///
/// The lease is acquired synchronously by the Router frame handler and then moved into the
/// session-owned Actor owner future. The future may clone `context` for Eval, but must retain this owner
/// through terminal encode/send so root finalization cannot race the Actor's terminal tail.
pub(crate) struct ActorMethodTestEffectExecution {
    context: ActorMethodTestEffectContext,
    _lease: TestCaseDerivedLease,
}

impl ActorMethodTestEffectExecution {
    fn new(capability: &str, lease: TestCaseDerivedLease) -> Self {
        debug_assert_eq!(capability, lease.state.capability.0);
        Self {
            context: ActorMethodTestEffectContext {
                admitted: lease.admitted_context(),
                effects: lease.effects(),
            },
            _lease: lease,
        }
    }

    pub(crate) fn context(&self) -> ActorMethodTestEffectContext {
        self.context.clone()
    }

    pub(crate) fn revoker(&self) -> TestRequestRevoker {
        self._lease.revoker()
    }

    pub(crate) fn revoke_exact(&self) -> bool {
        self._lease.revoker().revoke()
    }
}

impl TestCaseDerivedLease {
    pub(crate) fn effects(&self) -> TestEffectCaseContext {
        self.state.http.effects.clone()
    }

    pub(crate) fn admitted_context(&self) -> TestHttpAdmittedContext {
        self.state.admitted_context(&self.identity.request_id)
    }

    pub(crate) fn revoker(&self) -> TestRequestRevoker {
        TestRequestRevoker {
            registry: self.registry.clone(),
            identity: self.identity.clone(),
        }
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
            self.registry.release_derived(&self.state, &self.identity);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct TestHttpEntryRegistry {
    test_cases: TestCaseRegistry,
}

impl TestHttpEntryRegistry {
    pub(crate) fn disconnect_session(&self, router_session_id: &str) -> Result<()> {
        self.test_cases.disconnect_session(router_session_id)
    }

    #[cfg(test)]
    pub(crate) fn revoke_request(&self, router_session_id: &str, request_id: &str) -> bool {
        self.test_cases
            .revoke_request(router_session_id, request_id)
    }

    pub(crate) fn begin_root_case(
        &self,
        capability: &str,
        router_session_id: &str,
        request_id: String,
        activation_id: String,
        ingress_url: &str,
        deployment: ServiceDeploymentRef,
    ) -> Result<TestCaseRootLease> {
        self.test_cases.begin_root(
            capability,
            router_session_id,
            request_id,
            activation_id,
            ingress_url,
            deployment,
        )
    }

    pub(crate) fn begin_derived(
        &self,
        capability: &str,
        router_session_id: &str,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        self.test_cases
            .begin_derived(capability, router_session_id, request_id)
    }

    pub(crate) fn begin_derived_from_parent(
        &self,
        capability: &str,
        parent_request_id: &str,
        router_session_id: &str,
        request_id: String,
    ) -> Result<TestCaseDerivedLease> {
        self.test_cases.begin_derived_from_parent(
            capability,
            parent_request_id,
            router_session_id,
            request_id,
        )
    }

    pub(crate) fn begin_actor_method(
        &self,
        capability: &str,
        parent_request_id: &str,
        router_session_id: &str,
        invocation_id: String,
    ) -> Result<ActorMethodTestEffectExecution> {
        self.test_cases
            .begin_derived_from_parent(
                capability,
                parent_request_id,
                router_session_id,
                invocation_id,
            )
            .map(|lease| ActorMethodTestEffectExecution::new(capability, lease))
    }

    #[cfg(test)]
    pub(crate) fn self_ingress_for_request(
        &self,
        router_session_id: &str,
        request_id: &str,
    ) -> Option<TestHttpSelfIngressContext> {
        self.test_cases
            .self_ingress_for_request(router_session_id, request_id)
    }

    pub(crate) fn begin_nested_http(
        &self,
        activation_id: &str,
        router_session_id: &str,
        request_id: String,
    ) -> Result<Option<TestCaseDerivedLease>> {
        self.test_cases
            .begin_nested_http(activation_id, router_session_id, request_id)
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
    test_case_capability: String,
    parent_request_id: String,
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
        headers.push(json!({
            "name": TEST_CASE_CAPABILITY_HEADER,
            "value": self.test_case_capability,
        }));
        headers.push(json!({
            "name": TEST_CASE_PARENT_REQUEST_ID_HEADER,
            "value": self.parent_request_id,
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
mod tests;
