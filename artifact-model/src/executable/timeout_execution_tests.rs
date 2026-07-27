use serde_json::json;

use super::{ConcurrentLaneIr, ConcurrentPlanIr, ExprIr, ExprRefIr, InstructionSourceSite, StmtIr};
use crate::{SourcePosition, SourceSpanRef};

fn source_site(start: u32, end: u32) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 0,
            start: SourcePosition {
                line: 1,
                column: start,
                offset: Some(start),
            },
            end: SourcePosition {
                line: 1,
                column: end,
                offset: Some(end),
            },
        },
    }
}

#[test]
fn timeout_and_concurrent_file_ir_round_trip_with_exact_canonical_shape() {
    let statement = StmtIr::Timeout {
        duration_ms: 20,
        body: "timeout_body$0".to_string(),
        site: source_site(1, 14),
    };
    let expression = ExprIr::ConcurrentValue {
        plan: ConcurrentPlanIr {
            lanes: vec![
                ConcurrentLaneIr::Statement {
                    source_order: 0,
                    dependencies: vec![],
                    body: "concurrent_lane$0".to_string(),
                    site: source_site(20, 31),
                },
                ConcurrentLaneIr::Serial {
                    source_order: 1,
                    dependencies: vec![0],
                    body: "concurrent_lane$1".to_string(),
                    site: source_site(32, 48),
                },
                ConcurrentLaneIr::Tail {
                    source_order: 2,
                    dependencies: vec![0, 1],
                    tail: ExprRefIr { expression: 7 },
                    site: source_site(49, 54),
                },
            ],
            site: source_site(15, 55),
        },
    };

    let statement_bytes = serde_json::to_vec(&statement).expect("statement should serialize");
    let expression_bytes = serde_json::to_vec(&expression).expect("expression should serialize");

    assert_eq!(
        serde_json::from_slice::<StmtIr>(&statement_bytes).unwrap(),
        statement
    );
    assert_eq!(
        serde_json::from_slice::<ExprIr>(&expression_bytes).unwrap(),
        expression
    );
    assert_eq!(
        String::from_utf8(statement_bytes).unwrap(),
        r#"{"kind":"timeout","durationMs":20,"body":"timeout_body$0","site":{"kind":"source","span":{"sourceId":0,"start":{"line":1,"column":1,"offset":1},"end":{"line":1,"column":14,"offset":14}}}}"#
    );
    assert_eq!(
        serde_json::to_value(&expression).unwrap()["plan"]["lanes"][2],
        json!({
            "kind": "tail",
            "sourceOrder": 2,
            "dependencies": [0, 1],
            "tail": {"expression": 7},
            "site": {
                "kind": "source",
                "span": {
                    "sourceId": 0,
                    "start": {"line": 1, "column": 49, "offset": 49},
                    "end": {"line": 1, "column": 54, "offset": 54}
                }
            }
        })
    );
}

#[test]
fn timeout_and_concurrent_file_ir_reject_unknown_or_legacy_shapes() {
    let unknown_lane = json!({
        "kind": "concurrentValue",
        "plan": {
            "lanes": [{
                "kind": "parallel",
                "sourceOrder": 0,
                "dependencies": [],
                "body": "lane$0",
                "site": serde_json::to_value(source_site(1, 2)).unwrap()
            }],
            "site": serde_json::to_value(source_site(1, 2)).unwrap()
        }
    });
    let legacy_duration = json!({
        "kind": "timeout",
        "duration": "20ms",
        "value": {"expression": 0},
        "site": serde_json::to_value(source_site(1, 2)).unwrap()
    });
    let extra_tail_body = json!({
        "kind": "concurrentValue",
        "plan": {
            "lanes": [{
                "kind": "tail",
                "sourceOrder": 0,
                "dependencies": [],
                "tail": {"expression": 0},
                "body": "forged",
                "site": serde_json::to_value(source_site(1, 2)).unwrap()
            }],
            "site": serde_json::to_value(source_site(1, 2)).unwrap()
        }
    });

    assert!(serde_json::from_value::<ExprIr>(unknown_lane).is_err());
    assert!(serde_json::from_value::<ExprIr>(legacy_duration).is_err());
    assert!(serde_json::from_value::<ExprIr>(extra_tail_body).is_err());
}
