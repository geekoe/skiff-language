use std::collections::BTreeMap;

use skiff_artifact_model::{NominalTypeRefBaseIr, TypeRefIr};

use super::TypeOwnerNormalizer;
use crate::{VerificationError, VerificationObligation};

impl TypeOwnerNormalizer<'_, '_> {
    pub(super) fn normalize(&mut self, ty: &TypeRefIr) -> Result<TypeRefIr, VerificationError> {
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
                    .collect::<Result<BTreeMap<_, _>, VerificationError>>()?,
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
            TypeRefIr::LocalType { type_index } => {
                return Err(self.violation(format!(
                    "ownerless local type index {type_index} cannot enter a linked type"
                )));
            }
            TypeRefIr::DbObjectSymbol { symbol } => {
                return Err(self.violation(format!(
                    "DB object type {} has no package-owned bytecode identity",
                    symbol.symbol_path()
                )));
            }
            TypeRefIr::TypeParam { name } => {
                return Err(self.violation(format!(
                    "UnknownTypeParameter: type parameter {name:?} remains after concrete substitution"
                )));
            }
            TypeRefIr::Function { .. } => {
                return Err(
                    self.violation("function type has no complete package-owned bytecode identity")
                );
            }
            TypeRefIr::AnyInterface { .. } => {
                return Err(VerificationError::ProofUnavailable {
                    obligation: VerificationObligation::InterfaceSignature,
                    location: self.location,
                });
            }
        })
    }

    fn normalize_nominal_base(
        &mut self,
        base: &NominalTypeRefBaseIr,
    ) -> Result<NominalTypeRefBaseIr, VerificationError> {
        let normalized = match base {
            NominalTypeRefBaseIr::LocalType { type_index } => {
                return Err(self.violation(format!(
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
        type_ref_to_nominal_base(normalized).map_err(|detail| self.violation(detail))
    }

    fn normalize_types(
        &mut self,
        types: &[TypeRefIr],
    ) -> Result<Vec<TypeRefIr>, VerificationError> {
        types.iter().map(|ty| self.normalize(ty)).collect()
    }
}

fn type_ref_to_nominal_base(ty: TypeRefIr) -> Result<NominalTypeRefBaseIr, &'static str> {
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
        _ => Err("normalized applied-nominal base is not owner-complete"),
    }
}
