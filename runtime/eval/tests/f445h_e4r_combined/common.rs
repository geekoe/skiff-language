use super::imports::*;

pub(super) const SERVICE_ID: &str = "skiff.test/f445h-e4r-combined";
pub(super) const ACTOR_FILE_ID: &str = "file:f445h-e4r-combined-actor";
pub(super) const VERSION: &str = "1.0.0";

pub(super) fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

pub(super) fn integer() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "integer".to_string(),
        args: Vec::new(),
    }
}

pub(super) fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

pub(super) fn number(value: u64) -> LinkedExprIr {
    LinkedExprIr::Literal {
        value: LiteralIr::Number {
            value: serde_json::Number::from(value),
        },
    }
}

pub(super) fn call(target: LinkedCallTarget, args: &[u32]) -> CallIr {
    CallIr {
        target,
        concrete_receiver: None,
        site: site(),
        args: args
            .iter()
            .copied()
            .map(|expression| ExprRefIr { expression })
            .collect(),
        inout_args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

pub(super) fn native_sleep_call(argument: u32) -> LinkedExprIr {
    LinkedExprIr::Call {
        call: call(
            LinkedCallTarget::Native {
                target: NativeTarget {
                    namespace: "std.time".to_string(),
                    symbol: "sleep".to_string(),
                    binding_key: Some("std.time.sleep".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            &[argument],
        ),
    }
}
