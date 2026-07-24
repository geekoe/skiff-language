use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{PackageSchemaTypeId, PackageSchemaTypeRecord};

/// Read-only package-schema records admitted for one service execution.
///
/// The map itself is normally owned by an `Arc` at the host/eval seam. Boundary
/// plans borrow that map and preserve the record `Arc`s without cloning record
/// payloads.
/// Package-owned schema records admitted for one service contract.
///
/// The service selects the exact requirements but does not own these records.
pub type PackageSchemaRecords = BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>;
