use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{PackageSchemaTypeId, PackageSchemaTypeRecord};

/// Read-only package-schema records admitted for one service execution.
///
/// The map itself is normally owned by an `Arc` at the host/eval seam. Boundary
/// plans borrow that map and preserve the record `Arc`s without cloning record
/// payloads.
pub type ServiceSchemaRecords = BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>;
