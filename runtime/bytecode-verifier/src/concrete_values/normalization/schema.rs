use skiff_artifact_model::{PackageSchemaTypeId, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION};

use super::TypeOwnerNormalizer;
use crate::VerificationError;

impl TypeOwnerNormalizer<'_, '_> {
    pub(super) fn normalize_package_schema(
        &self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<TypeRefIr, VerificationError> {
        if package_id.is_empty()
            || stable_schema_key.is_empty()
            || package_schema_type_id.as_str().is_empty()
        {
            return Err(self.violation("PackageSchema owner triple is incomplete"));
        }
        let owner = self
            .resolver
            .unique_package_owner(package_id)
            .map_err(|error| self.authority_violation("resolving PackageSchema owner", error))?;
        if owner.artifact().schema_version != PACKAGE_ARTIFACT_SCHEMA_VERSION {
            return Err(self.violation(format!(
                "PackageSchema owner {package_id:?} is not a {PACKAGE_ARTIFACT_SCHEMA_VERSION} artifact"
            )));
        }
        let record = owner
            .artifact()
            .bytecode_schema_records
            .get(package_schema_type_id)
            .ok_or_else(|| {
                self.violation(format!(
                    "PackageSchema {package_id}:{stable_schema_key}:{package_schema_type_id} has no exact descriptor"
                ))
            })?;
        if record.package_id != package_id
            || record.stable_schema_key != stable_schema_key
            || &record.package_schema_type_id != package_schema_type_id
        {
            return Err(self.violation(format!(
                "PackageSchema {package_id}:{stable_schema_key}:{package_schema_type_id} disagrees with its exact descriptor triple"
            )));
        }
        Ok(TypeRefIr::PackageSchema {
            package_id: package_id.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: package_schema_type_id.clone(),
        })
    }
}
