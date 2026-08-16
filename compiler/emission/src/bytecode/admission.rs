use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorDeclarationIr, ActorImplementationIdentity, ActorMethodIdentity,
    AssignTargetIr, BinaryOpIr, BoxSourceIr, CallIr, CallTargetIr, CallableEffectSummary,
    CallbackInterfaceMethodIr, DbBodyIr, DbOpKindIr, DbTargetIr, ExprIr, ExprRefIr,
    FunctionTypeParamIr, InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr, LiteralIr,
    MetadataValue, NamedUnionBranchIr, NativeTarget, ReceiverCallAbi, ServiceBoundaryPlan,
    ServiceCallRef, ServiceSymbolRef, StatementAttributionId, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirEmissionAnchor, MirExecutableKind, MirFunction, MirParamMode, MirSlotKind,
    MirStmtKind, MirUnit, MirWritableRoot,
};

use super::{
    carriers::{
        analyze_machine_carriers, may_share_scalar_machine_carrier, PackageMachineCarrierFacts,
    },
    inputs::canonical_function_key,
    BytecodeEmissionError, Phase1MirFactMismatch, Phase1UnsupportedCapability,
};

mod gateway_parameter;
mod host_effects;
mod package_type_authority;
mod representation_carrier;
mod server_stream;

pub(crate) use gateway_parameter::DenseParameterMaterializationFact;
use host_effects::{HostEffectAdmissions, RegistryValueAuthority};
pub(crate) use representation_carrier::RepresentationCarrierFact;
use server_stream::ServerStreamAdmissions;

pub use gateway_parameter::GatewayParameterAuthority;
pub use server_stream::{ServerStreamEmitFact, ServerStreamGatewayAuthority};

const TASK_SUBMIT_METADATA_KEY: &str = "dispatchSubmit";

const CANONICAL_DURATION_MILLISECONDS_BINDING_KEY: &str = "core.duration.milliseconds";

/// Opaque proof that one exact MIR slice passed the Phase 1 bytecode boundary.
///
/// The proof cannot be constructed. Public planning and emission entry points
/// therefore accept only source facts checked by
/// [`admit_phase_1_bytecode_mir`]. The one public MIR view is read-only and is
/// scoped to source value-transfer fact projection, so downstream planning
/// consumes this normalized carrier instead of the pre-admission input.
#[derive(Debug)]
pub struct AdmittedPhase1BytecodeMir {
    units: Vec<MirUnit>,
    dense_parameter_materializations: BTreeMap<String, DenseParameterMaterializationFact>,
    machine_carriers: PackageMachineCarrierFacts,
    representation_carriers: Vec<RepresentationCarrierFact>,
    service_boundary_plans: BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
    local_interface_tables: LocalInterfaceFacts,
}

impl AdmittedPhase1BytecodeMir {
    pub(crate) fn units(&self) -> &[MirUnit] {
        &self.units
    }

    pub(crate) fn dense_parameter_materializations(
        &self,
    ) -> &BTreeMap<String, DenseParameterMaterializationFact> {
        &self.dense_parameter_materializations
    }

    pub(crate) fn machine_carriers(&self) -> &PackageMachineCarrierFacts {
        &self.machine_carriers
    }

    pub(crate) fn representation_carriers(&self) -> &[RepresentationCarrierFact] {
        &self.representation_carriers
    }

    pub(crate) fn service_boundary_plans(&self) -> &BTreeMap<ServiceCallRef, ServiceBoundaryPlan> {
        &self.service_boundary_plans
    }

    pub(crate) fn local_interface_tables(&self) -> &LocalInterfaceFacts {
        &self.local_interface_tables
    }

    /// Returns the normalized, admitted MIR used to project source-owned
    /// value-transfer facts.
    pub fn source_value_transfer_units(&self) -> &[MirUnit] {
        &self.units
    }
}

/// One exact compiler-owned local interface method row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalInterfaceMethodFact {
    pub(crate) slot: u32,
    pub(crate) method_name: String,
    pub(crate) method_abi_id: String,
    pub(crate) signature: InterfaceMethodSlotSignatureIr,
    pub(crate) effects: CallableEffectSummary,
    pub(crate) executable_index: u32,
    pub(crate) function_key: String,
    pub(crate) receiver_call_abi: ReceiverCallAbi,
}

/// One exact local interface table emitted from `InterfaceBox` source facts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalInterfaceTableFact {
    pub(crate) interface: InterfaceInstantiationRef,
    pub(crate) concrete_type: TypeRefIr,
    pub(crate) methods: Vec<LocalInterfaceMethodFact>,
}

#[derive(Debug, Clone)]
pub(crate) struct LocalInterfaceFacts {
    tables: Vec<LocalInterfaceTableFact>,
    provider_receiver_constants: BTreeMap<(String, String), TypeRefIr>,
    provider_conformances: Vec<(String, TypeRefIr, InterfaceInstantiationRef)>,
    provider_method_functions: BTreeMap<String, BTreeSet<u32>>,
}

impl LocalInterfaceFacts {
    pub(crate) fn empty() -> Self {
        Self {
            tables: Vec::new(),
            provider_receiver_constants: BTreeMap::new(),
            provider_conformances: Vec::new(),
            provider_method_functions: BTreeMap::new(),
        }
    }

    pub(crate) fn tables(&self) -> &[LocalInterfaceTableFact] {
        &self.tables
    }

    pub(crate) fn table(
        &self,
        interface: &InterfaceInstantiationRef,
        concrete_type: &TypeRefIr,
    ) -> Option<&LocalInterfaceTableFact> {
        self.tables
            .iter()
            .find(|table| &table.interface == interface && &table.concrete_type == concrete_type)
    }

    pub(crate) fn method_for_executable(
        &self,
        executable_index: u32,
    ) -> Option<&LocalInterfaceMethodFact> {
        self.tables
            .iter()
            .flat_map(|table| table.methods.iter())
            .find(|method| method.executable_index == executable_index)
    }

    pub(crate) fn concrete_type(&self, ty: &TypeRefIr) -> bool {
        self.tables.iter().any(|table| table.concrete_type == *ty)
    }

    pub(crate) fn provider_receiver_constant(
        &self,
        module_path: &str,
        constant: &skiff_compiler_lowering::mir::MirConst,
    ) -> bool {
        self.provider_receiver_constants
            .get(&(module_path.to_string(), constant.symbol.clone()))
            .is_some_and(|ty| ty == &constant.ty)
    }

    pub(crate) fn provider_conformance(
        &self,
        unit: &MirUnit,
        ty: &TypeRefIr,
        interface: &InterfaceInstantiationRef,
    ) -> bool {
        self.provider_conformances.iter().any(|row| {
            row.0 == unit.module_path
                && &row.1 == ty
                && local_interface_declaration_identity(unit, &row.2)
                    == local_interface_declaration_identity(unit, interface)
        })
    }

    pub(crate) fn provider_method_for_executable(
        &self,
        module_path: &str,
        executable_index: u32,
    ) -> bool {
        self.provider_method_functions
            .get(module_path)
            .is_some_and(|functions| functions.contains(&executable_index))
    }

    pub(crate) fn add_provider_public_instances(
        &mut self,
        facts: &ProviderPublicInstanceFacts,
    ) -> Result<(), BytecodeEmissionError> {
        let mut receiver_constants = BTreeMap::new();
        let mut conformances = Vec::new();
        let mut method_functions = BTreeMap::<String, BTreeSet<u32>>::new();
        for root in &facts.roots {
            if root.const_module.is_empty() || root.const_symbol.is_empty() {
                return Err(BytecodeEmissionError::UnsupportedConstruct {
                    function_key: String::new(),
                    construct: "provider public-instance facts",
                    location: format!(
                        " public instance `{}` has an empty receiver constant identity",
                        root.public_root
                    ),
                });
            }
            let receiver_key = (
                root.const_module.clone(),
                format!("{}.{}", root.const_module, root.const_symbol),
            );
            if receiver_constants
                .insert(receiver_key.clone(), root.receiver_type.clone())
                .is_some()
            {
                return Err(BytecodeEmissionError::UnsupportedConstruct {
                    function_key: String::new(),
                    construct: "provider public-instance facts",
                    location: format!(
                        " duplicate receiver constant `{}` for public instance `{}`",
                        receiver_key.1, root.public_root
                    ),
                });
            }
            let mut seen_slots = Vec::new();
            for interface in &root.interfaces {
                for method in &interface.methods {
                    if seen_slots
                        .iter()
                        .any(|seen: &(InterfaceInstantiationRef, u32)| {
                            seen.0 == interface.interface && seen.1 == method.slot
                        })
                    {
                        return Err(BytecodeEmissionError::UnsupportedConstruct {
                            function_key: String::new(),
                            construct: "provider public-instance facts",
                            location: format!(
                                " public instance `{}` repeats interface method slot {}",
                                root.public_root, method.slot
                            ),
                        });
                    }
                    seen_slots.push((interface.interface.clone(), method.slot));
                    method_functions
                        .entry(root.const_module.clone())
                        .or_default()
                        .insert(method.executable_index);
                }
                conformances.push((
                    root.const_module.clone(),
                    root.receiver_type.clone(),
                    interface.interface.clone(),
                ));
            }
        }
        self.provider_receiver_constants = receiver_constants;
        self.provider_conformances = conformances;
        self.provider_method_functions = method_functions;
        Ok(())
    }

    fn method_for_call<'a>(
        &'a self,
        unit: &MirUnit,
        function: &MirFunction,
        call: &CallIr,
    ) -> Option<&'a LocalInterfaceMethodFact> {
        let CallTargetIr::InterfaceMethod {
            interface,
            method_abi_id,
            slot,
        } = &call.target
        else {
            return None;
        };
        let receiver = call.args.first()?;
        let concrete_type =
            local_interface_receiver_concrete_type(unit, function, *receiver).ok()?;
        let table = self.table(interface, &concrete_type)?;
        table
            .methods
            .iter()
            .find(|method| method.slot == *slot && &method.method_abi_id == method_abi_id)
    }

    pub(crate) fn exact_call_result(
        &self,
        unit: &MirUnit,
        function: &MirFunction,
        expression_index: u32,
        expected: &TypeRefIr,
    ) -> bool {
        let Ok(expression) = function.expression(ExprRefIr {
            expression: expression_index,
        }) else {
            return false;
        };
        let ExprIr::Call { call } = &expression.expression else {
            return false;
        };
        self.method_for_call(unit, function, call)
            .is_some_and(|method| &method.signature.return_type == expected)
    }

    pub(crate) fn exact_concrete_construct_value(
        &self,
        function: &MirFunction,
        expression_index: u32,
        expected: &TypeRefIr,
    ) -> bool {
        if !matches!(
            expected,
            TypeRefIr::Builtin { name, args } if name == "string" && args.is_empty()
        ) && !matches!(
            expected,
            TypeRefIr::Literal {
                value: LiteralIr::String { .. }
            }
        ) {
            return false;
        }
        function.expressions.iter().any(|expression| {
            let ExprIr::Construct { type_ref, fields } = &expression.expression else {
                return false;
            };
            self.concrete_type(type_ref)
                && fields
                    .values()
                    .any(|field| field.expression == expression_index)
        })
    }

    pub(crate) fn exact_local_interface_string_length(
        &self,
        unit: &MirUnit,
        function: &MirFunction,
        call: &CallIr,
    ) -> bool {
        let CallTargetIr::ReceiverBuiltin { op } = &call.target else {
            return false;
        };
        if op.canonical_key != "receiver:string.length@1" || call.args.len() != 1 {
            return false;
        }
        let Ok(receiver) = function.expression(call.args[0]) else {
            return false;
        };
        if receiver.ty != TypeRefIr::builtin("string") {
            return false;
        }
        self.method_for_executable(function.executable_index)
            .is_some()
            || self.exact_call_result(
                unit,
                function,
                receiver.index,
                &TypeRefIr::builtin("string"),
            )
            || self.exact_concrete_construct_value(
                function,
                receiver.index,
                &TypeRefIr::builtin("string"),
            )
    }

    pub(crate) fn remote_interface_receiver(
        &self,
        unit: &MirUnit,
        function: &MirFunction,
        receiver: ExprRefIr,
    ) -> bool {
        let Ok(expression) = function.expression(receiver) else {
            return false;
        };
        if matches!(
            &expression.expression,
            ExprIr::InterfaceBox {
                source: BoxSourceIr::Remote { .. },
                ..
            }
        ) {
            return true;
        }
        let ExprIr::LoadSlot { slot } = &expression.expression else {
            return false;
        };
        for block in &function.blocks {
            for statement in &block.statements {
                let value = match &statement.kind {
                    MirStmtKind::InitSlot {
                        slot: candidate,
                        value,
                    } if candidate == slot => Some(*value),
                    MirStmtKind::Assign {
                        target: AssignTargetIr::Slot { slot: candidate },
                        value,
                        ..
                    } if candidate == slot => Some(*value),
                    _ => None,
                };
                let Some(value) = value else {
                    continue;
                };
                if function.expression(value).is_ok_and(|value| {
                    matches!(
                        &value.expression,
                        ExprIr::InterfaceBox {
                            source: BoxSourceIr::Remote { .. },
                            ..
                        }
                    )
                }) {
                    return true;
                }
            }
        }
        let _ = unit;
        false
    }

    pub(crate) fn remote_interface_return_face(
        &self,
        unit: &MirUnit,
        function: &MirFunction,
        return_type: &TypeRefIr,
    ) -> bool {
        function.expressions.iter().any(|expression| {
            expression.ty == *return_type
                && matches!(
                    &expression.expression,
                    ExprIr::Call {
                        call:
                            CallIr {
                                target: CallTargetIr::InterfaceMethod { .. },
                                args,
                                ..
                            },
                    } if args.first().is_some_and(|receiver| {
                        self.remote_interface_receiver(unit, function, *receiver)
                    })
                )
        })
    }

    pub(crate) fn remote_interface_facts_for_receiver<'a>(
        &'a self,
        unit: &MirUnit,
        function: &'a MirFunction,
        receiver: ExprRefIr,
    ) -> Option<&'a skiff_compiler_lowering::mir::MirRemoteInterfaceFacts> {
        let expression = function.expression(receiver).ok()?;
        if let ExprIr::InterfaceBox {
            source: BoxSourceIr::Remote { .. },
            ..
        } = &expression.expression
        {
            return expression.remote_interface.as_ref();
        }
        let ExprIr::LoadSlot { slot } = &expression.expression else {
            return None;
        };
        for block in &function.blocks {
            for statement in &block.statements {
                let value = match &statement.kind {
                    MirStmtKind::InitSlot {
                        slot: candidate,
                        value,
                    } if candidate == slot => Some(*value),
                    MirStmtKind::Assign {
                        target: AssignTargetIr::Slot { slot: candidate },
                        value,
                        ..
                    } if candidate == slot => Some(*value),
                    _ => None,
                };
                let value = value?;
                if let Ok(value) = function.expression(value) {
                    if matches!(
                        &value.expression,
                        ExprIr::InterfaceBox {
                            source: BoxSourceIr::Remote { .. },
                            ..
                        }
                    ) {
                        return value.remote_interface.as_ref();
                    }
                }
            }
        }
        let _ = unit;
        None
    }

    pub(crate) fn remote_interface_facts_for_call<'a>(
        &'a self,
        unit: &MirUnit,
        function: &'a MirFunction,
        interface: &InterfaceInstantiationRef,
        slot: u32,
    ) -> Option<&'a skiff_compiler_lowering::mir::MirRemoteInterfaceFacts> {
        let mut matches = function
            .expressions
            .iter()
            .filter(|expression| {
                let ExprIr::InterfaceBox {
                    source: BoxSourceIr::Remote { .. },
                    ..
                } = &expression.expression
                else {
                    return false;
                };
                expression.remote_interface.as_ref().is_some_and(|facts| {
                    &facts.interface == interface
                        && facts.methods.iter().any(|method| method.slot == slot)
                })
            })
            .filter_map(|expression| expression.remote_interface.as_ref());
        let first = matches.next()?;
        if matches.next().is_some() {
            let _ = unit;
            return None;
        }
        Some(first)
    }

    fn tables_for_interface(
        &self,
        interface: &InterfaceInstantiationRef,
    ) -> Vec<&LocalInterfaceTableFact> {
        self.tables
            .iter()
            .filter(move |table| &table.interface == interface)
            .collect()
    }
}

/// Exact provider public-instance facts consumed by Phase 1 admission.
///
/// These facts are assembled from source-owned public-instance API bindings,
/// interface operation rows and validated local conformances. They permit a
/// provider const receiver and its selected interface methods to enter the
/// bytecode image without inventing a local interface table or widening the
/// generic constant/receiver admission surface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderPublicInstanceFacts {
    pub roots: Vec<ProviderPublicInstanceRoot>,
}

impl ProviderPublicInstanceFacts {
    pub fn empty() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPublicInstanceRoot {
    pub public_root: String,
    pub const_module: String,
    pub const_symbol: String,
    pub receiver_type: TypeRefIr,
    pub interfaces: Vec<ProviderPublicInstanceInterface>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPublicInstanceInterface {
    pub interface: InterfaceInstantiationRef,
    pub methods: Vec<ProviderPublicInstanceMethod>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderPublicInstanceMethod {
    pub slot: u32,
    pub method_abi_id: String,
    pub executable_index: u32,
}

/// One exact actor declaration row retained by Phase 1 admission.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActorDeclarationFact {
    pub(crate) actor: ServiceSymbolRef,
    pub(crate) actor_abi_identity: ActorAbiIdentity,
    pub(crate) actor_implementation_identity: ActorImplementationIdentity,
    pub(crate) fields: BTreeMap<String, TypeRefIr>,
    pub(crate) key_field: String,
    pub(crate) id_type: TypeRefIr,
    pub(crate) method_implementations: BTreeMap<ActorMethodIdentity, u32>,
    pub(crate) create_identity: Option<ActorMethodIdentity>,
}

impl ActorDeclarationFact {
    pub(crate) fn actor_id_type(&self) -> TypeRefIr {
        self.id_type.clone()
    }
}

/// One exact actor method executable row joined from the declaration table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActorMethodFact {
    pub(crate) actor: ServiceSymbolRef,
    pub(crate) actor_abi_identity: ActorAbiIdentity,
    pub(crate) actor_implementation_identity: ActorImplementationIdentity,
    pub(crate) method_identity: ActorMethodIdentity,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActorFacts {
    declarations: BTreeMap<(String, String), ActorDeclarationFact>,
    local_type_indices: BTreeSet<u32>,
    publication_type_indices: BTreeSet<(String, u32)>,
    methods: BTreeMap<u32, ActorMethodFact>,
}

impl ActorFacts {
    pub(crate) fn actor(&self, actor: &ServiceSymbolRef) -> Option<&ActorDeclarationFact> {
        self.declarations
            .get(&(actor.module_path.clone(), actor.symbol.clone()))
    }

    pub(crate) fn actor_for_method(&self, executable_index: u32) -> Option<&ActorMethodFact> {
        self.methods.get(&executable_index)
    }

    pub(crate) fn exact_actor_method(
        &self,
        actor: &ServiceSymbolRef,
        abi: &ActorAbiIdentity,
        implementation: &ActorImplementationIdentity,
        method: &ActorMethodIdentity,
    ) -> bool {
        self.actor(actor).is_some_and(|declaration| {
            declaration.actor_abi_identity == *abi
                && declaration.actor_implementation_identity == *implementation
                && declaration.method_implementations.contains_key(method)
        })
    }

    pub(crate) fn is_actor_handle(&self, ty: &TypeRefIr) -> bool {
        match ty {
            TypeRefIr::ServiceSymbol { symbol } => self.actor(symbol).is_some(),
            TypeRefIr::LocalType { type_index } => self.local_type_indices.contains(type_index),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self
                .publication_type_indices
                .contains(&(module_path.clone(), *type_index)),
            _ => false,
        }
    }
}

pub(crate) fn collect_actor_facts(units: &[MirUnit]) -> Result<ActorFacts, BytecodeEmissionError> {
    let mut facts = ActorFacts::default();
    for unit in units {
        for declaration in &unit.actor_declarations {
            collect_actor_declaration(unit, declaration, &mut facts)?;
        }
    }
    Ok(facts)
}

fn collect_actor_declaration(
    unit: &MirUnit,
    declaration: &ActorDeclarationIr,
    facts: &mut ActorFacts,
) -> Result<(), BytecodeEmissionError> {
    let actor_name = declaration.abi.actor_name.as_str();
    if actor_name.is_empty()
        || declaration.actor_abi_identity.as_str().is_empty()
        || declaration
            .actor_implementation_identity
            .as_str()
            .is_empty()
        || declaration.abi.actor_runtime_abi_version.is_empty()
    {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            "actor declaration table has an empty exact identity field",
        ));
    }
    let attached_type = unit
        .type_table
        .iter()
        .enumerate()
        .find(|(_, declaration)| declaration.name == actor_name)
        .ok_or_else(|| {
            rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Actor,
                &format!(
                    "actor declaration table actor `{actor_name}` has no attached record type"
                ),
            )
        })?;
    let attached_type_index = u32::try_from(attached_type.0).map_err(|_| {
        rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            "actor attached record type index exceeds u32::MAX",
        )
    })?;
    let attached_type = attached_type.1;
    let TypeDescriptorIr::Record {
        fields: attached_fields,
    } = &attached_type.descriptor
    else {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            &format!("actor declaration table actor `{actor_name}` does not attach to a record"),
        ));
    };
    let declared_fields = declaration
        .abi
        .fields
        .iter()
        .map(|field| (field.name.clone(), field.ty.clone()))
        .collect::<BTreeMap<_, _>>();
    if declared_fields != *attached_fields {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            &format!("actor declaration table actor `{actor_name}` field facts drift from its attached record"),
        ));
    }
    let public_methods = declaration
        .abi
        .public_methods
        .iter()
        .map(|method| method.method_identity.clone())
        .collect::<BTreeSet<_>>();
    let implementation_methods = declaration
        .method_implementations
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if public_methods != implementation_methods {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            &format!("actor declaration table actor `{actor_name}` method rows are not exact"),
        ));
    }
    if declaration.abi.create.is_some() != declaration.create_implementation.is_some() {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            &format!("actor declaration table actor `{actor_name}` create rows are not exact"),
        ));
    }
    let create_identity = declaration
        .create_implementation
        .as_ref()
        .map(|create| create.identity.clone());
    if create_identity
        .as_ref()
        .is_some_and(|identity| public_methods.contains(identity))
    {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            &format!("actor declaration table actor `{actor_name}` create aliases a public method"),
        ));
    }
    let actor = ServiceSymbolRef {
        module_path: unit.module_path.clone(),
        symbol: actor_name.to_string(),
    };
    for (method_identity, executable_index) in declaration.method_implementations.iter() {
        let executable = unit
            .functions
            .iter()
            .find(|function| function.executable_index == *executable_index)
            .ok_or_else(|| {
                rejected(
                    unit,
                    None,
                    Phase1UnsupportedCapability::Actor,
                    &format!("actor declaration table actor `{actor_name}` method {method_identity:?} has an absent executable"),
                )
            })?;
        if executable.kind != MirExecutableKind::ImplMethod {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Actor,
                &format!("actor declaration table actor `{actor_name}` method {method_identity:?} target is not an impl method"),
            ));
        }
        let method_fact = ActorMethodFact {
            actor: actor.clone(),
            actor_abi_identity: declaration.actor_abi_identity.clone(),
            actor_implementation_identity: declaration.actor_implementation_identity.clone(),
            method_identity: method_identity.clone(),
        };
        if facts
            .methods
            .insert(*executable_index, method_fact)
            .is_some()
        {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Actor,
                &format!("actor declaration table executable {executable_index} has more than one exact actor row"),
            ));
        }
    }
    if let Some(create) = declaration.create_implementation.as_ref() {
        let executable = unit
            .functions
            .iter()
            .find(|function| function.executable_index == create.executable_index)
            .ok_or_else(|| {
                rejected(
                    unit,
                    None,
                    Phase1UnsupportedCapability::Actor,
                    &format!("actor declaration table actor `{actor_name}` create {:?} has an absent executable", create.identity),
                )
            })?;
        if executable.kind != MirExecutableKind::ImplMethod {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Actor,
                &format!("actor declaration table actor `{actor_name}` create {:?} target is not an impl method", create.identity),
            ));
        }
        let method_fact = ActorMethodFact {
            actor: actor.clone(),
            actor_abi_identity: declaration.actor_abi_identity.clone(),
            actor_implementation_identity: declaration.actor_implementation_identity.clone(),
            method_identity: create.identity.clone(),
        };
        if facts
            .methods
            .insert(create.executable_index, method_fact)
            .is_some()
        {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Actor,
                &format!(
                    "actor declaration table executable {} has more than one exact actor row",
                    create.executable_index
                ),
            ));
        }
    }
    let declaration_fact = ActorDeclarationFact {
        actor: actor.clone(),
        actor_abi_identity: declaration.actor_abi_identity.clone(),
        actor_implementation_identity: declaration.actor_implementation_identity.clone(),
        fields: declared_fields,
        key_field: declaration.abi.key_field.clone(),
        id_type: declaration.abi.actor_id_type.clone(),
        method_implementations: declaration.method_implementations.clone(),
        create_identity,
    };
    facts.local_type_indices.insert(attached_type_index);
    facts
        .publication_type_indices
        .insert((unit.module_path.clone(), attached_type_index));
    let key = (actor.module_path.clone(), actor.symbol.clone());
    if facts.declarations.insert(key, declaration_fact).is_some() {
        return Err(rejected(
            unit,
            None,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "actor declaration table repeats actor `{}`",
                actor.symbol_path()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn collect_local_interface_tables(
    units: &[MirUnit],
) -> Result<LocalInterfaceFacts, BytecodeEmissionError> {
    let mut tables = Vec::<LocalInterfaceTableFact>::new();
    for unit in units {
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            for expression in &function.expressions {
                let ExprIr::InterfaceBox {
                    interface,
                    source:
                        BoxSourceIr::Local {
                            concrete_type,
                            method_table,
                        },
                    ..
                } = &expression.expression
                else {
                    continue;
                };
                if &method_table.interface != interface {
                    return Err(BytecodeEmissionError::UnsupportedConstruct {
                        function_key: function_key.clone(),
                        construct: "local interface method table",
                        location: format!(
                            " expression {} table interface {:?} diverges from box interface {interface:?}",
                            expression.index, method_table.interface
                        ),
                    });
                }
                let mut methods = Vec::with_capacity(method_table.slots.len());
                for (ordinal, slot) in method_table.slots.iter().enumerate() {
                    if slot.slot != ordinal as u32 {
                        return Err(BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "local interface method table",
                            location: format!(
                                " expression {} slot {} is not dense from zero",
                                expression.index, slot.slot
                            ),
                        });
                    }
                    let target = unit
                        .function_by_executable_index(slot.target.executable_index)
                        .map_err(|_| BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "local interface method table",
                            location: format!(
                                " expression {} slot {} target executable {} is absent",
                                expression.index, slot.slot, slot.target.executable_index
                            ),
                        })?;
                    if target.kind != MirExecutableKind::ImplMethod {
                        return Err(BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "local interface method table",
                            location: format!(
                                " expression {} slot {} target is not an impl method",
                                expression.index, slot.slot
                            ),
                        });
                    }
                    let mut expected_params = Vec::with_capacity(target.params.len() + 1);
                    if let Some(receiver) = target.receiver.as_ref() {
                        expected_params.push(FunctionTypeParamIr {
                            name: "self".to_string(),
                            ty: receiver.ty.clone(),
                        });
                        if matches!(
                            target.slots.first().map(|slot| slot.kind),
                            Some(MirSlotKind::Param)
                        ) {
                            expected_params.extend(target.params.iter().skip(1).map(|parameter| {
                                FunctionTypeParamIr {
                                    name: parameter.name.clone(),
                                    ty: parameter.ty.clone(),
                                }
                            }));
                        } else {
                            expected_params.extend(target.params.iter().map(|parameter| {
                                FunctionTypeParamIr {
                                    name: parameter.name.clone(),
                                    ty: parameter.ty.clone(),
                                }
                            }));
                        }
                    } else {
                        expected_params.extend(target.params.iter().map(|parameter| {
                            FunctionTypeParamIr {
                                name: parameter.name.clone(),
                                ty: parameter.ty.clone(),
                            }
                        }));
                    }
                    let expected_signature = InterfaceMethodSlotSignatureIr {
                        params: expected_params,
                        return_type: target.return_type.clone(),
                    };
                    if slot.signature != expected_signature
                        || target.receiver.as_ref().is_some_and(|receiver| {
                            receiver.call_abi != slot.target.receiver_call_abi
                        })
                    {
                        return Err(BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "local interface method table",
                            location: format!(
                                " expression {} slot {} signature drifts from implementation function",
                                expression.index, slot.slot
                            ),
                        });
                    }
                    if matches!(target.effect_summary, CallableEffectSummary::Unknown { .. }) {
                        return Err(BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "local interface method effects",
                            location: format!(
                                " expression {} slot {} target has an unknown effect summary",
                                expression.index, slot.slot
                            ),
                        });
                    }
                    methods.push(LocalInterfaceMethodFact {
                        slot: slot.slot,
                        method_name: slot.method_name.clone(),
                        method_abi_id: slot.method_abi_id.clone(),
                        signature: slot.signature.clone(),
                        effects: target.effect_summary.clone(),
                        executable_index: slot.target.executable_index,
                        function_key: canonical_function_key(&unit.module_path, &target.symbol)?,
                        receiver_call_abi: slot.target.receiver_call_abi,
                    });
                }
                let table = LocalInterfaceTableFact {
                    interface: interface.clone(),
                    concrete_type: concrete_type.clone(),
                    methods,
                };
                if let Some(previous) = tables.iter().find(|existing| {
                    existing.interface == table.interface
                        && existing.concrete_type == table.concrete_type
                }) {
                    if previous != &table {
                        return Err(BytecodeEmissionError::UnsupportedConstruct {
                            function_key: function_key.clone(),
                            construct: "local interface method table",
                            location: format!(
                                " duplicate table for interface {interface:?} concrete type {concrete_type:?} diverges"
                            ),
                        });
                    }
                } else {
                    tables.push(table);
                }
            }
        }
    }
    Ok(LocalInterfaceFacts {
        tables,
        provider_receiver_constants: BTreeMap::new(),
        provider_conformances: Vec::new(),
        provider_method_functions: BTreeMap::new(),
    })
}

pub(crate) fn resolve_local_interface_table_for_call<'a>(
    unit: &MirUnit,
    function: &MirFunction,
    call: &CallIr,
    facts: &'a LocalInterfaceFacts,
) -> Result<&'a LocalInterfaceTableFact, BytecodeEmissionError> {
    let CallTargetIr::InterfaceMethod { interface, .. } = &call.target else {
        return Err(BytecodeEmissionError::UnsupportedConstruct {
            function_key: String::new(),
            construct: "local interface call target",
            location: " call is not an interface method".to_string(),
        });
    };
    let receiver =
        call.args
            .first()
            .ok_or_else(|| BytecodeEmissionError::UnsupportedConstruct {
                function_key: String::new(),
                construct: "local interface call target",
                location: " interface call has no receiver argument".to_string(),
            })?;
    let concrete_type =
        local_interface_receiver_concrete_type(unit, function, *receiver).map_err(|error| {
            BytecodeEmissionError::UnsupportedConstruct {
                function_key: String::new(),
                construct: "local interface call target",
                location: format!(" receiver join failed: {error}"),
            }
        })?;
    facts.table(interface, &concrete_type).ok_or_else(|| {
        BytecodeEmissionError::UnsupportedConstruct {
            function_key: String::new(),
            construct: "local interface call target",
            location: format!(
                " interface {interface:?} concrete type {concrete_type:?} has no exact local table"
            ),
        }
    })
}

fn local_interface_receiver_concrete_type(
    unit: &MirUnit,
    function: &MirFunction,
    receiver: ExprRefIr,
) -> Result<TypeRefIr, String> {
    let expression = function
        .expression(receiver)
        .map_err(|error| format!("receiver expression is absent: {error}"))?;
    if let ExprIr::InterfaceBox {
        source: BoxSourceIr::Local { concrete_type, .. },
        ..
    } = &expression.expression
    {
        return Ok(concrete_type.clone());
    }
    let ExprIr::LoadSlot { slot } = &expression.expression else {
        return Err("interface receiver is not an exact local box or slot".to_string());
    };
    let mut found = None;
    for block in &function.blocks {
        for statement in &block.statements {
            let value = match &statement.kind {
                MirStmtKind::InitSlot {
                    slot: candidate,
                    value,
                } if candidate == slot => Some(*value),
                MirStmtKind::Assign {
                    target: AssignTargetIr::Slot { slot: candidate },
                    value,
                    ..
                } if candidate == slot => Some(*value),
                _ => None,
            };
            let Some(value) = value else {
                continue;
            };
            let value_expression = function
                .expression(value)
                .map_err(|error| format!("slot initializer expression is absent: {error}"))?;
            let ExprIr::InterfaceBox {
                source: BoxSourceIr::Local { concrete_type, .. },
                ..
            } = &value_expression.expression
            else {
                return Err(
                    "interface receiver slot is not initialized from an exact local box"
                        .to_string(),
                );
            };
            if let Some(previous) = found.as_ref() {
                if previous != concrete_type {
                    return Err(
                        "interface receiver slot has ambiguous concrete local tables".to_string(),
                    );
                }
            } else {
                found = Some(concrete_type.clone());
            }
        }
    }
    found.ok_or_else(|| {
        let _ = unit;
        "interface receiver slot has no exact local box initializer".to_string()
    })
}

/// Admits the Phase 2 record/array MIR surface plus the retained Phase 1
/// scalar/local-call core, and the Phase 3 synchronous throw/catch/rethrow
/// surface (nominal/union payloads over the Phase 2 value face).
///
/// This is the production bytecode lane's source-owned capability boundary.
/// It runs before constant evaluation, value-transfer derivation, or bytecode
/// emission and returns no partially emitted state. The admission reads only
/// typed MIR facts; package names and binding strings never grant capability.
/// The exact supported value shapes are `record` and `array` recursively over
/// `number`/`boolean`/`null` and nested record/array; `string`, `bytes`,
/// `map`, representations, streams, host targets, tail calls, generics and
/// `InOut` remain rejected at this single boundary. Synchronous `throw`,
/// `catch` and `rethrow` are admitted when their payload types stay on the
/// Phase 2 record/array/scalar face (directly or as union leaves); host
/// effect, Pending, child and stream throw producers stay fail closed
/// through the existing target/effect rejections.
///
/// Phase 3 Amendment 1 admits the minimal compile-time string-literal slice:
/// a string literal is accepted only as a union/`CatchResult` discriminator
/// constant (`.tag` reads, `tag == "…"` equality and their narrowed types).
/// General string values (bindings, fields, aggregates, boundary payloads,
/// concatenation) remain rejected.
///
/// Phase 4 gate 1 admits exactly one host effect: the canonical
/// `std.time.sleep` binding with its pinned arity (one `Duration` argument),
/// pinned parameter type (`skiff.run/std::std.time.Duration`) and pinned
/// `void` result. Every other host binding, every other pending category and
/// any drifted/missing fact stay fail closed at this single boundary.
pub fn admit_phase_1_bytecode_mir(
    units: &[MirUnit],
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_server_stream_authorities_and_service_boundary_plans(
        units,
        &[],
        &BTreeMap::new(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_server_stream_authorities(
    units: &[MirUnit],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_server_stream_authorities_and_service_boundary_plans(
        units,
        server_stream_authorities,
        &BTreeMap::new(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_server_stream_authorities_and_service_boundary_plans(
    units: &[MirUnit],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    let gateway_parameter_authorities = server_stream_authorities
        .iter()
        .map(|authority| GatewayParameterAuthority::new(authority.entry().clone()))
        .collect::<Vec<_>>();
    admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
        units,
        &gateway_parameter_authorities,
        server_stream_authorities,
        service_boundary_plans,
    )
}

pub fn admit_phase_1_bytecode_mir_with_gateway_authorities(
    units: &[MirUnit],
    gateway_parameter_authorities: &[GatewayParameterAuthority],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
        units,
        gateway_parameter_authorities,
        server_stream_authorities,
        &BTreeMap::new(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans(
    units: &[MirUnit],
    gateway_parameter_authorities: &[GatewayParameterAuthority],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans_and_provider_public_instances(
        units,
        gateway_parameter_authorities,
        server_stream_authorities,
        service_boundary_plans,
        &ProviderPublicInstanceFacts::empty(),
    )
}

pub fn admit_phase_1_bytecode_mir_with_gateway_authorities_and_service_boundary_plans_and_provider_public_instances(
    units: &[MirUnit],
    gateway_parameter_authorities: &[GatewayParameterAuthority],
    server_stream_authorities: &[ServerStreamGatewayAuthority],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
    provider_public_instances: &ProviderPublicInstanceFacts,
) -> Result<AdmittedPhase1BytecodeMir, BytecodeEmissionError> {
    let units =
        package_type_authority::normalize_package_type_authorities(units).map_err(|error| {
            rejected(
                &units[error.unit_index],
                None,
                Phase1UnsupportedCapability::ValueShape,
                &format!("package type authority: {}", error.detail),
            )
        })?;
    let dense_parameter_materializations =
        gateway_parameter::analyze(&units, gateway_parameter_authorities).map_err(|detail| {
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                module_path: units
                    .first()
                    .map_or_else(String::new, |unit| unit.module_path.clone()),
                function_key: None,
                location: format!("rawHttp gateway parameter authority: {detail}"),
            }
        })?;
    server_stream::validate_authority_coverage(&units, server_stream_authorities).map_err(
        |detail| BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::Stream,
            module_path: units
                .first()
                .map_or_else(String::new, |unit| unit.module_path.clone()),
            function_key: None,
            location: format!("server-stream gateway authority: {detail}"),
        },
    )?;
    validate_service_boundary_plan_coverage(&units, service_boundary_plans)?;
    let mut local_interface_tables = collect_local_interface_tables(&units)?;
    local_interface_tables.add_provider_public_instances(provider_public_instances)?;
    let local_interface_method_functions = local_interface_tables
        .tables()
        .iter()
        .flat_map(|table| table.methods.iter())
        .map(|method| method.executable_index)
        .collect::<BTreeSet<_>>();
    let actor_facts = collect_actor_facts(&units)?;
    for unit in &units {
        unit.validate_executable_indices()?;
        if let Some(constant) = unit.constants.iter().find(|constant| {
            !local_interface_tables.provider_receiver_constant(&unit.module_path, constant)
        }) {
            return Err(rejected(
                unit,
                None,
                Phase1UnsupportedCapability::Constant,
                &format!(
                    "compile-time constant table constant `{}` outside an exact provider public-instance receiver",
                    constant.symbol
                ),
            ));
        }
        for declaration in &unit.type_table {
            if !declaration.type_params.is_empty() {
                return Err(rejected(
                    unit,
                    None,
                    Phase1UnsupportedCapability::Generic,
                    &format!("type declaration `{}`", declaration.name),
                ));
            }
            for implemented in &declaration.implements {
                let TypeRefIr::AnyInterface { interface } = implemented else {
                    return Err(rejected(
                        unit,
                        None,
                        Phase1UnsupportedCapability::Interface,
                        &format!(
                            "type declaration `{}` implements non-local interface {implemented:?}",
                            declaration.name
                        ),
                    ));
                };
                if local_interface_tables
                    .tables_for_interface(interface)
                    .is_empty()
                    && !local_interface_tables.provider_conformance(
                        unit,
                        &receiver_type_for_declaration(unit, declaration),
                        interface,
                    )
                {
                    return Err(rejected(
                        unit,
                        None,
                        Phase1UnsupportedCapability::Interface,
                        &format!(
                            "type declaration `{}` implements local interface {interface:?} without an exact local method table",
                            declaration.name
                        ),
                    ));
                }
            }
            if !matches!(declaration.descriptor, TypeDescriptorIr::Record { .. }) {
                if !matches!(declaration.descriptor, TypeDescriptorIr::Interface) {
                    return Err(rejected(
                        unit,
                        None,
                        Phase1UnsupportedCapability::ValueShape,
                        &format!("type declaration `{}`", declaration.name),
                    ));
                }
            }
        }
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            admit_function(
                &units,
                unit,
                &function_key,
                function,
                &dense_parameter_materializations,
                server_stream_authorities,
                &local_interface_tables,
                &local_interface_method_functions,
                &actor_facts,
            )?;
        }
    }
    let machine_carriers = analyze_machine_carriers(&units)?;
    let representation_carriers = representation_carrier::analyze(&units, &machine_carriers)?;
    Ok(AdmittedPhase1BytecodeMir {
        units,
        dense_parameter_materializations,
        machine_carriers,
        representation_carriers,
        service_boundary_plans: service_boundary_plans.clone(),
        local_interface_tables,
    })
}

fn receiver_type_for_declaration(
    unit: &MirUnit,
    declaration: &skiff_artifact_model::TypeDeclIr,
) -> TypeRefIr {
    unit.type_table
        .iter()
        .position(|candidate| candidate.name == declaration.name)
        .and_then(|index| u32::try_from(index).ok())
        .map_or_else(
            || TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: unit.module_path.clone(),
                    symbol: declaration.name.clone(),
                },
            },
            |type_index| TypeRefIr::LocalType { type_index },
        )
}

fn local_interface_declaration_identity(
    unit: &MirUnit,
    interface: &InterfaceInstantiationRef,
) -> Option<(TypeRefIr, Vec<TypeRefIr>)> {
    let declaration = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).ok()?;
    let declaration = match declaration {
        TypeRefIr::ServiceSymbol { symbol } => unit
            .type_table
            .iter()
            .find(|candidate| candidate.name == symbol.symbol)
            .map(|_| TypeRefIr::ServiceSymbol {
                symbol: symbol.clone(),
            })
            .or(Some(TypeRefIr::ServiceSymbol { symbol }))?,
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => {
            let name = unit.type_table.get(type_index as usize)?.name.clone();
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path,
                    symbol: name,
                },
            }
        }
        TypeRefIr::LocalType { type_index } => {
            let name = unit.type_table.get(type_index as usize)?.name.clone();
            TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: unit.module_path.clone(),
                    symbol: name,
                },
            }
        }
        other => other,
    };
    Some((declaration, interface.canonical_type_args.clone()))
}

fn validate_service_boundary_plan_coverage(
    units: &[MirUnit],
    service_boundary_plans: &BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
) -> Result<(), BytecodeEmissionError> {
    let mut required = BTreeMap::<ServiceCallRef, ()>::new();
    for unit in units {
        for service_call in &unit.external_refs.service_call_refs {
            required.insert(service_call.clone(), ());
        }
        for service_call in &unit.remote_interface_refs {
            required.insert(service_call.clone(), ());
        }
    }
    for service_call in required.keys() {
        let plan = service_boundary_plans.get(service_call).ok_or_else(|| {
            BytecodeEmissionError::MissingServiceBoundaryPlan {
                service_call: format!("{service_call:?}"),
            }
        })?;
        if plan.stream_item.is_some()
            || matches!(
                plan.callbacks,
                skiff_artifact_model::ServiceCallbackPlan::Unsupported { .. }
            )
        {
            return Err(BytecodeEmissionError::UnsupportedServiceBoundaryPlan {
                location: format!("service call {service_call:?}"),
                detail: "stream item and callback surfaces are disabled in the first service lane"
                    .to_string(),
            });
        }
    }
    for service_call in service_boundary_plans.keys() {
        if !required.contains_key(service_call) {
            return Err(BytecodeEmissionError::UnexpectedServiceBoundaryPlan {
                service_call: format!("{service_call:?}"),
            });
        }
    }
    Ok(())
}

fn admit_function(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    dense_parameter_materializations: &BTreeMap<String, DenseParameterMaterializationFact>,
    server_stream_authorities: &[ServerStreamGatewayAuthority],
    local_interface_tables: &LocalInterfaceFacts,
    local_interface_method_functions: &BTreeSet<u32>,
    actor_facts: &ActorFacts,
) -> Result<(), BytecodeEmissionError> {
    function.validate_expression_indices()?;
    function.validate_slot_types()?;
    let interface_impl = local_interface_method_functions.contains(&function.executable_index);
    let actor_method = actor_facts.actor_for_method(function.executable_index);
    let actor_impl = actor_method.is_some();
    let provider_impl = local_interface_tables
        .provider_method_for_executable(&unit.module_path, function.executable_index);
    let exact_actor_face = interface_impl || actor_impl || provider_impl;
    if interface_impl && actor_impl {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            "actor method is also a local interface implementation",
        ));
    }
    match function.kind {
        MirExecutableKind::Function => {}
        MirExecutableKind::ImplMethod if interface_impl || actor_impl || provider_impl => {}
        MirExecutableKind::ImplMethod => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Receiver,
                "implementation method outside an exact local interface table",
            ));
        }
    }
    if interface_impl || actor_impl || provider_impl {
        let Some(receiver) = function.receiver.as_ref() else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Receiver,
                "implementation has no receiver facts",
            ));
        };
        let Some(self_type) = function.self_type.as_ref() else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Receiver,
                "implementation has no self type",
            ));
        };
        if receiver.ty != *self_type
            || receiver.slot != 0
            || receiver.parameter_ordinal != 0
            || receiver.call_abi != ReceiverCallAbi::ExplicitSelfFirst
        {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Receiver,
                "implementation receiver facts are not exact",
            ));
        }
        let slot_zero = function.slots.first().ok_or_else(|| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Receiver,
                "implementation has no receiver slot zero",
            )
        })?;
        if slot_zero.slot != 0 || slot_zero.ty.as_ref() != Some(self_type) {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Receiver,
                "implementation receiver slot disagrees with self type",
            ));
        }
    } else if function.self_type.is_some() || function.receiver.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            "receiver facts",
        ));
    }
    if function.native {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            "native executable",
        ));
    }
    if !function.type_params.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Generic,
            "function type parameters",
        ));
    }
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .map_err(|error| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} exact host-effect admission: {}",
                        error.expression_index, error.detail
                    ),
                )
            })?;
    let server_stream = ServerStreamAdmissions::analyze(unit, function, server_stream_authorities)
        .map_err(|detail| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                &format!("exact server-stream admission: {detail}"),
            )
        })?;
    let dense_parameter_materialization = dense_parameter_materializations.get(function_key);
    if let Some(stream) = &function.stream_result {
        let TypeRefIr::Builtin { name, args } = &function.return_type else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                "stream result authority without Stream<T> return type",
            ));
        };
        if name != "Stream" || args.as_slice() != [stream.item_type.clone()] {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                "stream result authority differs from Stream<T> return type",
            ));
        }
        if !server_stream.admits_result(&stream.item_type) {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                "server-stream result lacks exact canonical gateway authority",
            ));
        }
    } else if !server_stream.admits_closure_carrier(&function.return_type) {
        if exact_actor_face
            || local_interface_tables.remote_interface_return_face(
                unit,
                function,
                &function.return_type,
            )
        {
            admit_type_with_exact_local_interface_face(
                units,
                unit,
                function_key,
                &function.return_type,
                true,
                "return type",
                local_interface_tables,
            )?;
        } else {
            admit_type_with_local_facts(
                units,
                unit,
                function_key,
                &function.return_type,
                true,
                "return type",
                local_interface_tables,
            )?;
        }
    }
    let mut parameter_slots = BTreeSet::new();
    let parameter_ordinal_offset = if function.receiver.is_some() {
        match function.slots.first().map(|slot| slot.kind) {
            Some(MirSlotKind::SelfValue) => 1,
            Some(MirSlotKind::Param) => 0,
            _ => {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Receiver,
                    "receiver function has an invalid slot zero kind",
                ));
            }
        }
    } else {
        0
    };
    for (parameter_index, parameter) in function.params.iter().enumerate() {
        if parameter.mode == MirParamMode::InOut {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::InOut,
                &format!("parameter {parameter_index}"),
            ));
        }
        if !dense_parameter_materialization
            .is_some_and(|fact| fact.slot == parameter.slot && fact.ty == parameter.ty)
            && !server_stream.admits_slot(parameter.slot, &parameter.ty)
            && !server_stream.admits_scalar_carrier(&parameter.ty)
            && !server_stream.admits_closure_carrier(&parameter.ty)
            && !actor_facts.is_actor_handle(&parameter.ty)
        {
            admit_type_with_registry_authority(
                units,
                unit,
                function_key,
                &parameter.ty,
                false,
                &format!("parameter {parameter_index} type"),
                host_effects.slot_authorities(parameter.slot),
                local_interface_tables,
                exact_actor_face,
            )?;
        }
        if usize::try_from(parameter.slot).ok() != Some(parameter_index + parameter_ordinal_offset)
        {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotCoverage,
                &format!(
                    "parameter {parameter_index} slot {} ordinal",
                    parameter.slot
                ),
            ));
        }
        let slot = function.slot(parameter.slot)?;
        if !parameter_slots.insert(parameter.slot) || slot.kind != MirSlotKind::Param {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotKind,
                &format!("parameter {parameter_index} slot {}", parameter.slot),
            ));
        }
        if slot.ty.as_ref() != Some(&parameter.ty) {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotCoverage,
                &format!("parameter {parameter_index} slot {} type", parameter.slot),
            ));
        }
    }
    for slot in &function.slots {
        if slot.kind == MirSlotKind::Param && !parameter_slots.contains(&slot.slot) {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::ParameterSlotCoverage,
                &format!("unbound parameter slot {}", slot.slot),
            ));
        }
        let Some(ty) = slot.ty.as_ref() else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::ValueShape,
                &format!("slot {} without an exact type", slot.slot),
            ));
        };
        if !dense_parameter_materialization
            .is_some_and(|fact| fact.slot == slot.slot && &fact.ty == ty)
            && !server_stream.admits_slot(slot.slot, ty)
            && !server_stream.admits_scalar_carrier(ty)
            && !server_stream.admits_closure_carrier(ty)
            && !actor_facts.is_actor_handle(ty)
        {
            admit_type_with_registry_authority(
                units,
                unit,
                function_key,
                ty,
                false,
                &format!("slot {} type", slot.slot),
                host_effects.slot_authorities(slot.slot),
                local_interface_tables,
                exact_actor_face,
            )?;
        }
    }
    if !function.expression_blocks.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            "value-block source facts",
        ));
    }
    admit_exception_regions(unit, function_key, function)?;
    admit_effects_with_authority(
        unit,
        function_key,
        function,
        &function.effect_summary,
        &host_effects,
    )?;
    let discriminator_literals = collect_discriminator_literal_positions(function)?;
    for expression in &function.expressions {
        admit_expression_with_host_effects(
            units,
            unit,
            function_key,
            function,
            expression,
            &discriminator_literals,
            &host_effects,
            &server_stream,
            local_interface_tables,
            actor_facts,
            provider_impl,
        )?;
    }
    for block in &function.blocks {
        for statement in &block.statements {
            admit_statement_with_authority(
                units,
                unit,
                function_key,
                function,
                statement,
                &host_effects,
                &server_stream,
                actor_facts,
            )?;
        }
    }
    if let Some(reason) = function.source_event_plan.unavailable_reason() {
        return Err(BytecodeEmissionError::Phase1SourceEventsUnavailable {
            module_path: unit.module_path.clone(),
            function_key: function_key.to_string(),
            reason,
        });
    }
    Ok(())
}

/// Admits the Phase 3 exception-region table as exact, function-local facts.
///
/// Every region must point at a `Catch` expression, carry the same slot and
/// catch type as that node, and the slot's frame type must equal the catch
/// type. Every `Catch` expression must be covered by exactly one region.
/// Missing, duplicate or drifted facts are stable typed rejections; no
/// partially admitted function escapes this boundary.
fn admit_exception_regions(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
) -> Result<(), BytecodeEmissionError> {
    let mut region_catch_exprs = BTreeSet::new();
    for (region_index, region) in function.regions.iter().enumerate() {
        let expression = function
            .expression(ExprRefIr {
                expression: region.catch_expr,
            })
            .map_err(|_| {
                exception_region_fact(
                    unit,
                    function_key,
                    &format!(
                        "region {region_index} references absent catch expression {}",
                        region.catch_expr
                    ),
                )
            })?;
        let ExprIr::Catch {
            catch_slot,
            catch_type,
            ..
        } = &expression.expression
        else {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch expression {} is not a Catch node",
                    region.catch_expr
                ),
            ));
        };
        if !region_catch_exprs.insert(region.catch_expr) {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} duplicates catch expression {}",
                    region.catch_expr
                ),
            ));
        }
        if region.catch_slot != *catch_slot {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch slot {} diverges from Catch node slot {catch_slot}",
                    region.catch_slot
                ),
            ));
        }
        if &region.catch_type != catch_type {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch type diverges from Catch node type {catch_type:?}"
                ),
            ));
        }
        let slot_type = function.slot_type(region.catch_slot).map_err(|_| {
            exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch slot {} is absent",
                    region.catch_slot
                ),
            )
        })?;
        if slot_type != catch_type {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "region {region_index} catch slot {} frame type {slot_type:?} diverges from catch type {catch_type:?}",
                    region.catch_slot
                ),
            ));
        }
    }
    for expression in &function.expressions {
        if matches!(expression.expression, ExprIr::Catch { .. })
            && !region_catch_exprs.contains(&expression.index)
        {
            return Err(exception_region_fact(
                unit,
                function_key,
                &format!(
                    "catch expression {} has no exception region",
                    expression.index
                ),
            ));
        }
    }
    Ok(())
}

fn exception_region_fact(
    unit: &MirUnit,
    function_key: &str,
    detail: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "exception region facts",
        location: format!(
            " in module `{}` function `{function_key}`: {detail}",
            unit.module_path
        ),
    }
}

fn function_contains_throw(function: &MirFunction) -> bool {
    function
        .expressions
        .iter()
        .any(|expression| matches!(expression.expression, ExprIr::Throw { .. }))
}

#[cfg(test)]
fn admit_effects(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    summary: &CallableEffectSummary,
) -> Result<(), BytecodeEmissionError> {
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .unwrap_or_default();
    admit_effects_with_authority(unit, function_key, function, summary, &host_effects)
}

fn admit_effects_with_authority(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    summary: &CallableEffectSummary,
    host_effects: &HostEffectAdmissions,
) -> Result<(), BytecodeEmissionError> {
    let effects = match summary {
        CallableEffectSummary::Unknown { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Effect,
                "unknown callable effect summary",
            ));
        }
        CallableEffectSummary::Analyzed { effects } => effects,
    };
    if !effects.inout_path_effects.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            "callable inout effects",
        ));
    }
    let has_pending_claim = effects.may_pending
        || effects.may_pending()
        || !effects.pending_effect_categories.is_empty();
    if has_pending_claim {
        // A throw inside a may-pending function remains fail-closed until
        // Phase 5 host/Pending rethrow support, and its rejection must still
        // name the throwing function rather than falling through to a tail
        // call or value-shape diagnostic.
        if function_contains_throw(function) {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::PendingEffect,
                "throw inside a may-pending function",
            ));
        }
    }
    let stream_pending = host_effects.has_stream_pending()
        || (function.stream_result.is_some()
            && function.blocks.iter().any(|block| {
                block
                    .statements
                    .iter()
                    .any(|statement| matches!(statement.kind, MirStmtKind::Emit { .. }))
            }));
    host_effects
        .validate_effect_coverage(effects, stream_pending)
        .map_err(|detail| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::PendingEffect,
                &format!("registry host-effect summary mismatch: {detail}"),
            )
        })?;
    let interface_call_conservative_effects = host_effects.has_interface_calls()
        && (effects.escapes_caller_value || effects.invokes_unknown_target);
    if effects.requires_same_heap_identity
        || (interface_call_conservative_effects && !host_effects.has_interface_calls())
        || (!host_effects.has_interface_calls()
            && (effects.escapes_caller_value || effects.invokes_unknown_target))
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Effect,
            "callable non-scalar effects",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn admit_statement(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    statement: &skiff_compiler_lowering::mir::MirStmt,
) -> Result<(), BytecodeEmissionError> {
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .unwrap_or_default();
    admit_statement_with_authority(
        units,
        unit,
        function_key,
        function,
        statement,
        &host_effects,
        &ServerStreamAdmissions::default(),
        &ActorFacts::default(),
    )
}

fn admit_statement_with_authority(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    statement: &skiff_compiler_lowering::mir::MirStmt,
    host_effects: &HostEffectAdmissions,
    server_stream: &ServerStreamAdmissions,
    actor_facts: &ActorFacts,
) -> Result<(), BytecodeEmissionError> {
    let actor_method = actor_facts.actor_for_method(function.executable_index);
    let capability = match &statement.kind {
        MirStmtKind::InitSlot { slot, value } => {
            admit_slot_value_type(
                unit,
                function_key,
                function,
                *slot,
                *value,
                Phase1MirFactMismatch::InitSlotType,
                &format!("statement {} init slot", statement.statement_index),
            )?;
            None
        }
        MirStmtKind::Expr { .. } | MirStmtKind::If { .. } => None,
        MirStmtKind::Return { value } => {
            if function.stream_result.is_some()
                && value.is_some_and(|value| !server_stream.admits_null_return(function, value))
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Stream,
                    &format!(
                        "statement {} returns a value from a server-stream producer",
                        statement.statement_index
                    ),
                ));
            }
            if let Some(value) = value.as_ref() {
                if is_tail_local_call(function, value.expression) {
                    if let Some(callee) = tail_local_call_callee(unit, function, value.expression) {
                        if callee_effect_may_pending(callee) {
                            return Err(rejected_function(
                                unit,
                                function_key,
                                Phase1UnsupportedCapability::PendingEffect,
                                &format!("tail call to pending function {}", callee.symbol),
                            ));
                        }
                    }
                    return Err(rejected_function(
                        unit,
                        function_key,
                        Phase1UnsupportedCapability::TailCall,
                        &format!("statement {}", statement.statement_index),
                    ));
                }
            }
            None
        }
        MirStmtKind::Assign { target, place, .. } => match target {
            AssignTargetIr::Slot { .. } => None,
            AssignTargetIr::ActorSelfField { field, field_type } => {
                let Some(actor_method) = actor_method else {
                    return Err(rejected_function(
                        unit,
                        function_key,
                        Phase1UnsupportedCapability::Actor,
                        &format!(
                            "statement {} actor self field outside an exact actor method",
                            statement.statement_index
                        ),
                    ));
                };
                let declaration = actor_facts
                    .actor(&actor_method.actor)
                    .expect("actor method fact joins its declaration");
                if declaration.fields.get(field) == Some(field_type) {
                    None
                } else {
                    Some(Phase1UnsupportedCapability::Actor)
                }
            }
            AssignTargetIr::Field { .. } | AssignTargetIr::Index { .. } => {
                if matches!(place.root, MirWritableRoot::ActorSelfField { .. }) {
                    if actor_method.is_none() {
                        return Err(rejected_function(
                            unit,
                            function_key,
                            Phase1UnsupportedCapability::Actor,
                            &format!(
                                "statement {} actor self write outside an exact actor method",
                                statement.statement_index
                            ),
                        ));
                    }
                    None
                } else {
                    None
                }
            }
        },
        MirStmtKind::Throw { payload_type, .. } => {
            admit_throw_payload_type(
                units,
                unit,
                function_key,
                payload_type,
                &format!("statement {} throw payload type", statement.statement_index),
            )?;
            None
        }
        MirStmtKind::Rethrow { exception_slot } => {
            function.slot(*exception_slot)?;
            None
        }
        MirStmtKind::Emit { operation, value } => {
            let Some(stream) = &function.stream_result else {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Stream,
                    &format!(
                        "statement {} emit without server-stream authority",
                        statement.statement_index
                    ),
                ));
            };
            let value_type = &function.expression(*value)?.ty;
            if !operation.is_empty()
                || !server_stream.admits_emit(
                    statement.statement_index,
                    value.expression,
                    value_type,
                )
                || !server_stream.admits_result(&stream.item_type)
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Stream,
                    &format!(
                        "statement {} emit differs from exact stream item facts",
                        statement.statement_index
                    ),
                ));
            }
            None
        }
        MirStmtKind::StreamNext { .. }
            if host_effects.admits_stream_next(statement.statement_index) =>
        {
            None
        }
        MirStmtKind::StreamNext { .. } => Some(Phase1UnsupportedCapability::Stream),
        MirStmtKind::TestEffectRegister { .. } => Some(Phase1UnsupportedCapability::HostTarget),
        MirStmtKind::ForIn { .. }
            if host_effects.admits_stream_for_in(statement.statement_index) =>
        {
            None
        }
        MirStmtKind::Dispatch { call } => {
            let expression = function.expression(*call).map_err(|error| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::ControlFlow,
                    &format!(
                        "statement {} dispatch call is absent: {error}",
                        statement.statement_index
                    ),
                )
            })?;
            let ExprIr::Call { call } = &expression.expression else {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::ControlFlow,
                    &format!(
                        "statement {} dispatch statement does not carry an exact task call",
                        statement.statement_index
                    ),
                ));
            };
            if call.metadata.contains_key(TASK_SUBMIT_METADATA_KEY) {
                None
            } else {
                Some(Phase1UnsupportedCapability::ControlFlow)
            }
        }
        MirStmtKind::Break
            if server_stream.has_exact_authority()
                && host_effects.admits_stream_break(statement.statement_index) =>
        {
            None
        }
        MirStmtKind::ForIn { .. }
        | MirStmtKind::While { .. }
        | MirStmtKind::Match { .. }
        | MirStmtKind::Break
        | MirStmtKind::Continue => Some(Phase1UnsupportedCapability::ControlFlow),
        MirStmtKind::Timeout { .. } | MirStmtKind::Concurrent { .. } => {
            Some(Phase1UnsupportedCapability::PendingEffect)
        }
        MirStmtKind::Assert { .. } => Some(Phase1UnsupportedCapability::ControlFlow),
    };
    if let Some(capability) = capability {
        return Err(rejected_function(
            unit,
            function_key,
            capability,
            &format!("statement {}", statement.statement_index),
        ));
    }
    Ok(())
}

fn tail_local_call_callee<'a>(
    unit: &'a MirUnit,
    function: &MirFunction,
    expression_index: u32,
) -> Option<&'a MirFunction> {
    let expression = function.expressions.get(expression_index as usize)?;
    let ExprIr::Call { call } = &expression.expression else {
        return None;
    };
    let CallTargetIr::LocalExecutable { executable_index } = call.target else {
        return None;
    };
    unit.function_by_executable_index(executable_index).ok()
}

fn callee_effect_may_pending(callee: &MirFunction) -> bool {
    matches!(
        &callee.effect_summary,
        CallableEffectSummary::Analyzed { effects } if effects.may_pending || effects.may_pending()
    )
}

fn is_tail_local_call(function: &MirFunction, expression_index: u32) -> bool {
    function
        .expressions
        .get(expression_index as usize)
        .is_some_and(|expression| {
            matches!(
                &expression.expression,
                ExprIr::Call { call }
                    if matches!(&call.target, CallTargetIr::LocalExecutable { .. })
                        && expression.direct_call.is_some()
            )
        })
}

/// Compile-time string literals are admitted only as union/`CatchResult`
/// discriminator constants: the right-hand operand of a `tag == "…"`
/// equality. General string values stay fail closed.
fn collect_discriminator_literal_positions(
    function: &MirFunction,
) -> Result<BTreeSet<u32>, BytecodeEmissionError> {
    let mut positions = BTreeSet::new();
    for expression in &function.expressions {
        let ExprIr::Binary {
            op: BinaryOpIr::Equal,
            left,
            right,
        } = &expression.expression
        else {
            continue;
        };
        if is_tag_field_read(function, *left)? && is_string_literal_expression(function, *right)? {
            positions.insert(right.expression);
        }
        if is_tag_field_read(function, *right)? && is_string_literal_expression(function, *left)? {
            positions.insert(left.expression);
        }
    }
    Ok(positions)
}

/// A `tag` field read is the discriminator position for a `CatchResult`
/// (or a tag-shaped named-union accessor whose result is a string-literal
/// union). Only these reads unlock string-literal type admission.
fn is_tag_field_read(
    function: &MirFunction,
    expression_ref: ExprRefIr,
) -> Result<bool, BytecodeEmissionError> {
    let expression = function.expression(expression_ref)?;
    let ExprIr::Field { object, field } = &expression.expression else {
        return Ok(false);
    };
    if field != "tag" {
        return Ok(false);
    }
    let object_type = &function.expression(*object)?.ty;
    Ok(is_catch_result_type(object_type) || is_string_literal_union(&expression.ty))
}

fn is_string_literal_expression(
    function: &MirFunction,
    expression_ref: ExprRefIr,
) -> Result<bool, BytecodeEmissionError> {
    Ok(matches!(
        function.expression(expression_ref)?.expression,
        ExprIr::Literal {
            value: LiteralIr::String { .. }
        }
    ))
}

fn is_string_literal_union(ty: &TypeRefIr) -> bool {
    let TypeRefIr::Union { items } = ty else {
        return false;
    };
    !items.is_empty()
        && items.iter().all(|item| {
            matches!(
                item,
                TypeRefIr::Literal {
                    value: LiteralIr::String { .. }
                }
            )
        })
}

fn is_string_literal_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Literal {
            value: LiteralIr::String { .. }
        }
    )
}

fn is_catch_result_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2
    )
}

/// After `result.tag == "err"` narrows a CatchResult binding, the expression
/// model retypes later loads of that binding as the err-branch record
/// `{ exception: Exception<E>, tag: "err" }`. The slot's frame type remains
/// the opaque `CatchResult<T,E>`; this admission recognizes exactly that
/// narrowed shape and rejects any other slot/load drift.
fn is_catch_result_narrowed_load(slot_type: &TypeRefIr, load_type: &TypeRefIr) -> bool {
    let TypeRefIr::Builtin { name, args } = slot_type else {
        return false;
    };
    if name != "CatchResult" || args.len() != 2 {
        return false;
    }
    let TypeRefIr::Record { fields } = load_type else {
        return false;
    };
    if fields.len() != 2 {
        return false;
    }
    let exception_type = TypeRefIr::Builtin {
        name: "Exception".to_string(),
        args: vec![args[1].clone()],
    };
    fields.get("exception") == Some(&exception_type)
        && fields
            .get("tag")
            .is_some_and(|tag| is_string_literal_type(tag) || is_string_literal_union(tag))
}

#[cfg(test)]
fn admit_expression(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    discriminator_literals: &BTreeSet<u32>,
) -> Result<(), BytecodeEmissionError> {
    let host_effects =
        HostEffectAdmissions::analyze(function, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
            .map_err(|error| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} exact host-effect admission: {}",
                        error.expression_index, error.detail
                    ),
                )
            })?;
    admit_expression_with_host_effects(
        units,
        unit,
        function_key,
        function,
        expression,
        discriminator_literals,
        &host_effects,
        &ServerStreamAdmissions::default(),
        &LocalInterfaceFacts::empty(),
        &ActorFacts::default(),
        false,
    )
}

fn admit_expression_with_host_effects(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    discriminator_literals: &BTreeSet<u32>,
    host_effects: &HostEffectAdmissions,
    server_stream: &ServerStreamAdmissions,
    local_interface_tables: &LocalInterfaceFacts,
    actor_facts: &ActorFacts,
    provider_impl: bool,
) -> Result<(), BytecodeEmissionError> {
    let registry_authorities = host_effects.expression_authorities(expression.index);
    let mut local_interface_exact_face = local_interface_tables
        .method_for_executable(function.executable_index)
        .is_some()
        || provider_impl
        || actor_facts
            .actor_for_method(function.executable_index)
            .is_some()
        || local_interface_tables.exact_call_result(
            unit,
            function,
            expression.index,
            &expression.ty,
        )
        || local_interface_tables.exact_concrete_construct_value(
            function,
            expression.index,
            &expression.ty,
        )
        || matches!(
            &expression.expression,
            ExprIr::Construct { type_ref, .. }
                if local_interface_tables.concrete_type(type_ref)
        );
    if let ExprIr::Call { call } = &expression.expression {
        if matches!(call.target, CallTargetIr::InterfaceMethod { .. })
            && call.args.first().is_some_and(|receiver| {
                local_interface_tables.remote_interface_receiver(unit, function, *receiver)
            })
        {
            local_interface_exact_face = true;
        }
        admit_call(
            unit,
            function_key,
            function,
            expression,
            call,
            host_effects,
            server_stream,
            local_interface_tables,
            actor_facts,
        )?;
    }
    if let ExprIr::Construct { type_ref, .. } = &expression.expression {
        if !server_stream.admits_construct(expression.index, type_ref)
            && !server_stream.admits_closure_carrier(type_ref)
        {
            admit_type_with_registry_authority(
                units,
                unit,
                function_key,
                type_ref,
                false,
                &format!("expression {} construct type", expression.index),
                registry_authorities,
                local_interface_tables,
                local_interface_exact_face || local_interface_tables.concrete_type(type_ref),
            )?;
        }
    }
    let discriminator_context = discriminator_literals.contains(&expression.index)
        || is_tag_field_read(
            function,
            ExprRefIr {
                expression: expression.index,
            },
        )?;
    if !server_stream.admits_expression(expression.index, &expression.ty)
        && !server_stream.admits_scalar_carrier(&expression.ty)
        && !server_stream.admits_closure_carrier(&expression.ty)
        && !registry_authorities
            .iter()
            .any(|authority| authority.admits(&expression.ty))
        && !host_effects.is_db_expression(expression.index)
        && !host_effects.is_db_body_expression(expression.index)
        && !actor_facts.is_actor_handle(&expression.ty)
        && !is_actor_registry_get_string_literal(function, expression.index)
    {
        admit_type_with_discriminator_flag(
            units,
            unit,
            function_key,
            &expression.ty,
            true,
            &format!("expression {} type", expression.index),
            discriminator_context,
            local_interface_tables,
            local_interface_exact_face,
        )?;
    }
    if expression.writable.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} writable facts", expression.index),
        ));
    }
    if let Some(stream) = &expression.stream_result {
        let exact_stream_authority = registry_authorities.iter().any(|authority| {
            authority.admits(&expression.ty)
                && matches!(
                    &expression.ty,
                    TypeRefIr::Builtin { name, args }
                        if name == "Stream" && args.as_slice() == [stream.item_type.clone()]
                )
        });
        if !exact_stream_authority {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Stream,
                &format!(
                    "expression {} stream facts lack exact producer authority",
                    expression.index
                ),
            ));
        }
    }
    if let Some(facts) = &expression.remote_interface {
        let ExprIr::InterfaceBox {
            interface,
            source:
                BoxSourceIr::Remote {
                    public_instance_key,
                    operations,
                    callee_protocol_identity,
                    ..
                },
            ..
        } = &expression.expression
        else {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Interface,
                &format!("expression {} remote interface facts", expression.index),
            ));
        };
        let exact = facts.interface == *interface
            && facts.public_instance_key == *public_instance_key
            && facts.callee_protocol_identity.as_str() == callee_protocol_identity.as_str()
            && facts.methods.len() == operations.slots.len()
            && facts
                .methods
                .iter()
                .zip(&operations.slots)
                .all(|(fact, slot)| {
                    fact.slot == slot.slot
                        && fact.method_abi_id == slot.method_abi_id
                        && fact.contract_operation_id.as_str() == slot.operation_abi_id
                });
        if !exact {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Interface,
                &format!(
                    "expression {} remote interface facts drift",
                    expression.index
                ),
            ));
        }
    }
    let capability = match &expression.expression {
        ExprIr::Literal { value } => match value {
            LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => None,
            LiteralIr::String { value }
                if discriminator_literals.contains(&expression.index)
                    || server_stream.admits_tag_literal(expression.index, value) =>
            {
                None
            }
            LiteralIr::String { .. } if local_interface_exact_face => None,
            LiteralIr::String { .. } if server_stream.admits_scalar_carrier(&expression.ty) => None,
            LiteralIr::String { .. } if host_effects.is_db_body_expression(expression.index) => {
                None
            }
            LiteralIr::String { .. }
                if is_actor_registry_get_string_literal(function, expression.index) =>
            {
                None
            }
            LiteralIr::String { .. } => Some(Phase1UnsupportedCapability::ValueShape),
        },
        ExprIr::LoadSlot { slot } => {
            let slot_type = function.slot_type(*slot)?;
            if slot_type != &expression.ty
                && !is_catch_result_narrowed_load(slot_type, &expression.ty)
                && !may_share_scalar_machine_carrier(slot_type, &expression.ty)
            {
                return Err(fact_mismatch(
                    unit,
                    function_key,
                    Phase1MirFactMismatch::LoadSlotType,
                    &format!(
                        "expression {} load slot {slot}: slot type {slot_type:?}, load type {:?}",
                        expression.index, expression.ty
                    ),
                ));
            }
            None
        }
        ExprIr::Unary { .. } => None,
        ExprIr::Binary { op, .. } => match op {
            BinaryOpIr::And | BinaryOpIr::Or => Some(Phase1UnsupportedCapability::ControlFlow),
            _ => None,
        },
        ExprIr::Call { .. } => None,
        ExprIr::LoadConst { .. } | ExprIr::LoadPackageConst { .. } => {
            Some(Phase1UnsupportedCapability::Constant)
        }
        ExprIr::ActorSelfField { field, field_type } => {
            let Some(actor_method) = actor_facts.actor_for_method(function.executable_index) else {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Actor,
                    &format!(
                        "expression {} actor self field outside an exact actor method",
                        expression.index
                    ),
                ));
            };
            let declaration = actor_facts
                .actor(&actor_method.actor)
                .expect("actor method fact joins its declaration");
            if declaration.fields.get(field) == Some(field_type) {
                None
            } else {
                Some(Phase1UnsupportedCapability::Actor)
            }
        }
        ExprIr::InterfaceBox {
            interface,
            source: BoxSourceIr::Remote { .. },
            ..
        } => {
            if expression.ty
                != (TypeRefIr::AnyInterface {
                    interface: interface.clone(),
                })
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "expression {} remote interface box type {:?} diverges from interface {interface:?}",
                        expression.index, expression.ty
                    ),
                ));
            }
            None
        }
        ExprIr::InterfaceBox {
            interface,
            source:
                BoxSourceIr::Local {
                    concrete_type,
                    method_table: _,
                },
            ..
        } => {
            if expression.ty
                != (TypeRefIr::AnyInterface {
                    interface: interface.clone(),
                })
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "expression {} local interface box type {:?} diverges from interface {interface:?}",
                        expression.index, expression.ty
                    ),
                ));
            }
            if local_interface_tables
                .table(interface, concrete_type)
                .is_none()
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "expression {} local interface table facts are missing",
                        expression.index
                    ),
                ));
            }
            None
        }
        ExprIr::Throw { payload_type, .. } => {
            admit_throw_payload_type(
                units,
                unit,
                function_key,
                payload_type,
                &format!("expression {} throw payload type", expression.index),
            )?;
            None
        }
        ExprIr::Rethrow { exception_slot } => {
            function.slot(*exception_slot)?;
            None
        }
        ExprIr::Catch {
            catch_slot,
            catch_type,
            ..
        } => {
            admit_type_with_local_facts(
                units,
                unit,
                function_key,
                catch_type,
                false,
                &format!("expression {} catch type", expression.index),
                local_interface_tables,
            )?;
            let slot_type = function.slot_type(*catch_slot).map_err(|_| {
                BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.to_string(),
                    construct: "catch slot facts",
                    location: format!(
                        " expression {} catch slot {catch_slot} is absent",
                        expression.index
                    ),
                }
            })?;
            if slot_type != catch_type {
                return Err(BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.to_string(),
                    construct: "catch slot facts",
                    location: format!(
                        " expression {} catch slot {catch_slot} frame type {slot_type:?} diverges from catch type {catch_type:?}",
                        expression.index
                    ),
                });
            }
            None
        }
        ExprIr::Timeout { .. } | ExprIr::ConcurrentValue { .. } => {
            Some(Phase1UnsupportedCapability::PendingEffect)
        }
        ExprIr::DbOperation { operation } => {
            admit_db_operation_facts(units, unit, function_key, operation)?;
            None
        }
        ExprIr::DbQuery { query } => {
            admit_db_target_facts(units, unit, function_key, &query.target)?;
            None
        }
        ExprIr::DbTransaction { .. } => None,
        ExprIr::DbLeaseClaim { claim } => {
            admit_db_target_facts(units, unit, function_key, &claim.target)?;
            None
        }
        ExprIr::DbLeaseRead { read } => {
            admit_db_target_facts(units, unit, function_key, &read.target)?;
            None
        }
        ExprIr::Field { .. }
        | ExprIr::Index { .. }
        | ExprIr::Construct { .. }
        | ExprIr::ArrayLiteral { .. } => None,
        ExprIr::RepresentationWrap { .. } | ExprIr::MapLiteral { .. } => {
            Some(Phase1UnsupportedCapability::ValueShape)
        }
        ExprIr::ValueBlock { .. }
            if host_effects.admits_db_transaction_argument(expression.index) =>
        {
            None
        }
        ExprIr::ValueBlock { .. } => Some(Phase1UnsupportedCapability::ValueShape),
    };
    if let Some(capability) = capability {
        return Err(rejected_function(
            unit,
            function_key,
            capability,
            &format!("expression {}", expression.index),
        ));
    }
    Ok(())
}

fn admit_db_operation_facts(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    operation: &skiff_artifact_model::DbOperationIr,
) -> Result<(), BytecodeEmissionError> {
    if operation.op != DbOpKindIr::Insert || operation.many {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ServiceTarget,
            "bytecode F6 facts currently admit single-object db insert only",
        ));
    }
    let body = operation
        .body
        .as_ref()
        .or(operation.insert_body.as_ref())
        .ok_or_else(|| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::ServiceTarget,
                "db insert has no object body",
            )
        })?;
    if !matches!(body, DbBodyIr::ObjectFields { .. }) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ServiceTarget,
            "bytecode F6 facts currently admit object-field insert only",
        ));
    }
    admit_db_target_facts(units, unit, function_key, &operation.target)?;
    if !matches!(operation.result_type, TypeRefIr::DbObjectSymbol { .. }) {
        admit_type_with_local_facts(
            units,
            unit,
            function_key,
            &operation.result_type,
            false,
            &format!(
                "db insert result type in module `{}` function `{function_key}`",
                unit.module_path
            ),
            &LocalInterfaceFacts::empty(),
        )?;
    }
    Ok(())
}

fn admit_db_target_facts(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    target: &DbTargetIr,
) -> Result<(), BytecodeEmissionError> {
    if target.type_name.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ServiceTarget,
            "db target carries no diagnostic type name",
        ));
    }
    if !matches!(
        target.type_ref,
        TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::DbObjectSymbol { .. }
    ) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ServiceTarget,
            &format!(
                "db target is not owner-internal local bytecode type: {:?}",
                target.type_ref
            ),
        ));
    }
    if let TypeRefIr::DbObjectSymbol { symbol } = &target.type_ref {
        if symbol.module_path != unit.module_path {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::ServiceTarget,
                &format!(
                    "db target module `{}` is not owner-internal to `{}`",
                    symbol.module_path, unit.module_path
                ),
            ));
        }
    }
    if !matches!(target.type_ref, TypeRefIr::DbObjectSymbol { .. }) {
        admit_type_with_local_facts(
            units,
            unit,
            function_key,
            &target.type_ref,
            false,
            &format!(
                "db target type in module `{}` function `{function_key}`",
                unit.module_path
            ),
            &LocalInterfaceFacts::empty(),
        )?;
    }
    Ok(())
}

fn admit_call(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    host_effects: &HostEffectAdmissions,
    server_stream: &ServerStreamAdmissions,
    local_interface_tables: &LocalInterfaceFacts,
    actor_facts: &ActorFacts,
) -> Result<(), BytecodeEmissionError> {
    if let CallTargetIr::Native { target } = &call.target {
        if target.binding_key.as_deref() == Some("std.actor.get") {
            return admit_actor_registry_get(
                unit,
                function_key,
                function,
                expression,
                call,
                target,
                actor_facts,
            );
        }
    }
    if !call.type_args.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Generic,
            &format!("expression {} call type arguments", expression.index),
        ));
    }
    if !call.inout_args.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} call inout arguments", expression.index),
        ));
    }
    if call.concrete_receiver.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            &format!("expression {} call receiver", expression.index),
        ));
    }
    if call.metadata.contains_key(TASK_SUBMIT_METADATA_KEY) {
        admit_task_submit_call(unit, function_key, function, expression, call)?;
        return Ok(());
    }
    if !call.metadata.is_empty() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!("expression {} call metadata", expression.index),
        ));
    }
    let callee = match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => unit
            .function_by_executable_index(*executable_index)
            .map_err(|_| {
                rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::NonLocalCallTarget,
                    &format!("expression {} call target", expression.index),
                )
            })?,
        CallTargetIr::PublicationExecutable { .. } | CallTargetIr::PackageCallable { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::NonLocalCallTarget,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::ServiceCall { .. } => return Ok(()),
        CallTargetIr::ServiceDependencySymbol { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::ServiceTarget,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::ActorMethod {
            actor,
            actor_abi_identity,
            actor_implementation_identity,
            method_identity,
        } => {
            if !actor_facts.exact_actor_method(
                actor,
                actor_abi_identity,
                actor_implementation_identity,
                method_identity,
            ) {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Actor,
                    &format!(
                        "expression {} actor method target is absent from the exact declaration table",
                        expression.index
                    ),
                ));
            }
            return Ok(());
        }
        CallTargetIr::Native { target } => {
            if target.binding_key.as_deref() == Some("std.db.operation") {
                if host_effects.executor_for_call(expression.index).is_none()
                    && !host_effects.has_db_calls()
                {
                    return Err(rejected_function(
                        unit,
                        function_key,
                        Phase1UnsupportedCapability::HostTarget,
                        &format!(
                            "expression {} native db call lacks exact db facts",
                            expression.index
                        ),
                    ));
                }
                return Ok(());
            }
            if target.binding_key.as_deref() == Some(CANONICAL_DURATION_MILLISECONDS_BINDING_KEY) {
                admit_duration_milliseconds_constructor(
                    unit,
                    function_key,
                    function,
                    expression,
                    call,
                    target,
                    host_effects,
                )?;
            } else if server_stream.admits_intrinsic_call(function, expression.index) {
                return Ok(());
            } else if host_effects.executor_for_call(expression.index).is_none() {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} native call lacks exact registry executor admission",
                        expression.index
                    ),
                ));
            }
            return Ok(());
        }
        CallTargetIr::ReceiverBuiltin { .. }
            if server_stream.admits_receiver_call(expression.index)
                || server_stream.admits_intrinsic_call(function, expression.index)
                || local_interface_tables
                    .exact_local_interface_string_length(unit, function, call) =>
        {
            return Ok(());
        }
        CallTargetIr::Builtin { op } if op == "db.transaction" => {
            if call.args.len() != 1 {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::PendingEffect,
                    &format!(
                        "expression {} db transaction must carry exactly one body value",
                        expression.index
                    ),
                ));
            }
            return Ok(());
        }
        CallTargetIr::Builtin { .. } | CallTargetIr::ReceiverBuiltin { .. } => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::HostTarget,
                &format!("expression {} call target", expression.index),
            ));
        }
        CallTargetIr::InterfaceMethod {
            interface,
            method_abi_id,
            slot,
        } => {
            if let Some(remote) = local_interface_tables
                .remote_interface_facts_for_call(unit, function, interface, *slot)
            {
                if remote
                    .methods
                    .iter()
                    .any(|method| method.slot == *slot && &method.method_abi_id == method_abi_id)
                {
                    return Ok(());
                }
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "expression {} remote interface call slot {slot} ABI {method_abi_id:?} is absent from exact remote table",
                        expression.index
                    ),
                ));
            }
            let table = resolve_local_interface_table_for_call(
                unit,
                function,
                call,
                local_interface_tables,
            )
            .map_err(|error| BytecodeEmissionError::UnsupportedConstruct {
                function_key: function_key.to_string(),
                construct: "local interface call target",
                location: format!(" expression {}: {}", expression.index, error),
            })?;
            let method = table
                .methods
                .iter()
                .find(|method| method.slot == *slot && &method.method_abi_id == method_abi_id)
                .ok_or_else(|| {
                    rejected_function(
                        unit,
                        function_key,
                        Phase1UnsupportedCapability::Interface,
                        &format!(
                            "expression {} interface call slot {slot} ABI {method_abi_id:?} is absent from exact local table",
                            expression.index
                        ),
                    )
                })?;
            if call.args.len() != method.signature.params.len() {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "expression {} interface call arity {} diverges from exact local row {}",
                        expression.index,
                        call.args.len(),
                        method.signature.params.len()
                    ),
                ));
            }
            if expression.ty != method.signature.return_type {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Interface,
                    &format!(
                        "expression {} interface call result type {:?} diverges from exact local row {:?}",
                        expression.index, expression.ty, method.signature.return_type
                    ),
                ));
            }
            return Ok(());
        }
        CallTargetIr::CallbackMethod {
            interface: _,
            method_abi_id,
            slot,
            methods,
        } => {
            admit_callback_method_call(
                unit,
                function_key,
                function,
                expression,
                call,
                method_abi_id,
                *slot,
                methods,
            )?;
            return Ok(());
        }
    };
    let Some(facts) = expression.direct_call.as_ref() else {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!("expression {} missing direct-call facts", expression.index),
        ));
    };
    if facts.concrete_receiver.is_some() || facts.receiver_call_abi.is_some() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Receiver,
            &format!("expression {} direct-call receiver facts", expression.index),
        ));
    }
    if facts
        .parameter_modes
        .iter()
        .any(|mode| *mode == MirParamMode::InOut)
        || facts
            .arguments
            .iter()
            .any(|argument| matches!(argument, MirCallArgument::InOut { .. }))
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::InOut,
            &format!("expression {} direct-call ABI", expression.index),
        ));
    }
    function.direct_call_facts(skiff_artifact_model::ExprRefIr {
        expression: expression.index,
    })?;
    admit_local_call_abi(
        unit,
        function_key,
        function,
        expression,
        call,
        facts,
        callee,
    )?;
    admit_local_call_source_event(unit, function_key, function, expression, call)?;
    Ok(())
}

fn admit_callback_method_call(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    method_abi_id: &str,
    selected_slot: u32,
    methods: &[CallbackInterfaceMethodIr],
) -> Result<(), BytecodeEmissionError> {
    let mut previous_slot = None;
    for method in methods {
        if previous_slot.is_some_and(|slot| slot >= method.slot) || method.method_abi_id.is_empty()
        {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Callback,
                &format!(
                    "expression {} callback method table is not dense or canonical",
                    expression.index
                ),
            ));
        }
        previous_slot = Some(method.slot);
    }
    let method = methods
        .iter()
        .find(|method| method.slot == selected_slot && method.method_abi_id == method_abi_id)
        .ok_or_else(|| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Callback,
                &format!(
                    "expression {} callback method slot {selected_slot} ABI {method_abi_id:?} is absent from exact callback rows",
                    expression.index
                ),
            )
        })?;
    if call.args.len() != method.signature.params.len() {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Callback,
            &format!(
                "expression {} callback call arity {} diverges from exact callback row {}",
                expression.index,
                call.args.len(),
                method.signature.params.len()
            ),
        ));
    }
    if expression.ty != method.signature.return_type {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Callback,
            &format!(
                "expression {} callback call result type {:?} diverges from exact callback row {:?}",
                expression.index, expression.ty, method.signature.return_type
            ),
        ));
    }
    let receiver = call.args.first().copied().ok_or_else(|| {
        rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Callback,
            &format!(
                "expression {} callback call has no carrier argument",
                expression.index
            ),
        )
    })?;
    let receiver_expression = function.expression(receiver)?;
    let ExprIr::LoadSlot { slot } = receiver_expression.expression else {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Callback,
            &format!(
                "expression {} callback carrier must be an exact callback parameter slot",
                expression.index
            ),
        ));
    };
    let receiver_slot = function.slot(slot)?;
    if receiver_slot.kind != MirSlotKind::Param
        || !matches!(
            &receiver_slot.ty,
            Some(TypeRefIr::AnyInterface { interface, .. })
                if interface.canonical_type_args.is_empty()
        )
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Callback,
            &format!(
                "expression {} callback carrier is not an exact non-generic any-interface parameter",
                expression.index
            ),
        ));
    }
    Ok(())
}

/// Phase 4 gate 1 companion: admits the pure `Duration.milliseconds`
/// constructor only when its exact argument and result stay on the pinned
/// sleep argument face. It is not a host effect, does not carry Pending, and
/// remains emitted as a synchronous constant/identity operation by the
/// bytecode emitter rather than an `InvokeHost` adapter.
fn admit_duration_milliseconds_constructor(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    target: &NativeTarget,
    host_effects: &HostEffectAdmissions,
) -> Result<(), BytecodeEmissionError> {
    if target.namespace != "Duration"
        || target.symbol != "milliseconds"
        || target.binding_key.as_deref() != Some(CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
        || !target.metadata.is_empty()
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds target identity is not exact",
                expression.index
            ),
        ));
    }
    if call.args.len() != 1 {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds arity {} (pinned arity is exactly one integer argument)",
                expression.index,
                call.args.len()
            ),
        ));
    }
    let argument = function.expression(call.args[0])?;
    let argument_type = &argument.ty;
    if !matches!(
        &argument.expression,
        ExprIr::Literal {
            value: LiteralIr::Number { .. }
        }
    ) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds argument must be a literal integer",
                expression.index
            ),
        ));
    }
    if !matches!(
        argument_type,
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty()
    ) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds argument type {argument_type:?} is not the pinned integer",
                expression.index
            ),
        ));
    }
    if !host_effects.admits_duration_constructor(expression.index, &expression.ty) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} Duration.milliseconds lacks the exact Sleep parameter closure",
                expression.index
            ),
        ));
    }
    Ok(())
}

fn admit_actor_registry_get(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &CallIr,
    target: &NativeTarget,
    actor_facts: &ActorFacts,
) -> Result<(), BytecodeEmissionError> {
    if target.namespace != "std.actor"
        || target.symbol != "get"
        || target.binding_key.as_deref() != Some("std.actor.get")
        || !target.metadata.is_empty()
        || !call.inout_args.is_empty()
        || !call.metadata.is_empty()
        || call.concrete_receiver.is_some()
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "expression {} std.actor.get target facts are not exact",
                expression.index
            ),
        ));
    }
    let actor = match call.type_args.get("T0") {
        Some(TypeRefIr::ServiceSymbol { symbol }) => symbol,
        _ => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::Actor,
                &format!(
                    "expression {} std.actor.get lacks an exact actor type argument",
                    expression.index
                ),
            ));
        }
    };
    let declaration = actor_facts.actor(actor).ok_or_else(|| {
        rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "expression {} std.actor.get actor `{}` is absent from the exact declaration table",
                expression.index,
                actor.symbol_path()
            ),
        )
    })?;
    if call.type_args.len() != 2
        || call.type_args.get("T1") != Some(&declaration.actor_id_type())
        || call.type_args.contains_key("T2")
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "expression {} std.actor.get type arguments drift from the exact actor declaration",
                expression.index
            ),
        ));
    }
    if call.args.len() != 1 {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "expression {} std.actor.get arity is not exactly one",
                expression.index
            ),
        ));
    }
    let argument_type = &function.expression(call.args[0])?.ty;
    if argument_type != &declaration.actor_id_type()
        && !is_actor_registry_get_string_literal(function, call.args[0].expression)
    {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "expression {} std.actor.get argument type drifts from the exact actor key type",
                expression.index
            ),
        ));
    }
    if !actor_facts.is_actor_handle(&expression.ty) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::Actor,
            &format!(
                "expression {} std.actor.get result drifts from the exact actor handle",
                expression.index
            ),
        ));
    }
    Ok(())
}

fn is_actor_registry_get_string_literal(function: &MirFunction, expression_index: u32) -> bool {
    let expression = match function.expression(ExprRefIr {
        expression: expression_index,
    }) {
        Ok(expression) => expression,
        Err(_) => return false,
    };
    if !matches!(
        &expression.expression,
        ExprIr::Literal {
            value: LiteralIr::String { .. }
        }
    ) {
        return false;
    }
    function.expressions.iter().any(|candidate| {
        let ExprIr::Call { call } = &candidate.expression else {
            return false;
        };
        is_std_actor_registry_get_target(call)
            && call
                .args
                .iter()
                .any(|argument| argument.expression == expression_index)
    })
}

fn is_std_actor_registry_get_target(call: &CallIr) -> bool {
    match &call.target {
        CallTargetIr::Native { target } => target.binding_key.as_deref() == Some("std.actor.get"),
        CallTargetIr::PackageCallable {
            package_callable_id,
            ..
        } => package_callable_id.as_str().ends_with(":std.actor.get"),
        _ => false,
    }
}

fn admit_task_submit_call(
    unit: &MirUnit,
    function_key: &str,
    _function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
) -> Result<(), BytecodeEmissionError> {
    let is_task_ref = matches!(
        &expression.ty,
        TypeRefIr::Builtin { name, args } if name == "TaskRef" && args.is_empty()
    );
    let is_discarded_statement = matches!(
        &expression.ty,
        TypeRefIr::Builtin { name, args }
            if args.is_empty() && matches!(name.as_str(), "void" | "null")
    );
    if !is_task_ref && !is_discarded_statement {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::HostTarget,
            &format!(
                "expression {} task submit must produce std.task.TaskRef or be a discarded void dispatch",
                expression.index
            ),
        ));
    }
    let metadata = call.metadata.get(TASK_SUBMIT_METADATA_KEY).ok_or_else(|| {
        rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!(
                "expression {} task submit metadata is absent",
                expression.index
            ),
        )
    })?;
    let MetadataValue::Object(metadata) = metadata else {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!(
                "expression {} task submit metadata must be an object",
                expression.index
            ),
        ));
    };
    let target_kind = metadata
        .get("targetKind")
        .and_then(|value| match value {
            MetadataValue::String(value) => Some(value.as_str()),
            _ => None,
        })
        .ok_or_else(|| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::NonLocalCallTarget,
                &format!(
                    "expression {} task submit targetKind is missing",
                    expression.index
                ),
            )
        })?;
    admit_task_timing_shape(unit, function_key, expression, metadata)?;
    match target_kind {
        "function" => {
            let CallTargetIr::LocalExecutable { executable_index } = call.target else {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::NonLocalCallTarget,
                    &format!(
                        "expression {} function task target must be owner-local",
                        expression.index
                    ),
                ));
            };
            let callee = unit
                .function_by_executable_index(executable_index)
                .map_err(|_| {
                    rejected_function(
                        unit,
                        function_key,
                        Phase1UnsupportedCapability::NonLocalCallTarget,
                        &format!("expression {} task target is absent", expression.index),
                    )
                })?;
            if callee.kind != MirExecutableKind::Function
                || !callee.type_params.is_empty()
                || !matches!(
                    &callee.return_type,
                    TypeRefIr::Builtin { name, args }
                        if args.is_empty() && matches!(name.as_str(), "void" | "null")
                )
                || call.args.len() != callee.params.len()
            {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::HostTarget,
                    &format!(
                        "expression {} task function target has invalid ABI",
                        expression.index
                    ),
                ));
            }
        }
        "actorMethod" => {
            if !matches!(call.target, CallTargetIr::ActorMethod { .. }) {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::Actor,
                    &format!(
                        "expression {} actor task target must be an actor method",
                        expression.index
                    ),
                ));
            }
        }
        other => {
            return Err(rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::NonLocalCallTarget,
                &format!(
                    "expression {} task submit target kind {other} is unsupported",
                    expression.index
                ),
            ));
        }
    }
    Ok(())
}

fn admit_task_timing_shape(
    unit: &MirUnit,
    function_key: &str,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    metadata: &BTreeMap<String, MetadataValue>,
) -> Result<(), BytecodeEmissionError> {
    let Some(timing) = metadata.get("timing") else {
        return Ok(());
    };
    let MetadataValue::Object(timing) = timing else {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!(
                "expression {} task timing must be an object",
                expression.index
            ),
        ));
    };
    let kind = timing
        .get("kind")
        .and_then(|value| match value {
            MetadataValue::String(value) => Some(value.as_str()),
            _ => None,
        })
        .ok_or_else(|| {
            rejected_function(
                unit,
                function_key,
                Phase1UnsupportedCapability::NonLocalCallTarget,
                &format!(
                    "expression {} task timing kind is missing",
                    expression.index
                ),
            )
        })?;
    match kind {
        "immediate" => Ok(()),
        "after" | "at" => {
            let present = timing
                .get("expr")
                .is_some_and(|value| matches!(value, MetadataValue::Number(_)));
            if present {
                Ok(())
            } else {
                Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::NonLocalCallTarget,
                    &format!(
                        "expression {} task timing {kind} requires an expression",
                        expression.index
                    ),
                ))
            }
        }
        other => Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::NonLocalCallTarget,
            &format!(
                "expression {} task timing kind {other} is unsupported",
                expression.index
            ),
        )),
    }
}

fn admit_local_call_abi(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
    facts: &skiff_compiler_lowering::mir::MirDirectCallFacts,
    callee: &MirFunction,
) -> Result<(), BytecodeEmissionError> {
    let parameter_count = callee.params.len();
    if facts.parameter_modes.len() != parameter_count
        || facts.arguments.len() != parameter_count
        || call.args.len() != parameter_count
    {
        return Err(fact_mismatch(
            unit,
            function_key,
            Phase1MirFactMismatch::LocalCallParameterCount,
            &format!("expression {} local call", expression.index),
        ));
    }
    for (parameter_index, parameter) in callee.params.iter().enumerate() {
        if facts.parameter_modes[parameter_index] != parameter.mode {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallParameterMode,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        }
        let MirCallArgument::Value { value } = &facts.arguments[parameter_index] else {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallArgument,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        };
        if call.args[parameter_index] != *value {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallArgument,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        }
        let argument = function.expression(*value)?;
        if argument.ty != parameter.ty {
            return Err(fact_mismatch(
                unit,
                function_key,
                Phase1MirFactMismatch::LocalCallArgumentType,
                &format!(
                    "expression {} local call parameter {parameter_index}",
                    expression.index
                ),
            ));
        }
    }
    if expression.ty != callee.return_type {
        return Err(fact_mismatch(
            unit,
            function_key,
            Phase1MirFactMismatch::LocalCallResultType,
            &format!("expression {} local call result", expression.index),
        ));
    }
    Ok(())
}

fn admit_local_call_source_event(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    expression: &skiff_compiler_lowering::mir::MirExpression,
    call: &skiff_artifact_model::CallIr,
) -> Result<(), BytecodeEmissionError> {
    let Some(events) = function.source_event_plan.events() else {
        return Ok(());
    };
    let mut matches = events.iter().filter(|event| {
        matches!(
            (event.attribution_id, event.anchor),
            (
                StatementAttributionId::Expression {
                    expression_index,
                    occurrence_ordinal: 0,
                },
                MirEmissionAnchor::LocalCall {
                    expression_index: anchor_expression,
                    occurrence_ordinal: 0,
                }
                | MirEmissionAnchor::TailLocalCallCandidate {
                    expression_index: anchor_expression,
                    occurrence_ordinal: 0,
                    ..
                },
            ) if expression_index == expression.index && anchor_expression == expression.index
        ) && event.site == call.site
    });
    if matches.next().is_none() || matches.next().is_some() {
        return Err(fact_mismatch(
            unit,
            function_key,
            Phase1MirFactMismatch::LocalCallSourceEvent,
            &format!("expression {} local call source event", expression.index),
        ));
    }
    Ok(())
}

fn admit_slot_value_type(
    unit: &MirUnit,
    function_key: &str,
    function: &MirFunction,
    slot: u32,
    value: skiff_artifact_model::ExprRefIr,
    mismatch: Phase1MirFactMismatch,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let slot_type = function.slot_type(slot)?;
    let value_type = &function.expression(value)?.ty;
    if slot_type != value_type {
        return Err(fact_mismatch(unit, function_key, mismatch, location));
    }
    Ok(())
}

/// Phase 3 throw payload leaves must carry a runtime catch identity: local
/// nominal record types and anonymous unions whose branches are nominal
/// records. Structural, scalar and literal-branch leaves have no runtime
/// identity and fail closed here instead of reaching a constant VmFailure.
/// This tightens the throw face only; catch/rethrow admission is unchanged.
fn admit_throw_payload_type(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    payload_type: &TypeRefIr,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    match payload_type {
        TypeRefIr::Union { items } => {
            if items.is_empty() {
                return Err(rejected_function(
                    unit,
                    function_key,
                    Phase1UnsupportedCapability::ValueShape,
                    &format!("{location} empty union"),
                ));
            }
            for item in items {
                admit_throw_payload_type(
                    units,
                    unit,
                    function_key,
                    item,
                    &format!("{location} union branch"),
                )?;
            }
            Ok(())
        }
        TypeRefIr::LocalType { type_index } => admit_nominal_record_leaf(
            units,
            unit,
            function_key,
            &unit.module_path,
            *type_index,
            location,
        ),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => admit_nominal_record_leaf(
            units,
            unit,
            function_key,
            module_path,
            *type_index,
            location,
        ),
        other => Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            &format!("{location} leaf {other:?} has no runtime catch identity"),
        )),
    }
}

fn admit_nominal_record_leaf(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    module_path: &str,
    type_index: u32,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let owning_unit = units
        .iter()
        .find(|candidate| candidate.module_path == module_path)
        .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
            context: format!("throw payload nominal leaf admission for module `{module_path}`"),
            message: "owning MIR unit disappeared".to_string(),
        })?;
    let declaration = owning_unit
        .type_table
        .get(type_index as usize)
        .ok_or_else(|| BytecodeEmissionError::MissingLocalType {
            module_path: module_path.to_string(),
            location: location.to_string(),
            type_index,
            type_count: owning_unit.type_table.len(),
        })?;
    if !matches!(declaration.descriptor, TypeDescriptorIr::Record { .. }) {
        return Err(rejected_function(
            unit,
            function_key,
            Phase1UnsupportedCapability::ValueShape,
            &format!(
                "{location} nominal `{}` is not a record leaf",
                declaration.name
            ),
        ));
    }
    Ok(())
}

fn admit_type(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    admit_type_with_local_facts(
        units,
        unit,
        function_key,
        ty,
        allow_void,
        location,
        &LocalInterfaceFacts::empty(),
    )
}

fn admit_type_with_local_facts(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    local_interface_tables: &LocalInterfaceFacts,
) -> Result<(), BytecodeEmissionError> {
    admit_type_with_discriminator_flag(
        units,
        unit,
        function_key,
        ty,
        allow_void,
        location,
        false,
        local_interface_tables,
        false,
    )
}

fn admit_type_with_exact_local_interface_face(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    local_interface_tables: &LocalInterfaceFacts,
) -> Result<(), BytecodeEmissionError> {
    admit_type_with_discriminator_flag(
        units,
        unit,
        function_key,
        ty,
        allow_void,
        location,
        false,
        local_interface_tables,
        true,
    )
}

fn admit_type_with_registry_authority(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    authorities: &[RegistryValueAuthority],
    local_interface_tables: &LocalInterfaceFacts,
    exact_local_interface_face: bool,
) -> Result<(), BytecodeEmissionError> {
    if authorities.iter().any(|authority| authority.admits(ty)) {
        return Ok(());
    }
    if exact_local_interface_face {
        admit_type_with_exact_local_interface_face(
            units,
            unit,
            function_key,
            ty,
            allow_void,
            location,
            local_interface_tables,
        )
    } else {
        admit_type_with_local_facts(
            units,
            unit,
            function_key,
            ty,
            allow_void,
            location,
            local_interface_tables,
        )
    }
}

fn admit_type_with_discriminator_flag(
    units: &[MirUnit],
    unit: &MirUnit,
    function_key: &str,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    allow_discriminator_literal: bool,
    local_interface_tables: &LocalInterfaceFacts,
    exact_local_interface_face: bool,
) -> Result<(), BytecodeEmissionError> {
    let mut context = TypeAdmissionContext {
        units,
        function_key,
        nominal_chain: Vec::new(),
        allow_discriminator_literal,
        local_interface_tables,
        exact_local_interface_face,
    };
    admit_type_nested(&mut context, unit, ty, allow_void, location, false)
}

/// Recursive Phase 2 type admission with a nominal-recursion guard.
///
/// `nested` distinguishes a record/array leaf from a top-level type: out-of-
/// surface nested leaves carry the stable Phase 2 record/array rejection,
/// while top-level rejections keep the legacy capability diagnostics.
struct TypeAdmissionContext<'a> {
    units: &'a [MirUnit],
    function_key: &'a str,
    nominal_chain: Vec<(String, u32)>,
    allow_discriminator_literal: bool,
    local_interface_tables: &'a LocalInterfaceFacts,
    exact_local_interface_face: bool,
}

fn admit_type_nested(
    context: &mut TypeAdmissionContext<'_>,
    unit: &MirUnit,
    ty: &TypeRefIr,
    allow_void: bool,
    location: &str,
    nested: bool,
) -> Result<(), BytecodeEmissionError> {
    match ty {
        TypeRefIr::Record { fields } => {
            for (name, field_ty) in fields {
                let saved_flag = context.allow_discriminator_literal;
                if name == "tag"
                    && (is_string_literal_type(field_ty) || is_string_literal_union(field_ty))
                {
                    context.allow_discriminator_literal = true;
                }
                let result = admit_type_nested(
                    context,
                    unit,
                    field_ty,
                    false,
                    &format!("{location} field `{name}`"),
                    true,
                );
                context.allow_discriminator_literal = saved_flag;
                result?;
            }
            Ok(())
        }
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
            admit_type_nested(
                context,
                unit,
                &args[0],
                false,
                &format!("{location} element type"),
                true,
            )
        }
        TypeRefIr::Builtin { name, args }
            if args.is_empty()
                && matches!(name.as_str(), "TaskRef" | "TaskStatus" | "TaskCancelResult") =>
        {
            Ok(())
        }
        TypeRefIr::Union { items } => {
            for item in items {
                admit_type_nested(
                    context,
                    unit,
                    item,
                    false,
                    &format!("{location} union leaf"),
                    nested,
                )?;
            }
            Ok(())
        }
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            admit_type_nested(
                context,
                unit,
                &args[0],
                true,
                &format!("{location} result type"),
                nested,
            )?;
            admit_type_nested(
                context,
                unit,
                &args[1],
                false,
                &format!("{location} error type"),
                nested,
            )
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            admit_type_nested(
                context,
                unit,
                &args[0],
                false,
                &format!("{location} payload type"),
                nested,
            )
        }
        // Compile-time string literals are admitted only inside a
        // discriminator context (a `.tag` result union or the constant side
        // of a `tag == "…"` equality). Everywhere else they stay on the
        // rejected Phase 2 value-shape face.
        TypeRefIr::Literal {
            value: LiteralIr::String { .. },
        } if context.allow_discriminator_literal || context.exact_local_interface_face => Ok(()),
        TypeRefIr::Builtin { name, args }
            if name == "string" && args.is_empty() && context.exact_local_interface_face =>
        {
            Ok(())
        }
        TypeRefIr::AnyInterface { interface } => {
            if !nested
                && interface.canonical_type_args.is_empty()
                && !interface.interface_abi_id.trim().is_empty()
            {
                return Ok(());
            }
            if context
                .local_interface_tables
                .tables_for_interface(interface)
                .is_empty()
            {
                return if nested {
                    Err(phase_2_nested_shape_rejection(
                        context.function_key,
                        Phase1UnsupportedCapability::Interface,
                        location,
                    ))
                } else {
                    Err(rejected_function(
                        unit,
                        context.function_key,
                        Phase1UnsupportedCapability::Interface,
                        location,
                    ))
                };
            }
            for argument in &interface.canonical_type_args {
                admit_type_nested(
                    context,
                    unit,
                    argument,
                    false,
                    &format!("{location} interface type argument"),
                    true,
                )?;
            }
            Ok(())
        }
        TypeRefIr::LocalType { type_index } => {
            admit_nominal_declaration(context, unit, &unit.module_path, *type_index, location)
        }
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => admit_nominal_declaration(context, unit, module_path, *type_index, location),
        _ => {
            if let Some(capability) = unsupported_type_capability(ty, allow_void) {
                return if nested {
                    Err(phase_2_nested_shape_rejection(
                        context.function_key,
                        capability,
                        location,
                    ))
                } else {
                    Err(rejected_function(
                        unit,
                        context.function_key,
                        capability,
                        location,
                    ))
                };
            }
            Ok(())
        }
    }
}

/// Recursively admits one nominal declaration (record, representation, named
/// union or transparent alias) against the Phase 2 value face, with a
/// nominal-recursion guard. Interface declarations stay rejected.
fn admit_nominal_declaration(
    context: &mut TypeAdmissionContext<'_>,
    unit: &MirUnit,
    module_path: &str,
    type_index: u32,
    location: &str,
) -> Result<(), BytecodeEmissionError> {
    let key = (module_path.to_string(), type_index);
    if context.nominal_chain.contains(&key) {
        return Err(phase_2_nested_shape_rejection(
            context.function_key,
            Phase1UnsupportedCapability::ValueShape,
            &format!("{location} (recursive record reference)"),
        ));
    }
    let owning_unit = context
        .units
        .iter()
        .find(|candidate| candidate.module_path == module_path)
        .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
            context: format!("Phase 2 record admission for module `{module_path}`"),
            message: "owning MIR unit disappeared".to_string(),
        })?;
    let declaration = owning_unit
        .type_table
        .get(type_index as usize)
        .ok_or_else(|| BytecodeEmissionError::MissingLocalType {
            module_path: module_path.to_string(),
            location: location.to_string(),
            type_index,
            type_count: owning_unit.type_table.len(),
        })?;
    context.nominal_chain.push(key);
    let result = match &declaration.descriptor {
        TypeDescriptorIr::Record { fields } => {
            for (name, field_ty) in fields {
                admit_type_nested(
                    context,
                    owning_unit,
                    field_ty,
                    false,
                    &format!("{location} field `{name}`"),
                    true,
                )?;
            }
            Ok(())
        }
        TypeDescriptorIr::Representation { representation } => admit_type_nested(
            context,
            owning_unit,
            representation,
            false,
            &format!("{location} representation"),
            true,
        ),
        TypeDescriptorIr::Union { branches } => {
            for branch in branches {
                match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => admit_type_nested(
                        context,
                        owning_unit,
                        nominal_type,
                        false,
                        &format!("{location} union branch"),
                        true,
                    )?,
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        admit_type_nested(
                            context,
                            owning_unit,
                            payload_type,
                            false,
                            &format!("{location} union branch"),
                            true,
                        )?
                    }
                    NamedUnionBranchIr::Literal { value } => admit_type_nested(
                        context,
                        owning_unit,
                        &TypeRefIr::Literal {
                            value: value.clone(),
                        },
                        false,
                        &format!("{location} union branch"),
                        true,
                    )?,
                }
            }
            Ok(())
        }
        TypeDescriptorIr::Alias { target } => admit_type_nested(
            context,
            owning_unit,
            target,
            false,
            &format!("{location} alias target"),
            true,
        ),
        TypeDescriptorIr::Interface => Err(rejected_function(
            unit,
            context.function_key,
            Phase1UnsupportedCapability::ValueShape,
            location,
        )),
    };
    context.nominal_chain.pop();
    result
}

fn phase_2_nested_shape_rejection(
    function_key: &str,
    capability: Phase1UnsupportedCapability,
    location: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "phase 2 record/array value shape",
        location: format!(" {location} ({capability:?})"),
    }
}

fn unsupported_type_capability(
    ty: &TypeRefIr,
    allow_void: bool,
) -> Option<Phase1UnsupportedCapability> {
    match ty {
        // Throw/rethrow expressions are typed `never`: the uninhabited type is
        // admitted only where the language itself places it (expression/result
        // positions), never as a data-shape leaf.
        TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty() && allow_void => {
            None
        }
        TypeRefIr::Builtin { name, args }
            if args.is_empty()
                && (matches!(name.as_str(), "integer" | "number" | "bool" | "null")
                    || matches!(name.as_str(), "TaskRef" | "TaskStatus" | "TaskCancelResult")
                    || (allow_void && name == "void")) =>
        {
            None
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => None,
            LiteralIr::String { .. } => Some(Phase1UnsupportedCapability::ValueShape),
        },
        TypeRefIr::TypeParam { .. } | TypeRefIr::AppliedNominal { .. } => {
            Some(Phase1UnsupportedCapability::Generic)
        }
        TypeRefIr::Function { .. } => Some(Phase1UnsupportedCapability::Callback),
        TypeRefIr::ServiceSymbol { .. } | TypeRefIr::DbObjectSymbol { .. } => {
            Some(Phase1UnsupportedCapability::ServiceTarget)
        }
        _ => Some(Phase1UnsupportedCapability::ValueShape),
    }
}

fn rejected_function(
    unit: &MirUnit,
    function_key: &str,
    capability: Phase1UnsupportedCapability,
    location: &str,
) -> BytecodeEmissionError {
    rejected(unit, Some(function_key), capability, location)
}

fn fact_mismatch(
    unit: &MirUnit,
    function_key: &str,
    mismatch: Phase1MirFactMismatch,
    location: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::Phase1MirFactMismatch {
        mismatch,
        module_path: unit.module_path.clone(),
        function_key: function_key.to_string(),
        location: location.to_string(),
    }
}

fn rejected(
    unit: &MirUnit,
    function_key: Option<&str>,
    capability: Phase1UnsupportedCapability,
    location: &str,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedPhase1Capability {
        capability,
        module_path: unit.module_path.clone(),
        function_key: function_key.map(str::to_string),
        location: location.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        CallIr, CallTargetIr, CallableEffectSummary, CallableMayEffects, ExprIr, ExprRefIr,
        FileIrUnit, InstructionSourceSite, LiteralIr, NativeTarget, PackageCallableId,
        PackageRefIr, PackageSymbolRef, PendingEffectCategory, SyntheticInstructionSiteReason,
        TypeDeclIr, TypeDescriptorIr, TypeRefIr,
    };
    use skiff_compiler_lowering::mir::{
        MirBlock, MirExecutableKind, MirExpression, MirFunction, MirLiveness, MirRegion, MirSlot,
        MirSlotKind, MirSourceEventPlan, MirSourceEventUnavailableReason, MirStmt, MirStmtKind,
        MirUnit,
    };

    use super::*;
    use crate::Phase1UnsupportedCapability;

    const FUNCTION_KEY: &str = "main::run";

    fn number() -> TypeRefIr {
        TypeRefIr::builtin("number")
    }

    fn local(index: u32) -> TypeRefIr {
        TypeRefIr::LocalType { type_index: index }
    }

    fn union(items: Vec<TypeRefIr>) -> TypeRefIr {
        TypeRefIr::Union { items }
    }

    fn record_declaration(name: &str, fields: BTreeMap<String, TypeRefIr>) -> TypeDeclIr {
        TypeDeclIr {
            name: name.to_string(),
            descriptor: TypeDescriptorIr::Record { fields },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }
    }

    fn slot(index: u32, ty: TypeRefIr) -> MirSlot {
        MirSlot {
            slot: index,
            name: format!("slot{index}"),
            kind: MirSlotKind::Local,
            writable_local: false,
            ty: Some(ty),
        }
    }

    fn expression(index: u32, expression: ExprIr, ty: TypeRefIr) -> MirExpression {
        MirExpression {
            index,
            expression,
            ty,
            writable: None,
            direct_call: None,
            stream_result: None,
            remote_interface: None,
        }
    }

    fn statement(index: u32, kind: MirStmtKind) -> MirStmt {
        MirStmt {
            statement_index: index,
            span: None,
            kind,
        }
    }

    fn synthetic_site() -> InstructionSourceSite {
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerDesugaring,
        }
    }

    fn function() -> MirFunction {
        MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "file:main".to_string(),
                module_path: "main".to_string(),
                executable_index: 0,
            },
            symbol: "main.run".to_string(),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            receiver: None,
            slots: Vec::new(),
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: Vec::new(),
            blocks: Vec::new(),
            regions: Vec::new(),
            statements: Vec::new(),
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new("callable:main:run".to_string()),
            effect_summary: CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
                },
            },
            source_span: None,
        }
    }

    fn unit(functions: Vec<MirFunction>, type_table: Vec<TypeDeclIr>) -> MirUnit {
        let mut file_ir = FileIrUnit::empty("main", "source-hash");
        file_ir.file_ir_identity = "file:main".to_string();
        file_ir.type_table = type_table;
        MirUnit {
            file_ir_identity: file_ir.file_ir_identity,
            package_id: "test.package".to_string(),
            module_path: file_ir.module_path,
            actor_declarations: file_ir.actor_declarations,
            external_refs: file_ir.external_refs,
            source_map: file_ir.source_map,
            type_table: file_ir.type_table,
            package_type_records: BTreeMap::new(),
            link_targets: file_ir.link_targets,
            constants: Vec::new(),
            functions,
        }
    }

    fn two_nominal_types() -> Vec<TypeDeclIr> {
        vec![
            record_declaration("A", BTreeMap::from([("x".to_string(), number())])),
            record_declaration("B", BTreeMap::from([("y".to_string(), number())])),
        ]
    }

    fn canonical_duration_type() -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                symbol_path: "std.time.Duration".to_string(),
                abi_expectation: Some("std-abi-fixture".to_string()),
            },
        }
    }

    fn native_target(namespace: &str, symbol: &str, binding_key: Option<&str>) -> NativeTarget {
        NativeTarget {
            namespace: namespace.to_string(),
            symbol: symbol.to_string(),
            binding_key: binding_key.map(str::to_string),
            metadata: BTreeMap::new(),
        }
    }

    fn native_call(target: NativeTarget, args: Vec<ExprRefIr>) -> CallIr {
        CallIr {
            target: CallTargetIr::Native { target },
            concrete_receiver: None,
            site: synthetic_site(),
            args,
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        }
    }

    fn sleep_pending_effects() -> CallableMayEffects {
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::NativeCall],
            inout_path_effects: Vec::new(),
        }
    }

    fn sleep_call_function(duration: TypeRefIr, call: CallIr, call_type: TypeRefIr) -> MirFunction {
        let mut function = function();
        function.slots.push(slot(0, duration.clone()));
        function.expressions.push(expression(
            0,
            ExprIr::LoadSlot { slot: 0 },
            duration.clone(),
        ));
        function
            .expressions
            .push(expression(1, ExprIr::Call { call }, call_type));
        function.effect_summary = CallableEffectSummary::Analyzed {
            effects: sleep_pending_effects(),
        };
        function
    }

    #[test]
    fn phase_4_admission_admits_canonical_sleep_call_and_its_pending_trace() {
        let duration = canonical_duration_type();
        let function = sleep_call_function(
            duration.clone(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                vec![ExprRefIr { expression: 0 }],
            ),
            TypeRefIr::builtin("void"),
        );
        let units = [unit(vec![function], Vec::new())];
        let function = &units[0].functions[0];

        admit_effects(&units[0], FUNCTION_KEY, function, &function.effect_summary)
            .expect("the canonical sleep pending trace is admitted");
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            function,
            &function.expressions[0],
            &BTreeSet::new(),
        )
        .expect("the pinned Duration argument type is admitted");
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            function,
            &function.expressions[1],
            &BTreeSet::new(),
        )
        .expect("the canonical sleep call is admitted");
    }

    #[test]
    fn phase_4_admission_rejects_other_host_binding_with_typed_error() {
        let duration = canonical_duration_type();
        let function = sleep_call_function(
            duration.clone(),
            native_call(
                native_target(
                    "Duration",
                    "milliseconds",
                    Some("core.duration.milliseconds"),
                ),
                vec![ExprRefIr { expression: 0 }],
            ),
            duration,
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("non-sleep host bindings stay rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_sleep_wrong_arity() {
        let function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                Vec::new(),
            ),
            TypeRefIr::builtin("void"),
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("sleep with zero arguments stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_sleep_wrong_argument_type() {
        let mut function = function();
        function.slots.push(slot(0, number()));
        function
            .expressions
            .push(expression(0, ExprIr::LoadSlot { slot: 0 }, number()));
        function.expressions.push(expression(
            1,
            ExprIr::Call {
                call: native_call(
                    native_target("std.time", "sleep", Some("std.time.sleep")),
                    vec![ExprRefIr { expression: 0 }],
                ),
            },
            TypeRefIr::builtin("void"),
        ));
        function.effect_summary = CallableEffectSummary::Analyzed {
            effects: sleep_pending_effects(),
        };
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("a non-Duration sleep argument stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_sleep_wrong_result_type() {
        let function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                vec![ExprRefIr { expression: 0 }],
            ),
            number(),
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("a non-void sleep result stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_native_target_without_binding_key() {
        let function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", None),
                vec![ExprRefIr { expression: 0 }],
            ),
            TypeRefIr::builtin("void"),
        );
        let units = [unit(vec![function], Vec::new())];
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &units[0].functions[0],
            &units[0].functions[0].expressions[1],
            &BTreeSet::new(),
        )
        .expect_err("a native target without a binding key stays rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_rejects_drifted_pending_trace() {
        let units = [unit(Vec::new(), Vec::new())];
        let summary = CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: Vec::new(),
                inout_path_effects: Vec::new(),
            },
        };
        let function = function();
        let error = admit_effects(&units[0], FUNCTION_KEY, &function, &summary)
            .expect_err("a mayPending flag without a category trace is a drifted fact");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::PendingEffect,
                ..
            }
        ));
    }

    #[test]
    fn phase_4_admission_admits_canonical_sleep_native_call_pending_trace() {
        let summary = CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::NativeCall],
                inout_path_effects: Vec::new(),
            },
        };
        let mut function = sleep_call_function(
            canonical_duration_type(),
            native_call(
                native_target("std.time", "sleep", Some("std.time.sleep")),
                vec![ExprRefIr { expression: 0 }],
            ),
            TypeRefIr::builtin("void"),
        );
        function.effect_summary = summary.clone();
        let units = [unit(vec![function], Vec::new())];
        admit_effects(&units[0], FUNCTION_KEY, &units[0].functions[0], &summary)
            .expect("the exact Sleep executor retains the NativeCall pending category");
    }

    #[test]
    fn phase_4_admission_rejects_other_package_symbol_types() {
        let units = [unit(Vec::new(), Vec::new())];
        let error = admit_type(
            &units,
            &units[0],
            FUNCTION_KEY,
            &TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: "skiff.run/std".to_string(),
                    },
                    symbol_path: "std.http.HttpRequest".to_string(),
                    abi_expectation: None,
                },
            },
            false,
            "expression 0 type",
        )
        .expect_err("only the pinned Duration package symbol is admitted");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_accepts_union_throw_statement() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.expressions.push(expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            local(0),
        ));
        let statement = statement(
            0,
            MirStmtKind::Throw {
                value: ExprRefIr { expression: 0 },
                payload_type: union(vec![local(0), local(1)]),
                site: synthetic_site(),
            },
        );
        admit_statement(&units, &units[0], FUNCTION_KEY, &function, &statement)
            .expect("a union throw statement on the Phase 2 face is admitted");
    }

    #[test]
    fn phase_3_admission_accepts_catch_expression_and_its_region() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.slots.push(slot(0, local(0)));
        function.expressions.push(expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            number(),
        ));
        function
            .expressions
            .push(expression(1, ExprIr::LoadSlot { slot: 0 }, local(0)));
        function.expressions.push(expression(
            2,
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 0 },
                catch_slot: 0,
                catch_type: local(0),
                body: ExprRefIr { expression: 1 },
            },
            TypeRefIr::Builtin {
                name: "CatchResult".to_string(),
                args: vec![number(), local(0)],
            },
        ));
        function.regions.push(MirRegion {
            id: 0,
            catch_expr: 2,
            catch_slot: 0,
            catch_type: local(0),
            cleanup_depth: 0,
        });

        admit_exception_regions(&units[0], FUNCTION_KEY, &function)
            .expect("a well-formed catch region is admitted");
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[2],
            &BTreeSet::new(),
        )
        .expect("a catch expression on the Phase 2 face is admitted");
    }

    #[test]
    fn phase_3_admission_accepts_rethrow_and_never_types() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.slots.push(slot(0, local(0)));

        admit_statement(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &statement(0, MirStmtKind::Rethrow { exception_slot: 0 }),
        )
        .expect("a rethrow statement is admitted");

        let rethrow = expression(
            0,
            ExprIr::Rethrow { exception_slot: 0 },
            TypeRefIr::builtin("never"),
        );
        function.expressions.push(rethrow.clone());
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[0],
            &BTreeSet::new(),
        )
        .expect("a rethrow expression typed never is admitted");
    }

    #[test]
    fn phase_3_admission_rejects_throw_payload_outside_the_phase_2_face() {
        let units = [unit(Vec::new(), Vec::new())];
        let function = function();
        let error = admit_statement(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &statement(
                0,
                MirStmtKind::Throw {
                    value: ExprRefIr { expression: 0 },
                    payload_type: TypeRefIr::builtin("string"),
                    site: synthetic_site(),
                },
            ),
        )
        .expect_err("string payloads stay rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_rejects_missing_catch_region_facts() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        function.slots.push(slot(0, local(0)));
        function.expressions.push(expression(
            0,
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 1 },
                catch_slot: 0,
                catch_type: local(0),
                body: ExprRefIr { expression: 1 },
            },
            TypeRefIr::Builtin {
                name: "CatchResult".to_string(),
                args: vec![number(), local(0)],
            },
        ));
        function
            .expressions
            .push(expression(1, ExprIr::LoadSlot { slot: 0 }, local(0)));
        let error = admit_exception_regions(&units[0], FUNCTION_KEY, &function)
            .expect_err("a Catch node without a region must fail closed");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                construct: "exception region facts",
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_keeps_host_effect_throw_fail_closed() {
        let mut function = function();
        function.expressions.push(expression(
            0,
            ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::Builtin {
                        op: "hostOp".to_string(),
                    },
                    concrete_receiver: None,
                    site: synthetic_site(),
                    args: Vec::new(),
                    inout_args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            },
            number(),
        ));
        function.blocks.push(MirBlock {
            id: 0,
            label: "entry".to_string(),
            statements: vec![statement(
                0,
                MirStmtKind::Throw {
                    value: ExprRefIr { expression: 0 },
                    payload_type: local(0),
                    site: synthetic_site(),
                },
            )],
            successors: Vec::new(),
        });
        function
            .statements
            .push(skiff_compiler_lowering::mir::MirStatementEntry {
                statement_index: 0,
                span: None,
            });
        let units = [unit(vec![function], two_nominal_types())];
        let error = admit_phase_1_bytecode_mir(&units).expect_err("host targets stay rejected");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::HostTarget,
                ..
            }
        ));
    }

    #[test]
    fn phase_3_admission_accepts_catch_result_tag_discriminator_reads() {
        let catch_type = local(1);
        let catch_result = TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![TypeRefIr::builtin("never"), catch_type.clone()],
        };
        let mut function = function();
        function.slots.push(slot(0, catch_type.clone()));
        function.slots.push(slot(1, catch_result.clone()));
        function.expressions.push(expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(1_u64),
                },
            },
            number(),
        ));
        function.expressions.push(expression(
            1,
            ExprIr::Construct {
                type_ref: catch_type.clone(),
                fields: BTreeMap::from([("marker".to_string(), ExprRefIr { expression: 0 })]),
            },
            catch_type.clone(),
        ));
        function.expressions.push(expression(
            2,
            ExprIr::Throw {
                value: ExprRefIr { expression: 1 },
                payload_type: catch_type.clone(),
                site: synthetic_site(),
            },
            TypeRefIr::builtin("never"),
        ));
        function.expressions.push(expression(
            3,
            ExprIr::LoadSlot { slot: 0 },
            catch_type.clone(),
        ));
        function.expressions.push(expression(
            4,
            ExprIr::Catch {
                try_expression: ExprRefIr { expression: 2 },
                catch_slot: 0,
                catch_type: catch_type.clone(),
                body: ExprRefIr { expression: 3 },
            },
            catch_result.clone(),
        ));
        function.expressions.push(expression(
            5,
            ExprIr::LoadSlot { slot: 1 },
            catch_result.clone(),
        ));
        function.expressions.push(expression(
            6,
            ExprIr::Field {
                object: ExprRefIr { expression: 5 },
                field: "tag".to_string(),
            },
            TypeRefIr::Union {
                items: vec![
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "err".to_string(),
                        },
                    },
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "ok".to_string(),
                        },
                    },
                ],
            },
        ));
        function.expressions.push(expression(
            7,
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
            TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
        ));
        function.expressions.push(expression(
            8,
            ExprIr::Binary {
                op: skiff_artifact_model::BinaryOpIr::Equal,
                left: ExprRefIr { expression: 6 },
                right: ExprRefIr { expression: 7 },
            },
            TypeRefIr::builtin("bool"),
        ));
        function.regions.push(MirRegion {
            id: 0,
            catch_expr: 4,
            catch_slot: 0,
            catch_type: catch_type.clone(),
            cleanup_depth: 0,
        });

        let units = [unit(Vec::new(), two_nominal_types())];
        let positions = collect_discriminator_literal_positions(&function)
            .expect("discriminator positions collect");
        assert_eq!(
            positions.iter().copied().collect::<Vec<_>>(),
            vec![7],
            "only the tag-equality string literal is a discriminator constant"
        );

        let mut unknown_function = function.clone();
        unknown_function.expressions[6].ty = TypeRefIr::builtin("unknown");
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &unknown_function,
            &unknown_function.expressions[6],
            &positions,
        )
        .expect_err("a materialized tag field cannot retain an unknown type");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape,
                ..
            }
        ));

        for index in 4..=8 {
            admit_expression(
                &units,
                &units[0],
                FUNCTION_KEY,
                &function,
                &function.expressions[index],
                &positions,
            )
            .unwrap_or_else(|error| {
                panic!("discriminator expression {index} should be admitted: {error:?}")
            });
        }

        // The narrowed err-branch record load stays a stable LoadSlot fact.
        let narrowed_tag = TypeRefIr::Literal {
            value: LiteralIr::String {
                value: "err".to_string(),
            },
        };
        let narrowed = expression(
            9,
            ExprIr::LoadSlot { slot: 1 },
            TypeRefIr::Record {
                fields: BTreeMap::from([
                    (
                        "exception".to_string(),
                        TypeRefIr::Builtin {
                            name: "Exception".to_string(),
                            args: vec![catch_type],
                        },
                    ),
                    ("tag".to_string(), narrowed_tag),
                ]),
            },
        );
        function.expressions.push(narrowed);
        admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[9],
            &positions,
        )
        .expect("the narrowed CatchResult load is admitted");
    }

    #[test]
    fn phase_3_admission_keeps_non_discriminator_string_literals_fail_closed() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let mut function = function();
        let literal = expression(
            0,
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
            TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "ok".to_string(),
                },
            },
        );
        function.expressions.push(literal.clone());
        let error = admit_expression(
            &units,
            &units[0],
            FUNCTION_KEY,
            &function,
            &function.expressions[0],
            &BTreeSet::new(),
        )
        .expect_err("a bare string literal stays rejected");
        assert!(
            matches!(
                error,
                BytecodeEmissionError::UnsupportedPhase1Capability {
                    capability: Phase1UnsupportedCapability::ValueShape,
                    ..
                }
            ),
            "unexpected rejection: {error:?}"
        );
    }

    #[test]
    fn phase_3_admission_accepts_nominal_record_and_nominal_branch_throws() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let function = function();
        for payload_type in [local(0), union(vec![local(0), local(1)])] {
            admit_statement(
                &units,
                &units[0],
                FUNCTION_KEY,
                &function,
                &statement(
                    0,
                    MirStmtKind::Throw {
                        value: ExprRefIr { expression: 0 },
                        payload_type,
                        site: synthetic_site(),
                    },
                ),
            )
            .unwrap_or_else(|error| {
                panic!("nominal record / nominal-branch throw must be admitted: {error:?}")
            });
        }
    }

    #[test]
    fn phase_3_admission_rejects_identityless_throw_leaves_fail_closed() {
        let units = [unit(Vec::new(), two_nominal_types())];
        let function = function();
        let cases = vec![
            ("scalar", TypeRefIr::builtin("number")),
            (
                "anonymous structural record",
                TypeRefIr::Record {
                    fields: BTreeMap::from([("x".to_string(), number())]),
                },
            ),
            (
                "literal-branch union",
                union(vec![
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "ok".to_string(),
                        },
                    },
                    TypeRefIr::Literal {
                        value: LiteralIr::Bool { value: true },
                    },
                ]),
            ),
            (
                "array leaf",
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![number()],
                },
            ),
        ];
        for (name, payload_type) in cases {
            let error = admit_statement(
                &units,
                &units[0],
                FUNCTION_KEY,
                &function,
                &statement(
                    0,
                    MirStmtKind::Throw {
                        value: ExprRefIr { expression: 0 },
                        payload_type,
                        site: synthetic_site(),
                    },
                ),
            )
            .expect_err("identity-less throws must fail closed");
            assert!(
                matches!(
                    error,
                    BytecodeEmissionError::UnsupportedPhase1Capability {
                        capability: Phase1UnsupportedCapability::ValueShape,
                        ..
                    }
                ),
                "{name} throw rejected with the wrong shape: {error:?}"
            );
        }
    }
}
