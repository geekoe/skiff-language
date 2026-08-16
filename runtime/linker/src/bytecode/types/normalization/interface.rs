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
        let identity = match identity {
            TypeRefIr::PackageSymbol { .. } => identity,
            TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. } => self.normalize(&identity)?,
            other => return Ok(other),
        };
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
        let mut records = owner
            .artifact()
            .bytecode_schema_records
            .values()
            .filter(|record| {
                record.package_id == *package_id && record.stable_schema_key == symbol.symbol_path
            });
        let Some(record) = records.next() else {
            return self.resolve_implementation_interface_schema(identity);
        };
        if records.next().is_some() {
            return Err(self.error(format!(
                "interface package symbol {:?} has multiple bytecode schema records",
                symbol.symbol_path
            )));
        }
        return Ok(interface_schema(record));
    }

    fn interface_public_schema_for_implementation_symbol(
        &self,
        owner: &skiff_runtime_loader::HydratedBytecodePackage,
        package_id: &str,
        symbol_path: &str,
    ) -> Result<Option<skiff_artifact_model::PackageSchemaTypeRecord>, BytecodeLinkError> {
        let Some(implementation_link) =
            owner.artifact().implementation_links.types.get(symbol_path)
        else {
            return Ok(None);
        };
        let mut public_paths = owner
            .artifact()
            .package_local_abi
            .public_symbols
            .keys()
            .filter(|public_path| {
                owner
                    .artifact()
                    .implementation_links
                    .types
                    .get(*public_path)
                    .is_some_and(|link| same_type_export(link, implementation_link))
            });
        let public_path = match (public_paths.next(), public_paths.next()) {
            (Some(public_path), None) => public_path,
            (None, _) => return Ok(None),
            (Some(_), Some(_)) => {
                return Err(self.error(format!(
                    "interface implementation symbol {symbol_path:?} maps to multiple public schema paths"
                )))
            }
        };
        let mut records = owner
            .artifact()
            .bytecode_schema_records
            .values()
            .filter(|record| {
                record.package_id == package_id && record.stable_schema_key == *public_path
            });
        let record = match (records.next(), records.next()) {
            (Some(record), None) => record,
            (None, _) => return Ok(None),
            (Some(_), Some(_)) => {
                return Err(self.error(format!(
                "interface public schema path {public_path:?} has multiple bytecode schema records"
            )))
            }
        };
        Ok(Some(record.clone()))
    }

    fn resolve_implementation_interface_schema(
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
        let Some(record) = self.interface_public_schema_for_implementation_symbol(
            owner,
            package_id,
            &symbol.symbol_path,
        )?
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

fn interface_schema(record: &skiff_artifact_model::PackageSchemaTypeRecord) -> TypeRefIr {
    TypeRefIr::PackageSchema {
        package_id: record.package_id.clone(),
        stable_schema_key: record.stable_schema_key.clone(),
        package_schema_type_id: record.package_schema_type_id.clone(),
    }
}

fn same_type_export(
    left: &skiff_artifact_model::TypeExport,
    right: &skiff_artifact_model::TypeExport,
) -> bool {
    left.file == right.file
        && left.type_index == right.type_index
        && left.is_interface == right.is_interface
        && left.descriptor == right.descriptor
        && left.type_params == right.type_params
        && left.interface_methods == right.interface_methods
        && left.actor == right.actor
}
