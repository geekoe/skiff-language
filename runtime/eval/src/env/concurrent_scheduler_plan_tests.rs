use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_linked_program::{
    BlockIr, ExecutableKind, ExprRefIr, LinkedConcurrentLaneIr, LinkedConcurrentPlanIr,
    LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedStmtIr, LiteralIr, SlotIr,
    SlotLayoutIr, StmtRefIr,
};

use super::{project_concurrent_plan, ConcurrentPlanKind, LaneEvaluation};

#[test]
fn concurrent_scheduler_projection_exports_only_a_direct_statement_let() {
    let executable = executable(
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Let {
                slot: 1,
                value: ExprRefIr { expression: 0 },
            },
        ],
        vec![
            block("direct-let", &[0]),
            block("ordinary", &[1]),
            block("serial", &[2]),
        ],
        2,
    );
    let linked = LinkedConcurrentPlanIr {
        lanes: vec![
            statement(0, &[], "direct-let"),
            statement(1, &[0], "ordinary"),
            serial(2, &[1], "serial"),
        ],
        site: site(),
    };

    let plan =
        project_concurrent_plan(&linked, &executable, ConcurrentPlanKind::Statement).unwrap();

    assert_eq!(plan.lanes()[0].export_slot(), Some(0));
    assert_eq!(plan.lanes()[1].export_slot(), None);
    assert_eq!(plan.lanes()[2].export_slot(), None);
    assert!(matches!(
        plan.lanes()[0].evaluation(),
        LaneEvaluation::Statement { body } if body == "direct-let"
    ));
    assert!(matches!(
        plan.lanes()[2].evaluation(),
        LaneEvaluation::Serial { body } if body == "serial"
    ));
}

#[test]
fn concurrent_scheduler_projection_accepts_one_final_closed_tail() {
    let executable = executable(
        vec![LinkedStmtIr::Expr {
            value: ExprRefIr { expression: 0 },
        }],
        vec![block("lane", &[0])],
        0,
    );
    let linked = LinkedConcurrentPlanIr {
        lanes: vec![
            statement(0, &[], "lane"),
            tail(1, &[0], ExprRefIr { expression: 0 }),
        ],
        site: site(),
    };

    let plan = project_concurrent_plan(&linked, &executable, ConcurrentPlanKind::Value).unwrap();

    assert!(matches!(
        plan.lanes()[1].evaluation(),
        LaneEvaluation::Tail {
            expression: ExprRefIr { expression: 0 }
        }
    ));
}

#[test]
fn concurrent_scheduler_projection_rejects_malformed_body_and_slot_shapes() {
    let base = executable(
        vec![
            LinkedStmtIr::Let {
                slot: 0,
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            },
        ],
        vec![
            block("empty", &[]),
            block("two", &[0, 1]),
            block("let", &[0]),
        ],
        1,
    );
    let malformed = [
        statement_plan(statement(0, &[], "missing")),
        statement_plan(statement(0, &[], "empty")),
        statement_plan(statement(0, &[], "two")),
    ];
    for plan in malformed {
        let error = project_concurrent_plan(&plan, &base, ConcurrentPlanKind::Statement)
            .expect_err("malformed body must fail closed");
        assert!(error.to_string().contains("concurrent"));
    }

    let mut bad_slot = base.clone();
    bad_slot.body.statements[0] = LinkedStmtIr::Let {
        slot: 1,
        value: ExprRefIr { expression: 0 },
    };
    let error = project_concurrent_plan(
        &statement_plan(statement(0, &[], "let")),
        &bad_slot,
        ConcurrentPlanKind::Statement,
    )
    .expect_err("out-of-range export slot must fail closed");
    assert!(error.to_string().contains("out of bounds"));
}

#[test]
fn concurrent_scheduler_projection_rejects_dependency_and_tail_shape_corruption() {
    let executable = executable(
        vec![LinkedStmtIr::Expr {
            value: ExprRefIr { expression: 0 },
        }],
        vec![block("lane", &[0])],
        0,
    );
    let malformed = [
        (
            ConcurrentPlanKind::Statement,
            LinkedConcurrentPlanIr {
                lanes: vec![statement(1, &[], "lane")],
                site: site(),
            },
        ),
        (
            ConcurrentPlanKind::Statement,
            LinkedConcurrentPlanIr {
                lanes: vec![statement(0, &[], "lane"), statement(1, &[0, 0], "lane")],
                site: site(),
            },
        ),
        (
            ConcurrentPlanKind::Statement,
            LinkedConcurrentPlanIr {
                lanes: vec![tail(0, &[], ExprRefIr { expression: 0 })],
                site: site(),
            },
        ),
        (
            ConcurrentPlanKind::Value,
            LinkedConcurrentPlanIr {
                lanes: vec![statement(0, &[], "lane")],
                site: site(),
            },
        ),
        (
            ConcurrentPlanKind::Value,
            LinkedConcurrentPlanIr {
                lanes: vec![
                    statement(0, &[], "lane"),
                    tail(1, &[], ExprRefIr { expression: 0 }),
                ],
                site: site(),
            },
        ),
        (
            ConcurrentPlanKind::Value,
            LinkedConcurrentPlanIr {
                lanes: vec![
                    tail(0, &[], ExprRefIr { expression: 0 }),
                    statement(1, &[0], "lane"),
                ],
                site: site(),
            },
        ),
    ];

    for (kind, plan) in malformed {
        project_concurrent_plan(&plan, &executable, kind)
            .expect_err("corrupt dependency or tail shape must fail closed");
    }
}

fn executable(
    statements: Vec<LinkedStmtIr>,
    blocks: Vec<BlockIr>,
    frame_size: usize,
) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "concurrent-projection-test".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: (0..frame_size)
                .map(|index| SlotIr {
                    index,
                    name: format!("slot-{index}"),
                    kind: "local".to_string(),
                })
                .collect(),
            frame_size,
        },
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks,
            statements,
            expressions: vec![LinkedExprIr::Literal {
                value: LiteralIr::Null,
            }],
        },
    }
}

fn block(label: &str, statements: &[u32]) -> BlockIr {
    BlockIr {
        label: label.to_string(),
        statements: statements
            .iter()
            .map(|statement| StmtRefIr {
                statement: *statement,
            })
            .collect(),
    }
}

fn statement_plan(lane: LinkedConcurrentLaneIr) -> LinkedConcurrentPlanIr {
    LinkedConcurrentPlanIr {
        lanes: vec![lane],
        site: site(),
    }
}

fn statement(source_order: u32, dependencies: &[u32], body: &str) -> LinkedConcurrentLaneIr {
    LinkedConcurrentLaneIr::Statement {
        source_order,
        dependencies: dependencies.to_vec(),
        body: body.to_string(),
        site: site(),
    }
}

fn serial(source_order: u32, dependencies: &[u32], body: &str) -> LinkedConcurrentLaneIr {
    LinkedConcurrentLaneIr::Serial {
        source_order,
        dependencies: dependencies.to_vec(),
        body: body.to_string(),
        site: site(),
    }
}

fn tail(source_order: u32, dependencies: &[u32], expression: ExprRefIr) -> LinkedConcurrentLaneIr {
    LinkedConcurrentLaneIr::Tail {
        source_order,
        dependencies: dependencies.to_vec(),
        tail: expression,
        site: site(),
    }
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}
