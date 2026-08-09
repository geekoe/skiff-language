use std::collections::BTreeSet;
use std::fmt;

use skiff_artifact_model::{
    ContractOperationId, GatewayAdapterPlan, GatewayEntryIdentity, GatewayEntryKey,
    GatewayEntryProtocolSurface, PackageCallableId,
};

use crate::{FunctionIndex, LinkedCallableSignature};

/// Unverified external operation entry facts. The verifier must independently
/// compare the operation contract, concrete signature and referenced function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedOperationEntry {
    contract_operation_id: ContractOperationId,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl LinkedOperationEntry {
    pub fn new(
        contract_operation_id: ContractOperationId,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            contract_operation_id,
            function,
            signature,
        }
    }

    pub const fn contract_operation_id(&self) -> &ContractOperationId {
        &self.contract_operation_id
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

/// Typed role of one optional callable in a deployment gateway entry.
///
/// Declaration order is the canonical order used by [`LinkedGatewayEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LinkedGatewayCallableRole {
    Handler,
    Pre,
    Guard,
    CloseHandler,
}

/// Unverified resolution of one deployment gateway callable role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedGatewayCallable {
    role: LinkedGatewayCallableRole,
    package_callable_id: PackageCallableId,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
}

impl LinkedGatewayCallable {
    pub fn new(
        role: LinkedGatewayCallableRole,
        package_callable_id: PackageCallableId,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
    ) -> Self {
        Self {
            role,
            package_callable_id,
            function,
            signature,
        }
    }

    pub const fn role(&self) -> LinkedGatewayCallableRole {
        self.role
    }

    pub const fn package_callable_id(&self) -> &PackageCallableId {
        &self.package_callable_id
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }
}

/// Local shape error in a gateway entry's canonical callable-role mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedGatewayEntryError {
    DuplicateCallableRole {
        role: LinkedGatewayCallableRole,
    },
    NonCanonicalCallableRoleOrder {
        previous: LinkedGatewayCallableRole,
        current: LinkedGatewayCallableRole,
    },
}

impl fmt::Display for LinkedGatewayEntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateCallableRole { role } => {
                write!(
                    formatter,
                    "gateway callable role {role:?} appears more than once"
                )
            }
            Self::NonCanonicalCallableRoleOrder { previous, current } => write!(
                formatter,
                "gateway callable role {current:?} must sort after {previous:?}"
            ),
        }
    }
}

impl std::error::Error for LinkedGatewayEntryError {}

/// Unverified gateway entry facts copied from the deployment and hydrated
/// package closure. The verifier must independently compare the protocol
/// surface, both adapter plans, every role's callable identity, concrete
/// signature and referenced function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedGatewayEntry {
    gateway_entry_key: GatewayEntryKey,
    gateway_entry_identity: GatewayEntryIdentity,
    protocol_surface: GatewayEntryProtocolSurface,
    callables: Box<[LinkedGatewayCallable]>,
    adapter_plan: GatewayAdapterPlan,
    close_adapter_plan: Option<GatewayAdapterPlan>,
}

impl LinkedGatewayEntry {
    pub fn try_new(
        gateway_entry_key: GatewayEntryKey,
        gateway_entry_identity: GatewayEntryIdentity,
        protocol_surface: GatewayEntryProtocolSurface,
        callables: Box<[LinkedGatewayCallable]>,
        adapter_plan: GatewayAdapterPlan,
        close_adapter_plan: Option<GatewayAdapterPlan>,
    ) -> Result<Self, LinkedGatewayEntryError> {
        validate_callable_order(&callables)?;
        Ok(Self {
            gateway_entry_key,
            gateway_entry_identity,
            protocol_surface,
            callables,
            adapter_plan,
            close_adapter_plan,
        })
    }

    pub const fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.gateway_entry_key
    }

    pub const fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        &self.gateway_entry_identity
    }

    pub const fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        &self.protocol_surface
    }

    pub fn callables(&self) -> &[LinkedGatewayCallable] {
        &self.callables
    }

    pub fn callable(&self, role: LinkedGatewayCallableRole) -> Option<&LinkedGatewayCallable> {
        self.callables.iter().find(|callable| callable.role == role)
    }

    pub fn handler(&self) -> Option<&LinkedGatewayCallable> {
        self.callable(LinkedGatewayCallableRole::Handler)
    }

    pub fn pre(&self) -> Option<&LinkedGatewayCallable> {
        self.callable(LinkedGatewayCallableRole::Pre)
    }

    pub fn guard(&self) -> Option<&LinkedGatewayCallable> {
        self.callable(LinkedGatewayCallableRole::Guard)
    }

    pub fn close_handler(&self) -> Option<&LinkedGatewayCallable> {
        self.callable(LinkedGatewayCallableRole::CloseHandler)
    }

    pub const fn adapter_plan(&self) -> &GatewayAdapterPlan {
        &self.adapter_plan
    }

    pub const fn close_adapter_plan(&self) -> Option<&GatewayAdapterPlan> {
        self.close_adapter_plan.as_ref()
    }
}

fn validate_callable_order(
    callables: &[LinkedGatewayCallable],
) -> Result<(), LinkedGatewayEntryError> {
    let mut seen = BTreeSet::new();
    let mut previous = None;
    for callable in callables {
        let current = callable.role();
        if !seen.insert(current) {
            return Err(LinkedGatewayEntryError::DuplicateCallableRole { role: current });
        }
        if let Some(previous) = previous {
            if current < previous {
                return Err(LinkedGatewayEntryError::NonCanonicalCallableRoleOrder {
                    previous,
                    current,
                });
            }
        }
        previous = Some(current);
    }
    Ok(())
}
