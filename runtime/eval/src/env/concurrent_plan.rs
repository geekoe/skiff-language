use skiff_runtime_linked_program::{
    ExprRefIr, LinkedConcurrentLaneIr, LinkedConcurrentPlanIr, LinkedExecutable, LinkedStmtIr,
};

use crate::{
    error::{Result, RuntimeError},
    program_ir::{
        program_block, program_expression_ref, program_statement_ref, program_u32_to_usize,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConcurrentPlanKind {
    Statement,
    Value,
}

#[derive(Clone, Debug)]
pub(crate) struct ConcurrentPlan {
    kind: ConcurrentPlanKind,
    lanes: Vec<ProjectedLane>,
}

#[derive(Clone, Debug)]
pub(crate) struct ProjectedLane {
    source_order: usize,
    dependencies: Vec<usize>,
    evaluation: LaneEvaluation,
    export_slot: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LaneEvaluation {
    Statement { body: String },
    Serial { body: String },
    Tail { expression: ExprRefIr },
}

pub(crate) fn project_concurrent_plan(
    linked: &LinkedConcurrentPlanIr,
    executable: &LinkedExecutable,
    kind: ConcurrentPlanKind,
) -> Result<ConcurrentPlan> {
    let mut lanes = Vec::with_capacity(linked.lanes.len());
    let mut tail_count = 0_usize;

    for (index, lane) in linked.lanes.iter().enumerate() {
        let source_order = usize::try_from(lane_source_order(lane)).map_err(|_| {
            invalid_plan(format!(
                "lane source order {} cannot be represented by this runtime",
                lane_source_order(lane)
            ))
        })?;
        if source_order != index {
            return Err(invalid_plan(format!(
                "lane source order is not contiguous at index {index}: {source_order}"
            )));
        }

        let dependencies = project_dependencies(lane, source_order)?;
        let (evaluation, export_slot) = match lane {
            LinkedConcurrentLaneIr::Statement { body, .. } => {
                let block = program_block(executable, body)?;
                if block.label != *body || block.statements.len() != 1 {
                    return Err(invalid_plan(format!(
                        "statement lane {source_order} body `{body}` must contain exactly one direct statement"
                    )));
                }
                let statement = program_statement_ref(executable, &block.statements[0])?;
                let export_slot = match statement {
                    LinkedStmtIr::Let { slot, .. } => {
                        let slot = program_u32_to_usize(*slot, "concurrent let slot")?;
                        if slot >= executable.slots.frame_size {
                            return Err(invalid_plan(format!(
                                "statement lane {source_order} export slot {slot} is out of bounds for frame size {}",
                                executable.slots.frame_size
                            )));
                        }
                        Some(slot)
                    }
                    _ => None,
                };
                (
                    LaneEvaluation::Statement { body: body.clone() },
                    export_slot,
                )
            }
            LinkedConcurrentLaneIr::Serial { body, .. } => {
                let block = program_block(executable, body)?;
                if block.label != *body {
                    return Err(invalid_plan(format!(
                        "serial lane {source_order} body `{body}` resolved to a different block"
                    )));
                }
                (LaneEvaluation::Serial { body: body.clone() }, None)
            }
            LinkedConcurrentLaneIr::Tail { tail, .. } => {
                tail_count += 1;
                program_expression_ref(executable, *tail)?;
                if dependencies.iter().copied().ne(0..source_order) {
                    return Err(invalid_plan(format!(
                        "tail lane {source_order} dependencies do not close over all prior lanes"
                    )));
                }
                (LaneEvaluation::Tail { expression: *tail }, None)
            }
        };

        lanes.push(ProjectedLane {
            source_order,
            dependencies,
            evaluation,
            export_slot,
        });
    }

    let valid_tail_shape = match kind {
        ConcurrentPlanKind::Statement => tail_count == 0,
        ConcurrentPlanKind::Value => {
            tail_count == 1
                && lanes
                    .last()
                    .is_some_and(|lane| matches!(lane.evaluation, LaneEvaluation::Tail { .. }))
        }
    };
    if !valid_tail_shape {
        return Err(invalid_plan("concurrent plan has an invalid tail shape"));
    }

    Ok(ConcurrentPlan { kind, lanes })
}

fn project_dependencies(lane: &LinkedConcurrentLaneIr, source_order: usize) -> Result<Vec<usize>> {
    let mut projected = Vec::with_capacity(lane_dependencies(lane).len());
    let mut previous = None;
    for dependency in lane_dependencies(lane) {
        let dependency = usize::try_from(*dependency).map_err(|_| {
            invalid_plan(format!(
                "lane {source_order} dependency {dependency} cannot be represented by this runtime"
            ))
        })?;
        if previous.is_some_and(|previous| previous >= dependency) {
            return Err(invalid_plan(format!(
                "lane {source_order} dependencies must be strictly sorted and unique"
            )));
        }
        if dependency >= source_order {
            return Err(invalid_plan(format!(
                "lane {source_order} dependency {dependency} is not a prior lane"
            )));
        }
        previous = Some(dependency);
        projected.push(dependency);
    }
    Ok(projected)
}

fn lane_source_order(lane: &LinkedConcurrentLaneIr) -> u32 {
    match lane {
        LinkedConcurrentLaneIr::Statement { source_order, .. }
        | LinkedConcurrentLaneIr::Serial { source_order, .. }
        | LinkedConcurrentLaneIr::Tail { source_order, .. } => *source_order,
    }
}

fn lane_dependencies(lane: &LinkedConcurrentLaneIr) -> &[u32] {
    match lane {
        LinkedConcurrentLaneIr::Statement { dependencies, .. }
        | LinkedConcurrentLaneIr::Serial { dependencies, .. }
        | LinkedConcurrentLaneIr::Tail { dependencies, .. } => dependencies,
    }
}

fn invalid_plan(message: impl Into<String>) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "invalid concurrent execution plan: {}",
        message.into()
    ))
}

impl ConcurrentPlan {
    pub(crate) fn kind(&self) -> ConcurrentPlanKind {
        self.kind
    }

    pub(crate) fn lanes(&self) -> &[ProjectedLane] {
        &self.lanes
    }

    #[cfg(test)]
    pub(super) fn for_test(kind: ConcurrentPlanKind, lanes: Vec<ProjectedLane>) -> Self {
        Self { kind, lanes }
    }
}

impl ProjectedLane {
    pub(crate) fn source_order(&self) -> usize {
        self.source_order
    }

    pub(crate) fn dependencies(&self) -> &[usize] {
        &self.dependencies
    }

    pub(crate) fn evaluation(&self) -> &LaneEvaluation {
        &self.evaluation
    }

    pub(crate) fn export_slot(&self) -> Option<usize> {
        self.export_slot
    }

    #[cfg(test)]
    pub(super) fn for_test(
        source_order: usize,
        dependencies: Vec<usize>,
        evaluation: LaneEvaluation,
        export_slot: Option<usize>,
    ) -> Self {
        Self {
            source_order,
            dependencies,
            evaluation,
            export_slot,
        }
    }
}
