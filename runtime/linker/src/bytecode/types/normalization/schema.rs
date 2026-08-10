use skiff_artifact_model::{PackageSchemaTypeId, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION};

use crate::bytecode::BytecodeLinkError;

use super::TypeNormalizer;

impl TypeNormalizer<'_> {
    pub(super) fn normalize_package_schema(
        &self,
        package_id: &str,
        stable_schema_key: &str,
        package_schema_type_id: &PackageSchemaTypeId,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let owner = self.unique_package_owner(package_id)?;
        if owner.artifact().schema_version != PACKAGE_ARTIFACT_SCHEMA_VERSION {
            return Err(self.error(format!(
                "PackageSchema owner {package_id:?} is not a v14 PackageArtifact"
            )));
        }
        let record = owner
            .artifact()
            .bytecode_schema_records
            .get(package_schema_type_id)
            .ok_or_else(|| {
                self.error(format!(
                    "PackageSchema {package_id}:{stable_schema_key}:{package_schema_type_id} has no exact bytecode descriptor"
                ))
            })?;
        if record.package_id != package_id
            || record.stable_schema_key != stable_schema_key
            || &record.package_schema_type_id != package_schema_type_id
        {
            return Err(self.error(format!(
                "PackageSchema {package_id}:{stable_schema_key}:{package_schema_type_id} disagrees with its v14 bytecode descriptor triple"
            )));
        }
        Ok(TypeRefIr::PackageSchema {
            package_id: package_id.to_string(),
            stable_schema_key: stable_schema_key.to_string(),
            package_schema_type_id: package_schema_type_id.clone(),
        })
    }
}
