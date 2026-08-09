use serde::Serialize;
use sha2::Digest;

use super::*;

/// Version of the canonical opcode-contract JSON projection. This is
/// deliberately independent from both the artifact schema and ISA versions:
/// changing projection shape increments this number, while changing any
/// projected contract fact changes only the fingerprint.
pub const OPCODE_CONTRACT_FORMAT: u8 = 1;

/// Canonical JSON bytes whose SHA-256 digest is persisted in the existing
/// `opcodeTableFingerprint` artifact header field.
pub fn opcode_contract_canonical_json() -> Vec<u8> {
    opcode_contracts_canonical_json(OPCODE_CONTRACTS)
}

/// Fingerprint of every wire, typed and execution-policy fact in the unique
/// 63-row opcode contract table.
pub fn opcode_table_fingerprint() -> String {
    opcode_contracts_fingerprint(OPCODE_CONTRACTS)
}

pub(crate) fn opcode_contracts_fingerprint(contracts: &[OpcodeContract]) -> String {
    hex::encode(sha2::Sha256::digest(opcode_contracts_canonical_json(
        contracts,
    )))
}

pub(crate) fn opcode_contracts_canonical_json(contracts: &[OpcodeContract]) -> Vec<u8> {
    let projection = ContractSetProjection {
        contract_format: OPCODE_CONTRACT_FORMAT,
        opcodes: contracts.iter().map(OpcodeProjection::from).collect(),
    };
    skiff_canonical_json::canonical_json_bytes(&projection)
        .expect("opcode contract projection always serializes to canonical JSON")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractSetProjection {
    contract_format: u8,
    opcodes: Vec<OpcodeProjection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpcodeProjection {
    kind: &'static str,
    opcode: u8,
    mnemonic: &'static str,
    operands: Vec<OperandProjection>,
    typed: TypedTransitionProjection,
    control: ControlProjection,
    pending: PendingProjection,
    checkpoint: CheckpointProjection,
    exception: ExceptionProjection,
    source: SourceProjection,
    region: RegionProjection,
    capabilities: Vec<&'static str>,
}

impl From<&OpcodeContract> for OpcodeProjection {
    fn from(contract: &OpcodeContract) -> Self {
        Self {
            kind: contract.kind.name(),
            opcode: contract.opcode,
            mnemonic: contract.mnemonic,
            operands: contract
                .operands
                .iter()
                .map(OperandProjection::from)
                .collect(),
            typed: TypedTransitionProjection::from(contract.typed),
            control: ControlProjection::from(contract.control),
            pending: PendingProjection::from(contract.pending),
            checkpoint: CheckpointProjection::from(contract.checkpoint),
            exception: ExceptionProjection::from(contract.exception),
            source: SourceProjection::from(contract.source),
            region: RegionProjection::from(contract.region),
            capabilities: contract
                .capabilities
                .iter()
                .map(|capability| capability.name())
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperandProjection {
    kind: &'static str,
    role: &'static str,
    linked_kind: &'static str,
    allowed_relocations: Vec<&'static str>,
}

impl From<&OperandSpec> for OperandProjection {
    fn from(operand: &OperandSpec) -> Self {
        Self {
            kind: operand.kind.name(),
            role: operand.role.name(),
            linked_kind: operand.linked_kind.name(),
            allowed_relocations: operand
                .allowed_relocations
                .iter()
                .map(|relocation| relocation.name())
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TypedTransitionProjection {
    stack_in: Vec<StackGroupProjection>,
    stack_out: Vec<StackGroupProjection>,
    slots: SlotProjection,
}

impl From<TypedTransition> for TypedTransitionProjection {
    fn from(typed: TypedTransition) -> Self {
        Self {
            stack_in: typed
                .stack_in
                .iter()
                .map(StackGroupProjection::from)
                .collect(),
            stack_out: typed
                .stack_out
                .iter()
                .map(StackGroupProjection::from)
                .collect(),
            slots: SlotProjection::from(typed.slots),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StackGroupProjection {
    arity: ArityProjection,
    value: ValueProjection,
}

impl From<&TypedStackGroup> for StackGroupProjection {
    fn from(group: &TypedStackGroup) -> Self {
        Self {
            arity: ArityProjection::from(group.arity),
            value: ValueProjection::from(group.value),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArityProjection {
    kind: &'static str,
    fixed: Option<u16>,
    operand: Option<&'static str>,
}

impl From<Arity> for ArityProjection {
    fn from(arity: Arity) -> Self {
        match arity {
            Arity::Fixed(fixed) => Self {
                kind: "fixed",
                fixed: Some(fixed),
                operand: None,
            },
            Arity::Declared(operand) => Self {
                kind: "declared",
                fixed: None,
                operand: Some(operand.name()),
            },
            Arity::FunctionResultCount => Self {
                kind: "functionResultCount",
                fixed: None,
                operand: None,
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValueProjection {
    kind: &'static str,
    operand: Option<&'static str>,
    secondary_operand: Option<&'static str>,
    input_group: Option<u8>,
}

impl From<ValueSource> for ValueProjection {
    fn from(value: ValueSource) -> Self {
        Self {
            kind: value.name(),
            operand: value.operand().map(OperandRole::name),
            secondary_operand: value.secondary_operand().map(OperandRole::name),
            input_group: value.input_group(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SlotProjection {
    kind: &'static str,
    effects: Vec<SlotEffectProjection>,
    target: Option<&'static str>,
    layout: Option<&'static str>,
}

impl From<SlotContract> for SlotProjection {
    fn from(slots: SlotContract) -> Self {
        match slots {
            SlotContract::None => Self {
                kind: "none",
                effects: Vec::new(),
                target: None,
                layout: None,
            },
            SlotContract::Effects(effects) => Self {
                kind: "effects",
                effects: effects.iter().map(SlotEffectProjection::from).collect(),
                target: None,
                layout: None,
            },
            SlotContract::InOutCallLoans { target, layout } => Self {
                kind: "inOutCallLoans",
                effects: Vec::new(),
                target: Some(target.name()),
                layout: Some(layout.name()),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SlotEffectProjection {
    operand: &'static str,
    action: &'static str,
    value: ValueProjection,
}

impl From<&SlotEffectContract> for SlotEffectProjection {
    fn from(effect: &SlotEffectContract) -> Self {
        Self {
            operand: effect.operand.name(),
            action: effect.action.name(),
            value: ValueProjection::from(effect.value),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ControlProjection {
    kind: &'static str,
    target: Option<&'static str>,
    branch_when: Option<&'static str>,
}

impl From<ControlContract> for ControlProjection {
    fn from(control: ControlContract) -> Self {
        match control {
            ControlContract::Fallthrough => Self::new("fallthrough", None, None),
            ControlContract::Jump { target } => Self::new("jump", Some(target), None),
            ControlContract::Branch { target, when } => {
                Self::new("branch", Some(target), Some(when.name()))
            }
            ControlContract::Switch { table } => Self::new("switch", Some(table), None),
            ControlContract::Return => Self::new("return", None, None),
            ControlContract::TailCall => Self::new("tailCall", None, None),
            ControlContract::Raise => Self::new("raise", None, None),
            ControlContract::Rethrow => Self::new("rethrow", None, None),
        }
    }
}

impl ControlProjection {
    fn new(
        kind: &'static str,
        target: Option<OperandRole>,
        branch_when: Option<&'static str>,
    ) -> Self {
        Self {
            kind,
            target: target.map(OperandRole::name),
            branch_when,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingProjection {
    kind: &'static str,
    target: Option<&'static str>,
    loan_layout: Option<&'static str>,
    resume: Option<&'static str>,
    mode: Option<&'static str>,
}

impl From<PendingContract> for PendingProjection {
    fn from(pending: PendingContract) -> Self {
        match pending {
            PendingContract::Never => Self::new("never", None, None, None, None),
            PendingContract::TransitiveTarget { target } => {
                Self::new("transitiveTarget", Some(target), None, None, None)
            }
            PendingContract::NoPendingTarget {
                target,
                loan_layout,
            } => Self::new(
                "noPendingTarget",
                Some(target),
                Some(loan_layout),
                None,
                None,
            ),
            PendingContract::ActualWithResume { resume, mode } => Self::new(
                "actualWithResume",
                None,
                None,
                Some(resume),
                Some(mode.name()),
            ),
        }
    }
}

impl PendingProjection {
    fn new(
        kind: &'static str,
        target: Option<OperandRole>,
        loan_layout: Option<OperandRole>,
        resume: Option<OperandRole>,
        mode: Option<&'static str>,
    ) -> Self {
        Self {
            kind,
            target: target.map(OperandRole::name),
            loan_layout: loan_layout.map(OperandRole::name),
            resume: resume.map(OperandRole::name),
            mode,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckpointProjection {
    kind: &'static str,
    budget_stop: Option<FailureDispositionProjection>,
    timeout_attribution: Option<&'static str>,
}

impl From<CheckpointContract> for CheckpointProjection {
    fn from(checkpoint: CheckpointContract) -> Self {
        match checkpoint {
            CheckpointContract::None => Self {
                kind: "none",
                budget_stop: None,
                timeout_attribution: None,
            },
            CheckpointContract::Budget {
                budget_stop,
                timeout_attribution,
            } => Self {
                kind: "budget",
                budget_stop: Some(FailureDispositionProjection::from(budget_stop)),
                timeout_attribution: Some(timeout_attribution.name()),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionProjection {
    behavior: ExceptionBehaviorProjection,
    failures: Vec<FailureProjection>,
}

impl From<ExceptionContract> for ExceptionProjection {
    fn from(exception: ExceptionContract) -> Self {
        Self {
            behavior: ExceptionBehaviorProjection::from(exception.behavior),
            failures: exception
                .failures
                .iter()
                .map(FailureProjection::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExceptionBehaviorProjection {
    kind: &'static str,
    operand: Option<&'static str>,
}

impl From<ExceptionBehavior> for ExceptionBehaviorProjection {
    fn from(behavior: ExceptionBehavior) -> Self {
        match behavior {
            ExceptionBehavior::None => Self::new("none", None),
            ExceptionBehavior::PropagateTarget { target } => {
                Self::new("propagateTarget", Some(target))
            }
            ExceptionBehavior::RaiseAtCurrentSite => Self::new("raiseAtCurrentSite", None),
            ExceptionBehavior::ThrowValue { type_ref } => Self::new("throwValue", Some(type_ref)),
            ExceptionBehavior::PreserveOriginal { source_slot } => {
                Self::new("preserveOriginal", Some(source_slot))
            }
        }
    }
}

impl ExceptionBehaviorProjection {
    fn new(kind: &'static str, operand: Option<OperandRole>) -> Self {
        Self {
            kind,
            operand: operand.map(OperandRole::name),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureProjection {
    kind: &'static str,
    trigger: &'static str,
    disposition: FailureDispositionProjection,
}

impl From<&FailureContract> for FailureProjection {
    fn from(failure: &FailureContract) -> Self {
        Self {
            kind: failure.kind.name(),
            trigger: failure.trigger.name(),
            disposition: FailureDispositionProjection::from(failure.disposition),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureDispositionProjection {
    kind: &'static str,
    identity: Option<&'static str>,
}

impl From<FailureDisposition> for FailureDispositionProjection {
    fn from(disposition: FailureDisposition) -> Self {
        match disposition {
            FailureDisposition::Catchable { identity } => Self {
                kind: "catchable",
                identity: Some(identity),
            },
            FailureDisposition::UncatchableTerminal => Self {
                kind: "uncatchableTerminal",
                identity: None,
            },
            FailureDisposition::InvariantTerminal => Self {
                kind: "invariantTerminal",
                identity: None,
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceProjection {
    kind: &'static str,
    use_kind: Option<&'static str>,
    origin: Option<&'static str>,
    operand: Option<&'static str>,
}

impl From<SourceContract> for SourceProjection {
    fn from(source: SourceContract) -> Self {
        match source {
            SourceContract::None => Self::new("none", None, None, None),
            SourceContract::Required { use_kind, origin } => {
                Self::new("required", Some(use_kind.name()), Some(origin.name()), None)
            }
            SourceContract::PreserveOriginal => Self::new("preserveOriginal", None, None, None),
            SourceContract::ActiveRegion { operand } => {
                Self::new("activeRegion", None, None, Some(operand))
            }
        }
    }
}

impl SourceProjection {
    fn new(
        kind: &'static str,
        use_kind: Option<&'static str>,
        origin: Option<&'static str>,
        operand: Option<OperandRole>,
    ) -> Self {
        Self {
            kind,
            use_kind,
            origin,
            operand: operand.map(OperandRole::name),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegionProjection {
    normal: RegionEffectProjection,
    raised: RegionEffectProjection,
}

impl From<RegionContract> for RegionProjection {
    fn from(region: RegionContract) -> Self {
        Self {
            normal: RegionEffectProjection::from(region.normal),
            raised: RegionEffectProjection::from(region.raised),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegionEffectProjection {
    kind: &'static str,
    operand: Option<&'static str>,
}

impl From<RegionEffect> for RegionEffectProjection {
    fn from(effect: RegionEffect) -> Self {
        match effect {
            RegionEffect::NotApplicable => Self::new("notApplicable", None),
            RegionEffect::Preserve => Self::new("preserve", None),
            RegionEffect::Enter { operand } => Self::new("enter", Some(operand)),
            RegionEffect::Leave { operand } => Self::new("leave", Some(operand)),
            RegionEffect::ExitFunction => Self::new("exitFunction", None),
            RegionEffect::TailReplace => Self::new("tailReplace", None),
            RegionEffect::Unwind => Self::new("unwind", None),
        }
    }
}

impl RegionEffectProjection {
    fn new(kind: &'static str, operand: Option<OperandRole>) -> Self {
        Self {
            kind,
            operand: operand.map(OperandRole::name),
        }
    }
}
