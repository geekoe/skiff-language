use crate::{shared::error::SourceSpan, ExpressionOwnerKey, SourceCompileError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionSourceSite {
    pub module_path: String,
    pub owner: ExpressionOwnerKey,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeoutSourcePlan {
    pub duration_milliseconds: u64,
    pub produces_value: bool,
    pub source_site: ExecutionSourceSite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConcurrentLaneKind {
    Statement,
    Serial,
    Tail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConcurrentLanePlan {
    pub source_order: u32,
    pub kind: ConcurrentLaneKind,
    pub dependencies: Vec<u32>,
    pub source_site: ExecutionSourceSite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConcurrentSourcePlan {
    pub produces_value: bool,
    pub lanes: Vec<ConcurrentLanePlan>,
    pub source_site: ExecutionSourceSite,
}

#[derive(Clone, Debug, Default)]
pub struct SourceExecutionSemantics {
    pub(super) timeout_plans: Vec<TimeoutSourcePlan>,
    pub(super) concurrent_plans: Vec<ConcurrentSourcePlan>,
}

impl SourceExecutionSemantics {
    pub fn timeout_plans(&self) -> &[TimeoutSourcePlan] {
        &self.timeout_plans
    }

    pub fn concurrent_plans(&self) -> &[ConcurrentSourcePlan] {
        &self.concurrent_plans
    }

    pub fn validate_complete(&self) -> Result<(), SourceCompileError> {
        for plan in &self.timeout_plans {
            if plan.duration_milliseconds == 0 || !valid_source_site(&plan.source_site) {
                return invalid_execution_plan(&plan.source_site, "timeout plan");
            }
        }
        for plan in &self.concurrent_plans {
            if !valid_source_site(&plan.source_site) {
                return invalid_execution_plan(&plan.source_site, "concurrent source site");
            }
            let tail_indexes = plan
                .lanes
                .iter()
                .enumerate()
                .filter_map(|(index, lane)| {
                    (lane.kind == ConcurrentLaneKind::Tail).then_some(index)
                })
                .collect::<Vec<_>>();
            let valid_tail_shape = if plan.produces_value {
                tail_indexes == [plan.lanes.len().saturating_sub(1)]
            } else {
                tail_indexes.is_empty()
            };
            if !valid_tail_shape {
                return invalid_execution_plan(&plan.source_site, "concurrent tail shape");
            }
            for (index, lane) in plan.lanes.iter().enumerate() {
                if lane.source_order as usize != index
                    || !valid_source_site(&lane.source_site)
                    || lane.source_site.module_path != plan.source_site.module_path
                    || lane.source_site.owner != plan.source_site.owner
                    || lane.dependencies.windows(2).any(|pair| pair[0] >= pair[1])
                    || lane
                        .dependencies
                        .iter()
                        .any(|dependency| *dependency >= lane.source_order)
                {
                    return invalid_execution_plan(&plan.source_site, "concurrent lane");
                }
                if lane.kind == ConcurrentLaneKind::Tail
                    && lane.dependencies != (0..lane.source_order).collect::<Vec<_>>()
                {
                    return invalid_execution_plan(
                        &plan.source_site,
                        "concurrent tail dependencies",
                    );
                }
            }
        }
        Ok(())
    }
}

fn valid_source_site(site: &ExecutionSourceSite) -> bool {
    !site.module_path.trim().is_empty()
        && site.span.start.offset <= site.span.end.offset
        && (site.span.start.offset < site.span.end.offset
            || site.span.start.line == site.span.end.line)
}

fn invalid_execution_plan<T>(
    site: &ExecutionSourceSite,
    subject: &str,
) -> Result<T, SourceCompileError> {
    Err(SourceCompileError::ContractValidation {
        message: format!(
            "source execution semantics contains an invalid {subject} in {} {:?}",
            site.module_path, site.owner
        ),
    })
}
