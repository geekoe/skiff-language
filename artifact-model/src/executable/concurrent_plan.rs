use serde::{Deserialize, Serialize};

use super::{ExprRefIr, InstructionSourceSite};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConcurrentPlanIr {
    pub lanes: Vec<ConcurrentLaneIr>,
    pub site: InstructionSourceSite,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum ConcurrentLaneIr {
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

impl ConcurrentLaneIr {
    pub fn source_order(&self) -> u32 {
        match self {
            Self::Statement { source_order, .. }
            | Self::Serial { source_order, .. }
            | Self::Tail { source_order, .. } => *source_order,
        }
    }

    pub fn dependencies(&self) -> &[u32] {
        match self {
            Self::Statement { dependencies, .. }
            | Self::Serial { dependencies, .. }
            | Self::Tail { dependencies, .. } => dependencies,
        }
    }

    pub fn site(&self) -> &InstructionSourceSite {
        match self {
            Self::Statement { site, .. } | Self::Serial { site, .. } | Self::Tail { site, .. } => {
                site
            }
        }
    }
}
