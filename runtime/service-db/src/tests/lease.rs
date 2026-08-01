use super::{super::*, support::*};

#[test]
fn guarded_filter_fences_held_lease_key() {
    let binding = thread_binding();
    let hold = DbLeaseHold {
        target_key: "Thread".to_string(),
        type_name: "Thread".to_string(),
        key: db_key(json!("thread-1")),
        slot: "writer".to_string(),
        token: "token-1".to_string(),
    };

    let filter = guarded_filter(&binding, doc! { "title": "Hello" }, &[hold], 1000)
        .expect("guarded filter should build");

    assert_eq!(
        filter,
        doc! {
            "$and": [
                { "title": "Hello" },
                {
                    "$or": [
                        { "_id": { "$ne": "thread-1" } },
                        {
                            "_id": "thread-1",
                            "$or": [
                                { "__skiffLeases.writer.maxExpiresAtMs": Bson::Null },
                                { "__skiffLeases.writer.maxExpiresAtMs": { "$exists": false } },
                                { "__skiffLeases.writer.maxExpiresAtMs": { "$gt": 1000_i64 } }
                            ],
                            "__skiffLeases.writer.token": "token-1",
                            "__skiffLeases.writer.expiresAtMs": { "$gt": 1000_i64 }
                        }
                    ]
                }
            ]
        }
    );
}

#[test]
fn lease_live_key_filter_requires_ttl_and_max_to_be_live() {
    let filter = lease_live_key_filter(
        "writer",
        Bson::String("thread-1".to_string()),
        "token-1",
        1000,
    );

    assert_eq!(
        filter,
        doc! {
            "_id": "thread-1",
            "$or": [
                { "__skiffLeases.writer.maxExpiresAtMs": Bson::Null },
                { "__skiffLeases.writer.maxExpiresAtMs": { "$exists": false } },
                { "__skiffLeases.writer.maxExpiresAtMs": { "$gt": 1000_i64 } }
            ],
            "__skiffLeases.writer.token": "token-1",
            "__skiffLeases.writer.expiresAtMs": { "$gt": 1000_i64 }
        }
    );
}

#[test]
fn lease_claim_expires_at_clamps_initial_ttl_to_max_deadline() {
    assert_eq!(lease_claim_expires_at_ms(1_000, 60_000, None), 61_000);
    assert_eq!(
        lease_claim_expires_at_ms(1_000, 60_000, Some(31_000)),
        31_000
    );
}

#[test]
fn guarded_filter_ignores_other_type_leases() {
    let binding = thread_binding();
    let hold = DbLeaseHold {
        target_key: "Other".to_string(),
        type_name: "Other".to_string(),
        key: db_key(json!("thread-1")),
        slot: "writer".to_string(),
        token: "token-1".to_string(),
    };

    let filter = guarded_filter(&binding, doc! { "title": "Hello" }, &[hold], 1000)
        .expect("guarded filter should build");

    assert_eq!(filter, doc! { "title": "Hello" });
}
