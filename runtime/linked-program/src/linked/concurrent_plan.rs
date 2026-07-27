use serde::{Deserialize, Serialize};

use super::{ExprRefIr, InstructionSourceSite};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LinkedConcurrentPlanIr {
    pub lanes: Vec<LinkedConcurrentLaneIr>,
    pub site: InstructionSourceSite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum LinkedConcurrentLaneIr {
    Statement {
        source_order: u32,
        dependencies: Vec<u32>,
        body: String,
        site: InstructionSourceSite,
    },
    Serial {
        source_order: u32,
        dependencies: Vec<u32>,
        body: String,
        site: InstructionSourceSite,
    },
    Tail {
        source_order: u32,
        dependencies: Vec<u32>,
        tail: ExprRefIr,
        site: InstructionSourceSite,
    },
}
