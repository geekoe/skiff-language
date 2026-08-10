use serde::Serialize;
use sha2::Digest;

use super::*;

mod attribution;
mod execution;

use attribution::{
    AttributionChargeProjection, FrameEntryStatementProjection, RegionProjection, SourceProjection,
    StatementProjection,
};
use execution::{ControlProjection, PendingProjection};

/// Version of the canonical opcode-contract JSON projection. This is
/// deliberately independent from both the artifact schema and ISA versions:
/// changing projection shape increments this number, while changing any
/// projected contract fact changes only the fingerprint.
pub const OPCODE_CONTRACT_FORMAT: u8 = 2;

/// Canonical JSON bytes whose SHA-256 digest is persisted in the existing
/// `opcodeTableFingerprint` artifact header field.
pub fn opcode_contract_canonical_json() -> Vec<u8> {
    opcode_contracts_canonical_json(OPCODE_CONTRACTS)
}

/// Fingerprint of the default attribution charges, frame-entry rule and every
/// wire, typed and execution-policy fact in the unique 63-row opcode table.
pub fn opcode_table_fingerprint() -> String {
    opcode_contracts_fingerprint(OPCODE_CONTRACTS)
}

pub(crate) fn opcode_contracts_fingerprint(contracts: &[OpcodeContract]) -> String {
    opcode_contracts_fingerprint_with_frame(FRAME_ENTRY_STATEMENT_CONTRACT, contracts)
}

pub(crate) fn opcode_contracts_fingerprint_with_frame(
    frame_entry: FrameEntryStatementContract,
    contracts: &[OpcodeContract],
) -> String {
    opcode_contracts_fingerprint_with_statement_authority(
        ATTRIBUTION_CHARGE_CONTRACT,
        frame_entry,
        contracts,
    )
}

pub(crate) fn opcode_contracts_fingerprint_with_statement_authority(
    attribution_charges: AttributionChargeContract,
    frame_entry: FrameEntryStatementContract,
    contracts: &[OpcodeContract],
) -> String {
    hex::encode(sha2::Sha256::digest(
        opcode_contracts_canonical_json_with_statement_authority(
            attribution_charges,
            frame_entry,
            contracts,
        ),
    ))
}

pub(crate) fn opcode_contracts_canonical_json(contracts: &[OpcodeContract]) -> Vec<u8> {
    opcode_contracts_canonical_json_with_frame(FRAME_ENTRY_STATEMENT_CONTRACT, contracts)
}

pub(crate) fn opcode_contracts_canonical_json_with_frame(
    frame_entry: FrameEntryStatementContract,
    contracts: &[OpcodeContract],
) -> Vec<u8> {
    opcode_contracts_canonical_json_with_statement_authority(
        ATTRIBUTION_CHARGE_CONTRACT,
        frame_entry,
        contracts,
    )
}

fn opcode_contracts_canonical_json_with_statement_authority(
    attribution_charges: AttributionChargeContract,
    frame_entry: FrameEntryStatementContract,
    contracts: &[OpcodeContract],
) -> Vec<u8> {
    let projection = ContractSetProjection {
        contract_format: OPCODE_CONTRACT_FORMAT,
        attribution_charges: AttributionChargeProjection::from(attribution_charges),
        frame_entry_statement: FrameEntryStatementProjection::from(frame_entry),
        opcodes: contracts.iter().map(OpcodeProjection::from).collect(),
    };
    skiff_canonical_json::canonical_json_bytes(&projection)
        .expect("opcode contract projection always serializes to canonical JSON")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractSetProjection {
    contract_format: u8,
    attribution_charges: AttributionChargeProjection,
    frame_entry_statement: FrameEntryStatementProjection,
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
    statement: StatementProjection,
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
            statement: StatementProjection::from(contract.statement),
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
