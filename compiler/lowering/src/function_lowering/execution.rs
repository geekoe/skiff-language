use skiff_artifact_model::{
    AssignTargetIr, BlockIr, ConcurrentLaneIr, ConcurrentPlanIr, ExprIr, InstructionSourceSite,
    SlotKind, StmtIr, SyntheticInstructionSiteReason, TypeRefIr,
};
use skiff_compiler_source::{
    ConcurrentLaneKind, ConcurrentSourcePlan, ExecutionSourceSite, TimeoutSourcePlan,
};
use skiff_syntax::{
    ast::{Block, Expr, Stmt, ValueBlock},
    error::{CompileError, Result},
};

use super::FunctionLowerer;
use crate::source_unit_lowering::source_span_ref;

impl FunctionLowerer<'_> {
    pub(super) fn lower_timeout_statement(&mut self, body: &Block) -> Result<StmtIr> {
        let plan = self.take_timeout_plan(false)?;
        Ok(StmtIr::Timeout {
            duration_ms: plan.duration_milliseconds,
            body: self.lower_scoped_block("timeout_body", body, |_| Ok(()))?,
            site: instruction_site(&plan.source_site),
        })
    }

    pub(super) fn lower_timeout_value(
        &mut self,
        value: &Expr,
        expected_target: Option<&skiff_artifact_model::TypeRefIr>,
    ) -> Result<ExprIr> {
        let plan = self.take_timeout_plan(true)?;
        Ok(ExprIr::Timeout {
            duration_ms: plan.duration_milliseconds,
            value: self.lower_expr_with_expected(value, expected_target)?,
            site: instruction_site(&plan.source_site),
        })
    }

    pub(super) fn lower_ternary_expr(
        &mut self,
        condition: &Expr,
        then_expr: &Expr,
        else_expr: &Expr,
        result_type: &TypeRefIr,
    ) -> Result<ExprIr> {
        let temp_name = format!("$ternary{}", self.slots.len());
        let temp_slot = self.declare_slot(&temp_name, SlotKind::Temp, true)?;
        let condition = self.lower_expr(condition)?;
        let then_value = self.lower_expr(then_expr)?;
        let else_value = self.lower_expr(else_expr)?;
        self.set_slot_type(temp_slot, Some(result_type.clone()));

        let then_label = self.next_block_label("ternary_then");
        let else_label = self.next_block_label("ternary_else");
        let then_stmt = self.push_stmt(
            StmtIr::Assign {
                target: AssignTargetIr::Slot { slot: temp_slot },
                value: then_value,
            },
            None,
        );
        self.record_generated_statement_event(
            then_stmt.statement,
            SyntheticInstructionSiteReason::CompilerDesugaring,
        )?;
        self.body.blocks.push(BlockIr {
            label: then_label.clone(),
            statements: vec![then_stmt],
        });
        let else_stmt = self.push_stmt(
            StmtIr::Assign {
                target: AssignTargetIr::Slot { slot: temp_slot },
                value: else_value,
            },
            None,
        );
        self.record_generated_statement_event(
            else_stmt.statement,
            SyntheticInstructionSiteReason::CompilerDesugaring,
        )?;
        self.body.blocks.push(BlockIr {
            label: else_label.clone(),
            statements: vec![else_stmt],
        });

        let body_label = self.next_block_label("ternary_body");
        let body_stmt = self.push_stmt(
            StmtIr::If {
                condition,
                then_block: then_label,
                else_block: Some(else_label),
            },
            None,
        );
        self.record_generated_statement_event(
            body_stmt.statement,
            SyntheticInstructionSiteReason::CompilerDesugaring,
        )?;
        self.body.blocks.push(BlockIr {
            label: body_label.clone(),
            statements: vec![body_stmt],
        });
        Ok(ExprIr::ValueBlock {
            block: body_label,
            result: self.push_expr(ExprIr::LoadSlot { slot: temp_slot }, result_type.clone()),
        })
    }

    pub(super) fn lower_user_value_block(
        &mut self,
        value: &ValueBlock,
        expected_target: Option<&skiff_artifact_model::TypeRefIr>,
    ) -> Result<ExprIr> {
        let label = self.next_block_label("value_body");
        self.push_scope();
        let mut block = BlockIr {
            label: label.clone(),
            statements: Vec::new(),
        };
        for statement in &value.body.statements {
            block.statements.push(self.lower_stmt(statement)?);
        }
        let result = self.lower_expr_with_expected(&value.tail, expected_target)?;
        self.pop_scope();
        self.body.blocks.push(block);
        Ok(ExprIr::ValueBlock {
            block: label,
            result,
        })
    }

    pub(super) fn lower_concurrent_statement(&mut self, body: &Block) -> Result<StmtIr> {
        Ok(StmtIr::Concurrent {
            plan: self.lower_concurrent_plan(body, None, None)?,
        })
    }

    pub(super) fn lower_concurrent_value(
        &mut self,
        value: &ValueBlock,
        expected_target: Option<&skiff_artifact_model::TypeRefIr>,
    ) -> Result<ExprIr> {
        Ok(ExprIr::ConcurrentValue {
            plan: self.lower_concurrent_plan(&value.body, Some(&value.tail), expected_target)?,
        })
    }

    fn lower_concurrent_plan(
        &mut self,
        body: &Block,
        tail: Option<&Expr>,
        expected_target: Option<&skiff_artifact_model::TypeRefIr>,
    ) -> Result<ConcurrentPlanIr> {
        let produces_value = tail.is_some();
        let plan = self.take_concurrent_plan(produces_value)?;
        let expected_lane_count = body.statements.len() + usize::from(produces_value);
        if plan.lanes.len() != expected_lane_count {
            return Err(CompileError::Semantic(format!(
                "compiler-checked concurrent plan has {} lanes but source lowering requires {expected_lane_count}",
                plan.lanes.len()
            )));
        }

        self.push_scope();
        let mut lanes = Vec::with_capacity(plan.lanes.len());
        for (statement, lane) in body.statements.iter().zip(&plan.lanes) {
            let source_order = lane.source_order;
            let label = self.next_block_label("concurrent_lane");
            let mut block = BlockIr {
                label: label.clone(),
                statements: Vec::new(),
            };
            let lowered = match lane.kind {
                ConcurrentLaneKind::Statement => {
                    if matches!(statement, Stmt::Serial { .. }) {
                        return Err(plan_mismatch(source_order, "statement", "serial source"));
                    }
                    block.statements.push(self.lower_stmt(statement)?);
                    ConcurrentLaneIr::Statement {
                        source_order,
                        dependencies: lane.dependencies.clone(),
                        body: label,
                        site: instruction_site(&lane.source_site),
                    }
                }
                ConcurrentLaneKind::Serial => {
                    let Stmt::Serial { body } = statement else {
                        return Err(plan_mismatch(source_order, "serial", "non-serial source"));
                    };
                    self.push_scope();
                    for statement in &body.statements {
                        block.statements.push(self.lower_stmt(statement)?);
                    }
                    self.pop_scope();
                    ConcurrentLaneIr::Serial {
                        source_order,
                        dependencies: lane.dependencies.clone(),
                        body: label,
                        site: instruction_site(&lane.source_site),
                    }
                }
                ConcurrentLaneKind::Tail => {
                    return Err(plan_mismatch(
                        source_order,
                        "statement or serial",
                        "tail plan",
                    ));
                }
            };
            self.body.blocks.push(block);
            lanes.push(lowered);
        }

        if let Some(tail) = tail {
            let lane = plan
                .lanes
                .last()
                .ok_or_else(|| plan_mismatch(0, "tail", "empty plan"))?;
            if lane.kind != ConcurrentLaneKind::Tail {
                return Err(plan_mismatch(lane.source_order, "tail", "non-tail plan"));
            }
            lanes.push(ConcurrentLaneIr::Tail {
                source_order: lane.source_order,
                dependencies: lane.dependencies.clone(),
                tail: self.lower_expr_with_expected(tail, expected_target)?,
                site: instruction_site(&lane.source_site),
            });
        }
        self.pop_scope();

        Ok(ConcurrentPlanIr {
            lanes,
            site: instruction_site(&plan.source_site),
        })
    }

    fn take_timeout_plan(&mut self, produces_value: bool) -> Result<TimeoutSourcePlan> {
        let plan = self
            .owner_timeout_plans()?
            .nth(self.timeout_plan_cursor)
            .cloned()
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "timeout lowering has no compiler-checked source plan at owner cursor {}",
                    self.timeout_plan_cursor
                ))
            })?;
        self.timeout_plan_cursor += 1;
        if plan.produces_value != produces_value {
            return Err(CompileError::Semantic(format!(
                "timeout source plan value shape mismatch at owner cursor {}",
                self.timeout_plan_cursor - 1
            )));
        }
        Ok(plan)
    }

    fn take_concurrent_plan(&mut self, produces_value: bool) -> Result<ConcurrentSourcePlan> {
        let plan = self
            .owner_concurrent_plans()?
            .nth(self.concurrent_plan_cursor)
            .cloned()
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "concurrent lowering has no compiler-checked source plan at owner cursor {}",
                    self.concurrent_plan_cursor
                ))
            })?;
        self.concurrent_plan_cursor += 1;
        if plan.produces_value != produces_value {
            return Err(CompileError::Semantic(format!(
                "concurrent source plan value shape mismatch at owner cursor {}",
                self.concurrent_plan_cursor - 1
            )));
        }
        Ok(plan)
    }

    fn owner_timeout_plans(&self) -> Result<impl Iterator<Item = &TimeoutSourcePlan> + '_> {
        let owner = self.expression_owner.as_ref().ok_or_else(|| {
            CompileError::Semantic("execution lowering requires an expression owner".to_string())
        })?;
        let semantics = self.execution_semantics.ok_or_else(|| {
            CompileError::Semantic(
                "execution lowering requires PackageSourceModel::execution_semantics()".to_string(),
            )
        })?;
        Ok(semantics.timeout_plans().iter().filter(move |plan| {
            plan.source_site.module_path == self.module_path && &plan.source_site.owner == owner
        }))
    }

    fn owner_concurrent_plans(&self) -> Result<impl Iterator<Item = &ConcurrentSourcePlan> + '_> {
        let owner = self.expression_owner.as_ref().ok_or_else(|| {
            CompileError::Semantic("execution lowering requires an expression owner".to_string())
        })?;
        let semantics = self.execution_semantics.ok_or_else(|| {
            CompileError::Semantic(
                "execution lowering requires PackageSourceModel::execution_semantics()".to_string(),
            )
        })?;
        Ok(semantics.concurrent_plans().iter().filter(move |plan| {
            plan.source_site.module_path == self.module_path && &plan.source_site.owner == owner
        }))
    }

    pub(crate) fn validate_execution_plans_consumed(&self) -> Result<()> {
        let Some(_) = self.execution_semantics else {
            if self.timeout_plan_cursor == 0 && self.concurrent_plan_cursor == 0 {
                return Ok(());
            }
            return Err(CompileError::Semantic(
                "execution lowering consumed plans without source semantics".to_string(),
            ));
        };
        let timeout_count = self.owner_timeout_plans()?.count();
        let concurrent_count = self.owner_concurrent_plans()?.count();
        if timeout_count != self.timeout_plan_cursor
            || concurrent_count != self.concurrent_plan_cursor
        {
            return Err(CompileError::Semantic(format!(
                "execution lowering did not consume the exact source plan set: timeout {}/{timeout_count}, concurrent {}/{concurrent_count}",
                self.timeout_plan_cursor, self.concurrent_plan_cursor
            )));
        }
        Ok(())
    }
}

fn instruction_site(site: &ExecutionSourceSite) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: source_span_ref(site.span),
    }
}

fn plan_mismatch(order: u32, expected: &str, actual: &str) -> CompileError {
    CompileError::Semantic(format!(
        "compiler-checked concurrent lane {order} expected {expected}, found {actual}"
    ))
}
