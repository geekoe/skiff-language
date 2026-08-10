use skiff_artifact_model::{InterfaceInstantiationRef, TypeRefIr};

use crate::bytecode::BytecodeLinkError;

use super::TypeNormalizer;

impl TypeNormalizer<'_> {
    pub(super) fn normalize_any_interface(
        &self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<TypeRefIr, BytecodeLinkError> {
        let identity =
            serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(|error| {
                self.error(format!(
                    "interface ABI identity is not an exact TypeRefIr: {error}"
                ))
            })?;
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
}
