use skiff_artifact_model::{InterfaceInstantiationRef, PackageRefIr, TypeRefIr};

use crate::bytecode::BytecodeLinkError;

use super::TypeNormalizer;

impl TypeNormalizer<'_> {
    pub(super) fn normalize_any_interface(
        &self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let identity = self.resolve_interface_package_symbol(
            serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(|error| {
                self.error(format!(
                    "interface ABI identity is not an exact TypeRefIr: {error}"
                ))
            })?,
        )?;
        let identity = self.normalize(&identity)?;
        let bytes = skiff_canonical_json::canonical_json_bytes(&identity).map_err(|error| {
            self.error(format!(
                "normalized interface ABI identity cannot be canonically encoded: {error}"
            ))
        })?;
        let interface_abi_id = String::from_utf8(bytes).map_err(|error| {
            self.error(format!(
                "normalized interface ABI identity is not UTF-8: {error}"
            ))
        })?;
        Ok(TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id,
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|ty| self.normalize(ty))
                    .collect::<Result<_, _>>()?,
            },
        })
    }

    fn resolve_interface_package_symbol(
        &self,
        identity: TypeRefIr,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let TypeRefIr::PackageSymbol { symbol } = &identity else {
            return Ok(identity);
        };
        let PackageRefIr::PackageId { package_id } = &symbol.package else {
            return Ok(identity);
        };
        let Some(owner) = self
            .deployment
            .packages()
            .values()
            .find(|package| package.reference().package_id.as_str() == package_id)
        else {
            return Ok(identity);
        };
        let Some(record) = owner
            .artifact()
            .bytecode_schema_records
            .values()
            .find(|record| {
                record.package_id == *package_id && record.stable_schema_key == symbol.symbol_path
            })
        else {
            return Ok(identity);
        };
        Ok(TypeRefIr::PackageSchema {
            package_id: record.package_id.clone(),
            stable_schema_key: record.stable_schema_key.clone(),
            package_schema_type_id: record.package_schema_type_id.clone(),
        })
    }
}
