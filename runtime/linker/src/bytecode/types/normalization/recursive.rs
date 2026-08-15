use skiff_artifact_model::{NominalTypeRefBaseIr, TypeRefIr};

use crate::bytecode::BytecodeLinkError;

use super::TypeNormalizer;

impl TypeNormalizer<'_> {
    pub(super) fn normalize(&self, ty: &TypeRefIr) -> Result<TypeRefIr, BytecodeLinkError> {
        Ok(match ty {
            TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
                name: name.clone(),
                args: self.normalize_types(args)?,
            },
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self.normalize_publication_type(module_path, *type_index)?,
            TypeRefIr::ServiceSymbol { symbol } => {
                self.normalize_service_symbol(&symbol.module_path, &symbol.symbol)?
            }
            TypeRefIr::PackageSymbol { symbol } => self.normalize_package_symbol(symbol)?,
            TypeRefIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => self.normalize_package_schema(
                package_id,
                stable_schema_key,
                package_schema_type_id,
            )?,
            TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
                base: self.normalize_nominal_base(base)?,
                arguments: self.normalize_types(arguments)?,
            },
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field)| Ok((name.clone(), self.normalize(field)?)))
                    .collect::<Result<_, BytecodeLinkError>>()?,
            },
            TypeRefIr::Union { items } => TypeRefIr::Union {
                items: self.normalize_types(items)?,
            },
            TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
                inner: Box::new(self.normalize(inner)?),
            },
            TypeRefIr::Literal { value } => TypeRefIr::Literal {
                value: value.clone(),
            },
            TypeRefIr::AnyInterface { interface } => self.normalize_any_interface(interface)?,
            TypeRefIr::LocalType { type_index } => {
                return Err(self.error(format!(
                    "ownerless local type index {type_index} cannot enter a linked type"
                )));
            }
            TypeRefIr::DbObjectSymbol { symbol } => {
                self.normalize_service_symbol(&symbol.module_path, &symbol.symbol)?
            }
            TypeRefIr::TypeParam { name } => {
                return Err(self.error(format!(
                    "type parameter {name:?} remains after concrete substitution"
                )));
            }
            TypeRefIr::Function { .. } => {
                return Err(self.error(
                    "function type has no complete package-owned bytecode identity".to_string(),
                ));
            }
        })
    }

    fn normalize_nominal_base(
        &self,
        base: &NominalTypeRefBaseIr,
    ) -> Result<NominalTypeRefBaseIr, BytecodeLinkError> {
        let normalized = match base {
            NominalTypeRefBaseIr::LocalType { type_index } => {
                return Err(self.error(format!(
                    "ownerless local nominal type index {type_index} cannot enter a linked type"
                )));
            }
            NominalTypeRefBaseIr::PublicationType {
                module_path,
                type_index,
            } => self.normalize_publication_type(module_path, *type_index)?,
            NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
                self.normalize_service_symbol(&symbol.module_path, &symbol.symbol)?
            }
            NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                self.normalize_package_symbol(symbol)?
            }
            NominalTypeRefBaseIr::PackageSchema {
                package_id,
                stable_schema_key,
                package_schema_type_id,
            } => self.normalize_package_schema(
                package_id,
                stable_schema_key,
                package_schema_type_id,
            )?,
        };
        type_ref_to_nominal_base(normalized).map_err(|detail| self.error(detail))
    }

    fn normalize_types(&self, types: &[TypeRefIr]) -> Result<Vec<TypeRefIr>, BytecodeLinkError> {
        types.iter().map(|ty| self.normalize(ty)).collect()
    }
}

fn type_ref_to_nominal_base(ty: TypeRefIr) -> Result<NominalTypeRefBaseIr, String> {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => Ok(NominalTypeRefBaseIr::PackageSymbol { symbol }),
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        }),
        _ => {
            Err("normalized applied-nominal base is not an owner-complete nominal type".to_string())
        }
    }
}
