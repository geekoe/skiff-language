use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    GatewayAdapterPlan, GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayIngressBinding, OperationCallableKind, OperationTargetRef, PackageArtifact,
    PackageCallableId, PackageCallableSignature, PackageLocalAbiSymbol, ServiceDeployment,
    ServiceDeploymentRef, ServiceIngressKey,
};

use super::HydratedPackageCodeSlot;

#[derive(Debug, Clone, PartialEq)]
pub struct HydratedGatewayCallable {
    callable_id: PackageCallableId,
    target: OperationTargetRef,
    signature: PackageCallableSignature,
}

impl HydratedGatewayCallable {
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

#[derive(Debug, Clone, PartialEq)]
pub struct HydratedGatewayEntry {
    owner: ServiceDeploymentRef,
    gateway_entry_key: GatewayEntryKey,
    gateway_entry_identity: GatewayEntryIdentity,
    protocol_surface: GatewayEntryProtocolSurface,
    adapter_plan: GatewayAdapterPlan,
    handler: Option<HydratedGatewayCallable>,
    pre: Option<HydratedGatewayCallable>,
    guard: Option<HydratedGatewayCallable>,
}

impl HydratedGatewayEntry {
    pub fn owner(&self) -> &ServiceDeploymentRef {
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

    pub fn handler(&self) -> Option<&HydratedGatewayCallable> {
        self.handler.as_ref()
    }

    pub fn pre(&self) -> Option<&HydratedGatewayCallable> {
        self.pre.as_ref()
    }

    pub fn guard(&self) -> Option<&HydratedGatewayCallable> {
        self.guard.as_ref()
    }
}

pub(super) struct HydratedGatewayIngress {
    pub(super) entries:
        BTreeMap<(ServiceDeploymentRef, GatewayEntryKey), Arc<HydratedGatewayEntry>>,
    pub(super) selectors: BTreeMap<ServiceIngressKey, Arc<HydratedGatewayEntry>>,
}

pub(super) fn hydrate_gateway_ingress(
    assembly_bindings: &[GatewayIngressBinding],
    deployments: &BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
    packages: &[HydratedPackageCodeSlot],
) -> anyhow::Result<HydratedGatewayIngress> {
    let packages = packages
        .iter()
        .map(|slot| (slot.reference().package_build_id.clone(), slot))
        .collect::<BTreeMap<_, _>>();
    let mut entries = BTreeMap::new();
    let mut expected_bindings = BTreeMap::new();

    for (owner, deployment) in deployments {
        let implementation = packages
            .get(&deployment.implementation.package_build_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "gateway activation {owner:?} targets missing implementation package {}",
                    deployment.implementation.package_build_id
                )
            })?;
        if implementation.reference() != &deployment.implementation {
            anyhow::bail!(
                "gateway activation {owner:?} implementation package reference is not exact"
            );
        }

        for (key, source) in &deployment.gateway_entries {
            let entry = Arc::new(HydratedGatewayEntry {
                owner: owner.clone(),
                gateway_entry_key: key.clone(),
                gateway_entry_identity: source.gateway_entry_identity.clone(),
                protocol_surface: source.protocol_surface.clone(),
                adapter_plan: source.adapter_plan.clone(),
                handler: source
                    .handler
                    .as_ref()
                    .map(|callable| {
                        hydrate_callable(owner, key, "handler", implementation.artifact(), callable)
                    })
                    .transpose()?,
                pre: source
                    .pre
                    .as_ref()
                    .map(|callable| {
                        hydrate_callable(owner, key, "pre", implementation.artifact(), callable)
                    })
                    .transpose()?,
                guard: source
                    .guard
                    .as_ref()
                    .map(|callable| {
                        hydrate_callable(owner, key, "guard", implementation.artifact(), callable)
                    })
                    .transpose()?,
            });
            if entries
                .insert((owner.clone(), key.clone()), entry)
                .is_some()
            {
                anyhow::bail!("gateway entry {owner:?}/{key} is duplicated");
            }
        }

        for binding in &deployment.ingress {
            let entry = deployment
                .gateway_entries
                .get(&binding.gateway_entry_key)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "gateway selector {:?} references missing entry {} in {owner:?}",
                        binding.selector,
                        binding.gateway_entry_key
                    )
                })?;
            let expected = GatewayIngressBinding {
                selector: binding.selector.clone(),
                deployment: owner.clone(),
                gateway_entry_key: binding.gateway_entry_key.clone(),
                gateway_entry_identity: entry.gateway_entry_identity.clone(),
            };
            let key = expected.service_ingress_key();
            if let Some(first) = expected_bindings.insert(key.clone(), expected) {
                anyhow::bail!(
                    "gateway ingress key {key:?} is declared by both {:?} and {owner:?}",
                    first.deployment
                );
            }
        }
    }

    let mut declared_bindings = BTreeMap::new();
    for binding in assembly_bindings {
        let key = binding.service_ingress_key();
        if declared_bindings
            .insert(key.clone(), binding.clone())
            .is_some()
        {
            anyhow::bail!("RuntimeAssembly repeats gateway ingress key {key:?}");
        }
    }
    if declared_bindings != expected_bindings {
        anyhow::bail!(
            "RuntimeAssembly gatewayIngress does not exactly match hydrated deployment ingress"
        );
    }

    let mut selectors = BTreeMap::new();
    for (key, binding) in declared_bindings {
        let entry = entries
            .get(&(
                binding.deployment.clone(),
                binding.gateway_entry_key.clone(),
            ))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "RuntimeAssembly gateway ingress key {key:?} targets missing entry {:?}/{}",
                    binding.deployment,
                    binding.gateway_entry_key
                )
            })?;
        if entry.gateway_entry_identity() != &binding.gateway_entry_identity {
            anyhow::bail!(
                "RuntimeAssembly gateway ingress key {key:?} entry identity mismatches deployment"
            );
        }
        selectors.insert(key, Arc::clone(entry));
    }

    Ok(HydratedGatewayIngress { entries, selectors })
}

fn hydrate_callable(
    owner: &ServiceDeploymentRef,
    key: &GatewayEntryKey,
    role: &str,
    implementation: &PackageArtifact,
    callable_id: &PackageCallableId,
) -> anyhow::Result<HydratedGatewayCallable> {
    let link = implementation.callable_links.get(callable_id).ok_or_else(|| {
        anyhow::anyhow!(
            "gateway entry {owner:?}/{key} {role} callable {callable_id} is missing from implementation package {}",
            implementation.package_build_id
        )
    })?;
    if &link.callable_id != callable_id {
        anyhow::bail!(
            "gateway entry {owner:?}/{key} {role} callable link key {callable_id} mismatches nested id {}",
            link.callable_id
        );
    }
    if link.target.callable_abi_id != callable_id.as_str()
        || link.target.callable_kind != OperationCallableKind::InternalFunction
    {
        anyhow::bail!(
            "gateway entry {owner:?}/{key} {role} callable {callable_id} has a mismatched implementation target"
        );
    }

    let mut signatures = implementation
        .package_local_abi
        .implementation_symbols
        .values()
        .filter_map(|symbol| match symbol {
            PackageLocalAbiSymbol::Callable {
                callable_id: nested_id,
                signature,
            } if nested_id == callable_id => Some(signature),
            _ => None,
        });
    let signature = signatures.next().ok_or_else(|| {
        anyhow::anyhow!(
            "gateway entry {owner:?}/{key} {role} callable {callable_id} has no exact implementation Local ABI signature"
        )
    })?;
    if signatures.next().is_some() {
        anyhow::bail!(
            "gateway entry {owner:?}/{key} {role} callable {callable_id} has ambiguous implementation Local ABI signatures"
        );
    }

    Ok(HydratedGatewayCallable {
        callable_id: callable_id.clone(),
        target: link.target.clone(),
        signature: signature.clone(),
    })
}
