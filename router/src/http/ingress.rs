//! Service-scoped HTTP ingress resolution.
//!
//! The resolver consumes one captured whole `RoutingEpoch` (plan §3.3) and
//! projects the exact HTTP binding for a trusted service/version selector.
//! The epoch's immutable `gateway_ingress` projection carries
//! selector/deployment/gateway-entry identity but not the HTTP surface
//! (operation mode / adapter kind); that surface is supplied as the typed
//! `HttpGatewaySurfaceView` seam (see the W-http leaf). E-bootstrap/E-http
//! wiring later promotes the surface view into the production composition.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use skiff_artifact_model::{
    DeploymentGatewayEntry, GatewayAdapterKind, GatewayDispatchMode, GatewayEntryKey,
    GatewayEntryProtocolSurface, GatewayProtocolSurface, IngressProtocol, IngressSelector,
    ServiceDeploymentRef,
};
use skiff_deployment::storage::CanonicalArtifactStore;

use crate::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};

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
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity,
    pub mode: HttpDispatchMode,
    pub adapter_kind: HttpAdapterKind,
    pub selector: IngressSelector,
}

/// Service-scoped ingress resolution port.
pub trait HttpIngressResolver: Send + Sync {
    /// Resolves the exact HTTP binding for a captured whole epoch.
    fn resolve(
        &self,
        epoch: &Arc<RoutingEpoch>,
        selector: &ServiceDeploymentSelector,
        method: &str,
        path: &str,
    ) -> Result<HttpIngressBinding, HttpError>;

    /// Whether an explicit OPTIONS binding exists for this service/path
    /// (service-managed CORS). Failures mean automatic CORS handling.
    fn has_explicit_options_ingress(
        &self,
        epoch: &Arc<RoutingEpoch>,
        selector: &ServiceDeploymentSelector,
        path: &str,
    ) -> bool;
}

/// Real resolver over a captured `RoutingEpoch` and the HTTP surface view.
#[derive(Clone)]
pub struct EpochHttpIngressResolver {
    surfaces: Arc<HttpGatewaySurfaceView>,
    /// E-activation §8: when present, every resolve rebuilds the HTTP
    /// surface from the live active epoch (activation swap visibility);
    /// absent keeps the static captured surface.
    live: Option<LiveHttpSurfaceSource>,
}

impl fmt::Debug for EpochHttpIngressResolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EpochHttpIngressResolver")
            .field("live", &self.live.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct LiveHttpSurfaceSource {
    epoch_store: Arc<ActiveRoutingEpochStore>,
    artifact_store: CanonicalArtifactStore,
}

impl EpochHttpIngressResolver {
    pub fn new(surfaces: Arc<HttpGatewaySurfaceView>) -> Self {
        Self {
            surfaces,
            live: None,
        }
    }

    pub fn new_with_epoch_store(
        surfaces: Arc<HttpGatewaySurfaceView>,
        epoch_store: Arc<ActiveRoutingEpochStore>,
        artifact_store: CanonicalArtifactStore,
    ) -> Self {
        Self {
            surfaces,
            live: Some(LiveHttpSurfaceSource {
                epoch_store,
                artifact_store,
            }),
        }
    }

    pub fn surfaces(&self) -> &Arc<HttpGatewaySurfaceView> {
        &self.surfaces
    }

    fn current_surfaces(&self) -> Result<Arc<HttpGatewaySurfaceView>, HttpError> {
        let Some(live) = &self.live else {
            return Ok(Arc::clone(&self.surfaces));
        };
        let Some(current) = live.epoch_store.capture() else {
            return Ok(Arc::clone(&self.surfaces));
        };
        http_surface_view_from_epoch(&live.artifact_store, &current)
            .map(Arc::new)
            .map_err(|message| HttpError::internal(message))
    }
}

impl HttpIngressResolver for EpochHttpIngressResolver {
    fn resolve(
        &self,
        epoch: &Arc<RoutingEpoch>,
        selector: &ServiceDeploymentSelector,
        method: &str,
        path: &str,
    ) -> Result<HttpIngressBinding, HttpError> {
        let method = method.trim().to_ascii_uppercase();
        let matches = epoch.ingress_projection().iter().filter(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.method.as_deref() == Some(method.as_str())
                && binding.selector.path == path
                && binding.deployment.service_id == selector.service_id
                && binding.deployment.contract_version == selector.contract_version
        });
        let mut matched = matches;
        let Some(binding) = matched.next() else {
            return Err(HttpError::platform(
                404,
                "AssemblyIngressNotFound",
                format!("No committed RuntimeAssembly ingress matches {selector} {method} {path}"),
                None,
            ));
        };
        if matched.next().is_some() {
            return Err(HttpError::internal(
                "RuntimeAssembly ingress projection contains duplicate exact HTTP bindings",
            ));
        }
        if !epoch
            .deployment_projection()
            .iter()
            .any(|deployment| deployment == &binding.deployment)
        {
            return Err(HttpError::internal(
                "RuntimeAssembly ingress references a deployment absent from the epoch projection",
            ));
        }
        let surfaces = self.current_surfaces()?;
        let surface = surfaces
            .get(&binding.deployment, &binding.gateway_entry_key)
            .ok_or_else(|| {
                HttpError::internal(format!(
                    "RuntimeAssembly HTTP ingress is missing its gateway surface for {} {}",
                    binding.deployment.service_id,
                    binding.gateway_entry_key.as_str()
                ))
            })?;
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
            deployment: binding.deployment.clone(),
            gateway_entry_key: binding.gateway_entry_key.clone(),
            gateway_entry_identity: binding.gateway_entry_identity.clone(),
            mode: surface.mode,
            adapter_kind: surface.adapter_kind,
            selector: binding.selector.clone(),
        })
    }

    fn has_explicit_options_ingress(
        &self,
        epoch: &Arc<RoutingEpoch>,
        selector: &ServiceDeploymentSelector,
        path: &str,
    ) -> bool {
        self.resolve(epoch, selector, "OPTIONS", path).is_ok()
    }
}

/// Builds the HTTP surface view from one epoch's resolved deployment
/// records (E-activation §8 live-surface rebuild after an activation swap).
fn http_surface_view_from_epoch(
    artifact_store: &CanonicalArtifactStore,
    epoch: &RoutingEpoch,
) -> Result<HttpGatewaySurfaceView, String> {
    let mut entries = BTreeMap::new();
    for deployment in epoch.deployment_projection() {
        let record = artifact_store
            .read_service_deployment(deployment)
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
