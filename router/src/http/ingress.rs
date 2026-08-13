//! Service-scoped HTTP ingress resolution.
//!
//! The resolver projects the exact HTTP binding for a trusted
//! service/version selector from the lazy-load release pointer table and the
//! deployment record it names: `(serviceId, version)` -> release pointer ->
//! `ServiceDeploymentRef` -> deployment record -> gateway entry. It no longer
//! depends on the epoch's `gateway_ingress`/`deployment_projection`
//! projections; the epoch is consumed only for its profile coordinate and the
//! request frame metadata.
//!
//! Release resolution and record reads fail closed: an unset release pointer,
//! an unreadable pointer/record, an unmatched ingress or a missing gateway
//! surface all reject the request.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use skiff_artifact_model::{
    DeploymentGatewayEntry, GatewayAdapterKind, GatewayDispatchMode, GatewayEntryKey,
    GatewayEntryProtocolSurface, GatewayProtocolSurface, IngressProtocol, IngressSelector,
    ServiceDeploymentRef,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::release::ReleaseResolver;

use super::error::HttpError;
use super::selector::ServiceDeploymentSelector;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpDispatchMode {
    Unary,
    ServerStream,
}

impl HttpDispatchMode {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Unary => "unary",
            Self::ServerStream => "serverStream",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpAdapterKind {
    TypedJson,
    RawHttp,
}

/// Typed HTTP gateway surface for one deployment gateway entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpGatewaySurface {
    pub mode: HttpDispatchMode,
    pub adapter_kind: HttpAdapterKind,
}

/// Immutable view of HTTP gateway surfaces keyed by deployment and gateway
/// entry key.
///
/// Constructed from canonical deployment gateway entries
/// (`skiff-artifact-model::DeploymentGatewayEntry`); the HTTP layer never
/// reads deployment files itself. The same gateway entry key may exist in
/// multiple deployments (for example aihub and codex-relay both publish
/// `v1ModelsGet`); surfaces are therefore deployment-scoped and resolved
/// together with the exact service selector binding.
#[derive(Debug, Clone, Default)]
pub struct HttpGatewaySurfaceView {
    surfaces: BTreeMap<(ServiceDeploymentRef, GatewayEntryKey), HttpGatewaySurface>,
}

impl HttpGatewaySurfaceView {
    pub fn from_deployment_gateway_entries(
        entries: &BTreeMap<(ServiceDeploymentRef, GatewayEntryKey), DeploymentGatewayEntry>,
    ) -> Result<Self, String> {
        let mut surfaces = BTreeMap::new();
        for ((deployment, key), entry) in entries {
            let surface = http_surface(&entry.protocol_surface, key.as_str())?;
            surfaces.insert((deployment.clone(), key.clone()), surface);
        }
        Ok(Self { surfaces })
    }

    pub fn get(
        &self,
        deployment: &ServiceDeploymentRef,
        key: &GatewayEntryKey,
    ) -> Option<&HttpGatewaySurface> {
        self.surfaces.get(&(deployment.clone(), key.clone()))
    }

    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

fn http_surface(
    surface: &GatewayEntryProtocolSurface,
    key: &str,
) -> Result<HttpGatewaySurface, String> {
    let GatewayProtocolSurface::Http(http) = &surface.protocol else {
        return Err(format!(
            "gateway entry {key} has no HTTP protocol surface in the HTTP surface view"
        ));
    };
    let mode = match http.dispatch_mode {
        GatewayDispatchMode::Unary => HttpDispatchMode::Unary,
        GatewayDispatchMode::ServerStream => HttpDispatchMode::ServerStream,
    };
    let adapter_kind = match http.adapter_kind {
        GatewayAdapterKind::TypedJson => HttpAdapterKind::TypedJson,
        GatewayAdapterKind::RawHttp => HttpAdapterKind::RawHttp,
        _ => {
            return Err(format!(
                "gateway entry {key} uses a non-HTTP adapter kind in the HTTP surface view"
            ));
        }
    };
    Ok(HttpGatewaySurface { mode, adapter_kind })
}

/// Exact HTTP ingress binding projected for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpIngressBinding {
    pub deployment: ServiceDeploymentRef,
    /// The buildId consumed by the runtime: the release-resolved deployment's
    /// artifact identity (`deployment_artifact_identity`).
    pub build_id: String,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity,
    pub mode: HttpDispatchMode,
    pub adapter_kind: HttpAdapterKind,
    pub selector: IngressSelector,
}

/// Service-scoped ingress resolution port.
pub trait HttpIngressResolver: Send + Sync {
    /// Resolves the exact HTTP binding for one service/version selector
    /// (M4: release pointer table + deployment record; no epoch).
    fn resolve(
        &self,
        selector: &ServiceDeploymentSelector,
        method: &str,
        path: &str,
    ) -> Result<HttpIngressBinding, HttpError>;

    /// Whether an explicit OPTIONS binding exists for this service/path
    /// (service-managed CORS). Failures mean automatic CORS handling.
    fn has_explicit_options_ingress(
        &self,
        selector: &ServiceDeploymentSelector,
        path: &str,
    ) -> bool;

    /// Whether any HTTP ingress binding exists for this service/path
    /// (method-agnostic), used by automatic CORS preflight handling.
    fn has_ingress_path(&self, selector: &ServiceDeploymentSelector, path: &str) -> bool;
}

/// Real resolver over the release pointer table and deployment records.
///
/// The profile is fixed at construction (router config profile); ingress
/// surface reconstruction happens from the release-resolved deployment
/// record. Without an artifact store the resolver fails closed for every
/// request.
#[derive(Clone)]
pub struct StoreHttpIngressResolver {
    profile: String,
    release: Option<Arc<dyn ReleaseResolver>>,
    artifact_store: Option<CanonicalArtifactStore>,
}

impl fmt::Debug for StoreHttpIngressResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoreHttpIngressResolver")
            .field("profile", &self.profile)
            .field("artifact_store", &self.artifact_store.is_some())
            .finish_non_exhaustive()
    }
}

impl StoreHttpIngressResolver {
    /// Builds a resolver without an artifact store: every request fails
    /// closed (release resolution is unavailable).
    pub fn new(_surfaces: Arc<HttpGatewaySurfaceView>, profile: impl Into<String>) -> Self {
        Self {
            profile: profile.into(),
            release: None,
            artifact_store: None,
        }
    }

    /// Builds a resolver over a live artifact store; the profile is used to
    /// resolve the release pointer for each request.
    pub fn new_with_live_artifact_store(
        _surfaces: Arc<HttpGatewaySurfaceView>,
        artifact_store: CanonicalArtifactStore,
        profile: impl Into<String>,
    ) -> Self {
        let release = Arc::new(crate::release::StoreReleaseResolver::new(
            artifact_store.clone(),
        ));
        Self {
            profile: profile.into(),
            release: Some(release),
            artifact_store: Some(artifact_store),
        }
    }
}

impl HttpIngressResolver for StoreHttpIngressResolver {
    fn resolve(
        &self,
        selector: &ServiceDeploymentSelector,
        method: &str,
        path: &str,
    ) -> Result<HttpIngressBinding, HttpError> {
        let method = method.trim().to_ascii_uppercase();
        let deployment = self.resolve_deployment(selector)?;
        let record = self.deployment_record(&deployment)?;
        let mut matched = record.ingress.iter().filter(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.method.as_deref() == Some(method.as_str())
                && binding.selector.path == path
        });
        let Some(binding) = matched.next() else {
            return Err(HttpError::platform(
                404,
                "AssemblyIngressNotFound",
                format!("No release ingress matches {selector} {method} {path}"),
                None,
            ));
        };
        if matched.next().is_some() {
            return Err(HttpError::internal(
                "deployment record contains duplicate exact HTTP ingress bindings",
            ));
        }
        let gateway_entry_key = binding.gateway_entry_key.clone();
        let entry = record
            .gateway_entries
            .get(&gateway_entry_key)
            .ok_or_else(|| {
                HttpError::internal(format!(
                    "release ingress references a gateway entry absent from the deployment record for {}",
                    deployment.service_id
                ))
            })?;
        let surface = http_surface(&entry.protocol_surface, gateway_entry_key.as_str())
            .map_err(HttpError::internal)?;
        if surface.mode == HttpDispatchMode::ServerStream
            && surface.adapter_kind != HttpAdapterKind::RawHttp
        {
            return Err(HttpError::platform(
                500,
                "InvalidAssemblyIngress",
                "only rawHttp bindings may use serverStream mode",
                None,
            ));
        }
        Ok(HttpIngressBinding {
            deployment: deployment.clone(),
            build_id: deployment.deployment_artifact_identity.as_str().to_string(),
            gateway_entry_key,
            gateway_entry_identity: entry.gateway_entry_identity.clone(),
            mode: surface.mode,
            adapter_kind: surface.adapter_kind,
            selector: binding.selector.clone(),
        })
    }

    fn has_explicit_options_ingress(
        &self,
        selector: &ServiceDeploymentSelector,
        path: &str,
    ) -> bool {
        self.resolve(selector, "OPTIONS", path).is_ok()
    }

    fn has_ingress_path(&self, selector: &ServiceDeploymentSelector, path: &str) -> bool {
        let Ok(deployment) = self.resolve_deployment(selector) else {
            return false;
        };
        let Ok(record) = self.deployment_record(&deployment) else {
            return false;
        };
        record.ingress.iter().any(|binding| {
            binding.selector.protocol == IngressProtocol::Http && binding.selector.path == path
        })
    }
}

impl StoreHttpIngressResolver {
    fn resolve_deployment(
        &self,
        selector: &ServiceDeploymentSelector,
    ) -> Result<ServiceDeploymentRef, HttpError> {
        let release = self.release.as_ref().ok_or_else(|| {
            HttpError::internal("HTTP ingress resolver has no release resolver configured")
        })?;
        match release.resolve(
            &self.profile,
            &selector.service_id,
            &selector.contract_version,
        ) {
            Ok(Some(deployment)) => Ok(deployment),
            Ok(None) => Err(HttpError::platform(
                404,
                "ReleaseNotFound",
                format!("No release pointer resolves {selector}"),
                None,
            )),
            Err(message) => Err(HttpError::internal(format!(
                "release resolution failed for {selector}: {message}"
            ))),
        }
    }

    fn deployment_record(
        &self,
        deployment: &ServiceDeploymentRef,
    ) -> Result<Arc<skiff_artifact_model::ServiceDeployment>, HttpError> {
        let store = self.artifact_store.as_ref().ok_or_else(|| {
            HttpError::internal("HTTP ingress resolver has no artifact store configured")
        })?;
        store.read_service_deployment(deployment).map_err(|error| {
            HttpError::internal(format!(
                "read deployment record {} for HTTP ingress: {error}",
                deployment.service_id
            ))
        })
    }
}

/// Builds the HTTP surface view from the deployment records referenced by
/// the release pointer table (M4: pointer-table scan).
pub(crate) fn http_surface_view_from_pointers(
    artifact_store: &CanonicalArtifactStore,
    profile: &str,
) -> Result<HttpGatewaySurfaceView, String> {
    let release = crate::release::StoreReleaseResolver::new(artifact_store.clone());
    let mut entries = BTreeMap::new();
    for deployment in release
        .all_deployments(profile)
        .map_err(|error| error.to_string())?
    {
        let record = artifact_store
            .read_service_deployment(&deployment)
            .map_err(|error| {
                format!(
                    "read deployment record {} for HTTP surface: {error}",
                    deployment.service_id
                )
            })?;
        for (key, entry) in &record.gateway_entries {
            if !matches!(
                entry.protocol_surface.protocol,
                GatewayProtocolSurface::Http(_)
            ) {
                continue;
            }
            entries.insert((deployment.clone(), key.clone()), entry.clone());
        }
    }
    HttpGatewaySurfaceView::from_deployment_gateway_entries(&entries)
}
