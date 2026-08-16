use std::collections::BTreeSet;
use std::fmt;

use skiff_artifact_model::{
    ContractOperationId, GatewayAdapterPlan, GatewayEntryIdentity, GatewayEntryKey,
    GatewayEntryProtocolSurface, PackageCallableId, ReceiverCallAbi,
};

use crate::{ConstantIndex, FunctionIndex, LinkedCallableSignature};

/// Exact external operation entry facts joined from the hydrated deployment
/// contract, concrete signature and referenced function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedOperationEntry {
    contract_operation_id: ContractOperationId,
    function: FunctionIndex,
    signature: LinkedCallableSignature,
    receiver: Option<LinkedOperationReceiver>,
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
            receiver: None,
        }
    }

    pub fn new_with_receiver(
        contract_operation_id: ContractOperationId,
        function: FunctionIndex,
        signature: LinkedCallableSignature,
        receiver: LinkedOperationReceiver,
    ) -> Self {
        Self {
            contract_operation_id,
            function,
            signature,
            receiver: Some(receiver),
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

    pub const fn receiver(&self) -> Option<&LinkedOperationReceiver> {
        self.receiver.as_ref()
    }
}

/// Exact const receiver bound to one provider operation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedOperationReceiver {
    constant: ConstantIndex,
    receiver_call_abi: ReceiverCallAbi,
}

impl LinkedOperationReceiver {
    pub fn new(constant: ConstantIndex, receiver_call_abi: ReceiverCallAbi) -> Self {
        Self {
            constant,
            receiver_call_abi,
        }
    }

    pub const fn constant(&self) -> ConstantIndex {
        self.constant
    }

    pub const fn receiver_call_abi(&self) -> ReceiverCallAbi {
        self.receiver_call_abi
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

/// Exact resolution of one deployment gateway callable role.
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
    CloseHandlerPlanMismatch {
        has_close_handler: bool,
        has_close_adapter_plan: bool,
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
            Self::CloseHandlerPlanMismatch {
                has_close_handler,
                has_close_adapter_plan,
            } => write!(
                formatter,
                "gateway close handler presence ({has_close_handler}) does not match close adapter plan presence ({has_close_adapter_plan})"
            ),
        }
    }
}

impl std::error::Error for LinkedGatewayEntryError {}

/// Gateway entry facts joined from the deployment and hydrated package
/// closure, retaining the exact protocol surface, adapter plans and callable
/// references.
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
        let has_close_handler = callables
            .iter()
            .any(|callable| callable.role() == LinkedGatewayCallableRole::CloseHandler);
        if has_close_handler != close_adapter_plan.is_some() {
            return Err(LinkedGatewayEntryError::CloseHandlerPlanMismatch {
                has_close_handler,
                has_close_adapter_plan: close_adapter_plan.is_some(),
            });
        }
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
