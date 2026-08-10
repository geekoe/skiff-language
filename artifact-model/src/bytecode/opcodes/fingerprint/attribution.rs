use serde::Serialize;

use super::super::{
    AttributionChargeContract, FrameEntryStatementContract, OperandRole, RegionContract,
    RegionEffect, SourceContract, StatementContract,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AttributionChargeProjection {
    statement: &'static str,
    expression: &'static str,
    generated: &'static str,
}

impl From<AttributionChargeContract> for AttributionChargeProjection {
    fn from(contract: AttributionChargeContract) -> Self {
        Self {
            statement: contract.statement.name(),
            expression: contract.expression.name(),
            generated: contract.generated.name(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrameEntryStatementProjection {
    charge_kind: &'static str,
}

impl From<FrameEntryStatementContract> for FrameEntryStatementProjection {
    fn from(contract: FrameEntryStatementContract) -> Self {
        Self {
            charge_kind: contract.charge_kind.name(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatementProjection {
    kind: &'static str,
    charge_kind: Option<&'static str>,
    attribution: Option<&'static str>,
}

impl From<StatementContract> for StatementProjection {
    fn from(contract: StatementContract) -> Self {
        match contract {
            StatementContract::None => Self {
                kind: "none",
                charge_kind: None,
                attribution: None,
            },
            StatementContract::RequiredEvent {
                charge_kind,
                attribution,
            } => Self {
                kind: "requiredEvent",
                charge_kind: Some(charge_kind.name()),
                attribution: Some(attribution.name()),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SourceProjection {
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
pub(super) struct RegionProjection {
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
