use serde_json::json;
use skiff_artifact_model::{InstructionSourceSite, SourcePosition, SourceSpanRef};
use skiff_runtime_linked_program::{
    ExprRefIr, LinkedConcurrentLaneIr, LinkedConcurrentPlanIr, LinkedExprIr,
};

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
fn linked_timeout_and_concurrent_shapes_are_strict_and_round_trip() {
    let expression = LinkedExprIr::ConcurrentValue {
        plan: LinkedConcurrentPlanIr {
            lanes: vec![
                LinkedConcurrentLaneIr::Statement {
                    source_order: 0,
                    dependencies: vec![],
                    body: "lane$0".to_string(),
                    site: source_site(10, 20),
                },
                LinkedConcurrentLaneIr::Tail {
                    source_order: 1,
                    dependencies: vec![0],
                    tail: ExprRefIr { expression: 2 },
                    site: source_site(21, 25),
                },
            ],
            site: source_site(5, 26),
        },
    };

    let encoded = serde_json::to_value(&expression).expect("linked expression should serialize");
    assert_eq!(
        serde_json::from_value::<LinkedExprIr>(encoded.clone()).unwrap(),
        expression
    );
    assert_eq!(encoded["plan"]["lanes"][1]["kind"], "tail");

    let mut unknown_field = encoded;
    unknown_field["plan"]["lanes"][0]["forged"] = json!(true);
    assert!(serde_json::from_value::<LinkedExprIr>(unknown_field).is_err());

    let unknown_kind = json!({
        "kind": "concurrentValue",
        "plan": {
            "lanes": [{
                "kind": "parallel",
                "sourceOrder": 0,
                "dependencies": [],
                "body": "lane$0",
                "site": serde_json::to_value(source_site(10, 20)).unwrap()
            }],
            "site": serde_json::to_value(source_site(5, 21)).unwrap()
        }
    });
    assert!(serde_json::from_value::<LinkedExprIr>(unknown_kind).is_err());
}
