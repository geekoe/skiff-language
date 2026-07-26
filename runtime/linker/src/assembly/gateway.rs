use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    GatewayAdapterPlan, GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    IngressSelector, OperationTargetRef, PackageCallableId, PackageCallableSignature,
    ServiceDeploymentRef,
};
use skiff_runtime_linked_program::SharedPackageLinkedImage;
use skiff_runtime_loader::{
    HydratedGatewayCallable, HydratedGatewayEntry, HydratedRuntimeAssembly,
};

use super::LinkedActivationTemplate;

#[derive(Debug, Clone, PartialEq)]
pub struct LinkedGatewayCallable {
    callable_id: PackageCallableId,
    target: OperationTargetRef,
    signature: PackageCallableSignature,
}

impl LinkedGatewayCallable {
    pub fn callable_id(&self) -> &PackageCallableId {
        &self.callable_id
    }

    pub fn target(&self) -> &OperationTargetRef {
        &self.target
    }

    pub fn signature(&self) -> &PackageCallableSignature {
        &self.signature
    }
}

impl From<&HydratedGatewayCallable> for LinkedGatewayCallable {
    fn from(source: &HydratedGatewayCallable) -> Self {
        Self {
            callable_id: source.callable_id().clone(),
            target: source.target().clone(),
            signature: source.signature().clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinkedGatewayEntry {
    owner: ServiceDeploymentRef,
    gateway_entry_key: GatewayEntryKey,
    gateway_entry_identity: GatewayEntryIdentity,
    protocol_surface: GatewayEntryProtocolSurface,
    adapter_plan: GatewayAdapterPlan,
    handler: LinkedGatewayCallable,
    pre: Option<LinkedGatewayCallable>,
    guard: Option<LinkedGatewayCallable>,
}

impl LinkedGatewayEntry {
    pub(super) fn from_hydrated(source: &HydratedGatewayEntry) -> Self {
        Self {
            owner: source.owner().clone(),
            gateway_entry_key: source.gateway_entry_key().clone(),
            gateway_entry_identity: source.gateway_entry_identity().clone(),
            protocol_surface: source.protocol_surface().clone(),
            adapter_plan: source.adapter_plan().clone(),
            handler: source.handler().into(),
            pre: source.pre().map(Into::into),
            guard: source.guard().map(Into::into),
        }
    }

    pub fn owner(&self) -> &ServiceDeploymentRef {
        &self.owner
    }

    pub fn activation(&self) -> &ServiceDeploymentRef {
        &self.owner
    }

    pub fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.gateway_entry_key
    }

    pub fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        &self.gateway_entry_identity
    }

    pub fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        &self.protocol_surface
    }

    pub fn adapter_plan(&self) -> &GatewayAdapterPlan {
        &self.adapter_plan
    }

    pub fn handler(&self) -> &LinkedGatewayCallable {
        &self.handler
    }

    pub fn pre(&self) -> Option<&LinkedGatewayCallable> {
        self.pre.as_ref()
    }

    pub fn guard(&self) -> Option<&LinkedGatewayCallable> {
        self.guard.as_ref()
    }
}

pub(super) fn link_gateway_ingress(
    hydrated: &HydratedRuntimeAssembly,
    activations: &BTreeMap<ServiceDeploymentRef, LinkedActivationTemplate>,
    image: &SharedPackageLinkedImage,
) -> anyhow::Result<(
    BTreeMap<(ServiceDeploymentRef, GatewayEntryKey), Arc<LinkedGatewayEntry>>,
    BTreeMap<IngressSelector, Arc<LinkedGatewayEntry>>,
)> {
    let mut entries = BTreeMap::new();
    for ((owner, key), source) in hydrated.gateway_entries() {
        let activation = activations
            .get(owner)
            .ok_or_else(|| anyhow::anyhow!("gateway entry {owner:?}/{key} has no activation"))?;
        if source.owner() != owner
            || source.gateway_entry_key() != key
            || activation.deployment().gateway_entries.get(key).is_none()
        {
            anyhow::bail!("gateway entry {owner:?}/{key} mismatches its activation declaration");
        }
        let implementation = image
            .code_by_build(activation.implementation_package_build_id())
            .ok_or_else(|| {
                anyhow::anyhow!("gateway entry {owner:?}/{key} implementation is not linked")
            })?;
        validate_gateway_callable(owner, key, "handler", implementation, source.handler())?;
        if let Some(callable) = source.pre() {
            validate_gateway_callable(owner, key, "pre", implementation, callable)?;
        }
        if let Some(callable) = source.guard() {
            validate_gateway_callable(owner, key, "guard", implementation, callable)?;
        }
        if entries
            .insert(
                (owner.clone(), key.clone()),
                Arc::new(LinkedGatewayEntry::from_hydrated(source)),
            )
            .is_some()
        {
            anyhow::bail!("linked gateway entry {owner:?}/{key} is duplicated");
        }
    }

    let mut ingress = BTreeMap::new();
    for (selector, source) in hydrated.gateway_ingress() {
        let key = (source.owner().clone(), source.gateway_entry_key().clone());
        let entry = entries.get(&key).ok_or_else(|| {
            anyhow::anyhow!(
                "gateway selector {selector:?} targets missing linked entry {:?}/{}",
                key.0,
                key.1
            )
        })?;
        if entry.gateway_entry_identity() != source.gateway_entry_identity() {
            anyhow::bail!("gateway selector {selector:?} linked entry identity mismatch");
        }
        if ingress
            .insert(selector.clone(), Arc::clone(entry))
            .is_some()
        {
            anyhow::bail!("linked gateway selector {selector:?} is duplicated");
        }
    }

    let declared = hydrated
        .assembly()
        .gateway_ingress
        .iter()
        .map(|binding| binding.selector.clone())
        .collect::<BTreeSet<_>>();
    if declared.len() != hydrated.assembly().gateway_ingress.len()
        || declared != ingress.keys().cloned().collect()
    {
        anyhow::bail!("linked gateway ingress does not exactly match RuntimeAssembly declaration");
    }
    Ok((entries, ingress))
}

fn validate_gateway_callable(
    owner: &ServiceDeploymentRef,
    key: &GatewayEntryKey,
    role: &str,
    implementation: &skiff_runtime_linked_program::SharedPackageCode,
    callable: &HydratedGatewayCallable,
) -> anyhow::Result<()> {
    let target = implementation
        .callable_target(callable.callable_id())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "gateway entry {owner:?}/{key} {role} callable {} is absent from linked implementation package",
                callable.callable_id()
            )
        })?;
    if target != callable.target()
        || target.callable_abi_id != callable.callable_id().as_str()
        || implementation
            .artifact()
            .callable_links
            .get(callable.callable_id())
            .map(|link| (&link.callable_id, &link.target))
            != Some((callable.callable_id(), callable.target()))
    {
        anyhow::bail!(
            "gateway entry {owner:?}/{key} {role} callable {} target is not exact",
            callable.callable_id()
        );
    }
    Ok(())
}
