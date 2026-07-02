use mongodb::bson::{doc, Bson, Document};
use skiff_runtime_capability_context::{DbDocument, DbKey};
use time::OffsetDateTime;

use crate::{Result, ServiceDbError};

use super::metadata::DbCollectionMetadata;

#[derive(Clone, Debug, PartialEq)]
pub struct DbLeaseHold {
    pub type_name: String,
    pub key: DbKey,
    pub slot: String,
    pub token: String,
}

#[derive(Clone, Debug)]
pub struct DbLeaseHandle {
    pub hold: DbLeaseHold,
    pub value: DbDocument,
    pub ttl_ms: u64,
}

pub(super) const SKIFF_LEASES_FIELD: &str = "__skiffLeases";
pub(super) const LEASE_TOKEN_FIELD: &str = "token";
pub(super) const LEASE_OWNER_FIELD: &str = "owner";
pub(super) const LEASE_REQUEST_ID_FIELD: &str = "requestId";
pub(super) const LEASE_CLAIMED_AT_MS_FIELD: &str = "claimedAtMs";
pub(super) const LEASE_EXPIRES_AT_MS_FIELD: &str = "expiresAtMs";
pub(super) const LEASE_MAX_EXPIRES_AT_MS_FIELD: &str = "maxExpiresAtMs";

pub fn service_db_now_ms() -> i64 {
    let now = OffsetDateTime::now_utc();
    now.unix_timestamp()
        .saturating_mul(1_000)
        .saturating_add(i64::from(now.millisecond()))
}

pub(super) fn add_ms(now_ms: i64, delta_ms: u64) -> i64 {
    now_ms.saturating_add(delta_ms.min(i64::MAX as u64) as i64)
}

pub(super) fn lease_claim_expires_at_ms(
    now_ms: i64,
    ttl_ms: u64,
    max_expires_at_ms: Option<i64>,
) -> i64 {
    let ttl_expires_at_ms = add_ms(now_ms, ttl_ms);
    max_expires_at_ms
        .map(|max| ttl_expires_at_ms.min(max))
        .unwrap_or(ttl_expires_at_ms)
}

pub(super) fn key_bson(binding: &DbCollectionMetadata, key: &DbKey) -> Result<Bson> {
    binding
        .key_filter(key)?
        .remove("_id")
        .ok_or_else(|| ServiceDbError::Decode("db key filter did not materialize _id".to_string()))
}

pub(super) fn lease_slot_path(slot: &str) -> String {
    format!("{SKIFF_LEASES_FIELD}.{slot}")
}

pub(super) fn lease_field(slot: &str, field: &str) -> String {
    format!("{}.{field}", lease_slot_path(slot))
}

pub(super) fn lease_available_filter(slot: &str, now_ms: i64) -> Document {
    let mut missing_slot = Document::new();
    missing_slot.insert(lease_slot_path(slot), doc! { "$exists": false });
    let mut expired = Document::new();
    expired.insert(
        lease_field(slot, LEASE_EXPIRES_AT_MS_FIELD),
        doc! { "$lte": now_ms },
    );
    let mut max_expired = Document::new();
    max_expired.insert(
        lease_field(slot, LEASE_MAX_EXPIRES_AT_MS_FIELD),
        doc! { "$lte": now_ms },
    );
    doc! {
        "$or": [
            missing_slot,
            expired,
            max_expired,
        ]
    }
}

pub(super) fn has_matching_lease_guards(
    binding: &DbCollectionMetadata,
    guards: &[DbLeaseHold],
) -> bool {
    !matching_lease_guards(binding, guards).is_empty()
}

pub(super) fn matching_lease_guards<'a>(
    binding: &DbCollectionMetadata,
    guards: &'a [DbLeaseHold],
) -> Vec<&'a DbLeaseHold> {
    let type_name = binding
        .canonical_type_name()
        .unwrap_or_else(|| binding.type_name.clone());
    guards
        .iter()
        .filter(|guard| guard.type_name == type_name)
        .collect()
}

pub(super) fn guarded_filter(
    binding: &DbCollectionMetadata,
    filter: Document,
    guards: &[DbLeaseHold],
    now_ms: i64,
) -> Result<Document> {
    let guard_filters = matching_lease_guards(binding, guards)
        .into_iter()
        .map(|guard| lease_guard_filter(binding, guard, now_ms))
        .collect::<Result<Vec<_>>>()?;
    Ok(and_filter(filter, guard_filters))
}

pub(super) fn lease_guard_filter(
    binding: &DbCollectionMetadata,
    guard: &DbLeaseHold,
    now_ms: i64,
) -> Result<Document> {
    let key_bson = key_bson(binding, &guard.key)?;
    let live = lease_live_key_filter(&guard.slot, key_bson.clone(), &guard.token, now_ms);
    Ok(doc! {
        "$or": [
            doc! { "_id": { "$ne": key_bson } },
            live,
        ]
    })
}

pub(super) fn lease_live_key_filter(
    slot: &str,
    key_bson: Bson,
    token: &str,
    now_ms: i64,
) -> Document {
    let mut max_null = Document::new();
    max_null.insert(lease_field(slot, LEASE_MAX_EXPIRES_AT_MS_FIELD), Bson::Null);
    let mut max_missing = Document::new();
    max_missing.insert(
        lease_field(slot, LEASE_MAX_EXPIRES_AT_MS_FIELD),
        doc! { "$exists": false },
    );
    let mut max_live = Document::new();
    max_live.insert(
        lease_field(slot, LEASE_MAX_EXPIRES_AT_MS_FIELD),
        doc! { "$gt": now_ms },
    );
    let mut live = doc! {
        "_id": key_bson,
        "$or": [max_null, max_missing, max_live],
    };
    live.insert(lease_field(slot, LEASE_TOKEN_FIELD), token);
    live.insert(
        lease_field(slot, LEASE_EXPIRES_AT_MS_FIELD),
        doc! { "$gt": now_ms },
    );
    live
}

pub(super) fn and_filter(filter: Document, clauses: Vec<Document>) -> Document {
    if clauses.is_empty() {
        return filter;
    }
    let mut items = Vec::with_capacity(clauses.len() + 1);
    items.push(Bson::Document(filter));
    items.extend(clauses.into_iter().map(Bson::Document));
    doc! { "$and": Bson::Array(items) }
}

pub(super) fn lease_document<'a>(document: &'a Document, slot: &str) -> Option<&'a Document> {
    document
        .get_document(SKIFF_LEASES_FIELD)
        .ok()
        .and_then(|leases| leases.get_document(slot).ok())
}

pub(super) fn lease_i64(lease: &Document, field: &str) -> Option<i64> {
    match lease.get(field) {
        Some(Bson::Int64(value)) => Some(*value),
        Some(Bson::Int32(value)) => Some(i64::from(*value)),
        Some(Bson::Double(value)) if value.fract() == 0.0 => Some(*value as i64),
        _ => None,
    }
}

pub(super) fn lease_lost_error(type_name: &str, slot: &str) -> ServiceDbError {
    ServiceDbError::LeaseLost(format!("db lease {type_name}.{slot} was lost"))
}
