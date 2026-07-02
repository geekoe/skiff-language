use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    LiteralIr, PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::prelude_registry::compiler_owned_type_symbol;
use skiff_compiler_projection_input::ProjectionSourceSymbolKey;

use super::index::ContractProjectionIndex;
use crate::projection_source_symbol_text;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContractTypeKey {
    Builtin {
        name: String,
        args: Vec<ContractTypeKey>,
    },
    Named(ContractNamedTypeKey),
    PackageSymbol {
        package: ContractPackageRefKey,
        symbol_path: String,
        abi_expectation: Option<String>,
    },
    AnyInterface {
        interface: Box<ContractTypeKey>,
        canonical_type_args: Vec<ContractTypeKey>,
    },
    DbObjectSymbol {
        module_path: String,
        symbol: String,
    },
    Record {
        fields: BTreeMap<String, ContractTypeKey>,
    },
    Union {
        items: Vec<ContractTypeKey>,
    },
    Nullable {
        inner: Box<ContractTypeKey>,
    },
    Literal(ContractLiteralKey),
    TypeParam {
        name: String,
    },
    Function {
        params: Vec<ContractFunctionTypeParamKey>,
        return_type: Box<ContractTypeKey>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContractNamedTypeKey {
    Public { symbol: String },
    Source { source: ProjectionSourceSymbolKey },
}

impl ContractNamedTypeKey {
    pub fn canonical_symbol(&self) -> String {
        match self {
            ContractNamedTypeKey::Public { symbol } => symbol.clone(),
            ContractNamedTypeKey::Source { source } if source.module_path().is_empty() => {
                source.symbol().to_string()
            }
            ContractNamedTypeKey::Source { source } => projection_source_symbol_text(source),
        }
    }

    pub fn canonical_symbol_ref(&self) -> std::borrow::Cow<'_, str> {
        match self {
            ContractNamedTypeKey::Public { symbol } => std::borrow::Cow::Borrowed(symbol),
            ContractNamedTypeKey::Source { source } if source.module_path().is_empty() => {
                std::borrow::Cow::Borrowed(source.symbol())
            }
            ContractNamedTypeKey::Source { source } => {
                std::borrow::Cow::Owned(projection_source_symbol_text(source))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContractFunctionTypeParamKey {
    pub name: String,
    pub ty: ContractTypeKey,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContractLiteralKey {
    Null,
    Bool(bool),
    Number(String),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContractPackageRefKey {
    PackageId { package_id: String },
    Dependency { dependency_ref: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractTypeCanonicalizationError {
    MissingModule {
        module_path: String,
    },
    MissingLocalType {
        module_path: String,
        type_index: u32,
    },
    AliasCycle {
        module_path: String,
        type_index: u32,
    },
    BoundaryUnsafeAnyInterface {
        interface_abi_id: String,
    },
    InvalidAnyInterfaceIdentity {
        interface_abi_id: String,
    },
}

impl<'a> ContractProjectionIndex<'a> {
    pub fn canonical_type_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Result<ContractTypeKey, ContractTypeCanonicalizationError> {
        let mut alias_stack = BTreeSet::new();
        self.canonical_type_ref_inner(module_path, ty, &mut alias_stack)
    }

    pub fn source_symbol_for_type_ref(&self, module_path: &str, ty: &TypeRefIr) -> Option<String> {
        self.source_key_for_type_ref(module_path, ty)
            .map(|key| projection_source_symbol_text(&key))
            .or_else(|| match ty {
                TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                    Some(service_symbol_path(symbol))
                }
                TypeRefIr::Native { .. }
                | TypeRefIr::LocalType { .. }
                | TypeRefIr::PublicationType { .. }
                | TypeRefIr::PackageSymbol { .. }
                | TypeRefIr::Record { .. }
                | TypeRefIr::Union { .. }
                | TypeRefIr::Nullable { .. }
                | TypeRefIr::Literal { .. }
                | TypeRefIr::TypeParam { .. }
                | TypeRefIr::AnyInterface { .. }
                | TypeRefIr::Function { .. } => None,
            })
    }

    pub fn source_key_for_type_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Option<ProjectionSourceSymbolKey> {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .unit_by_module_path(module_path)?
                .type_table
                .get(*type_index as usize)
                .map(|decl| ProjectionSourceSymbolKey::new(module_path, &decl.name)),
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self
                .unit_by_module_path(module_path)?
                .type_table
                .get(*type_index as usize)
                .map(|decl| ProjectionSourceSymbolKey::new(module_path, &decl.name)),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                if let Some(source_key) =
                    self.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
                {
                    return Some(source_key);
                }
                let source_module = self.source_module_for_reference_module(&symbol.module_path);
                if self
                    .unit_by_module_path(source_module)
                    .and_then(|unit| unit.declarations.types.get(&symbol.symbol))
                    .is_some()
                {
                    return Some(ProjectionSourceSymbolKey::new(
                        source_module,
                        &symbol.symbol,
                    ));
                }
                None
            }
            TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::Native { .. }
            | TypeRefIr::Record { .. }
            | TypeRefIr::Union { .. }
            | TypeRefIr::Nullable { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. }
            | TypeRefIr::AnyInterface { .. }
            | TypeRefIr::Function { .. } => None,
        }
    }

    fn canonical_type_ref_inner(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        alias_stack: &mut BTreeSet<(String, u32)>,
    ) -> Result<ContractTypeKey, ContractTypeCanonicalizationError> {
        match ty {
            TypeRefIr::Native { name, args } => Ok(ContractTypeKey::Builtin {
                name: name.clone(),
                args: self.canonical_type_refs(module_path, args, alias_stack)?,
            }),
            TypeRefIr::LocalType { type_index } => {
                self.canonical_local_type(module_path, *type_index, alias_stack)
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => self.canonical_local_type(module_path, *type_index, alias_stack),
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                self.canonical_service_symbol(symbol, alias_stack)
            }
            TypeRefIr::PackageSymbol { symbol } => Ok(package_symbol_key(symbol)),
            TypeRefIr::Record { fields } => {
                let mut canonical_fields = BTreeMap::new();
                for (name, ty) in fields {
                    canonical_fields.insert(
                        name.clone(),
                        self.canonical_type_ref_inner(module_path, ty, alias_stack)?,
                    );
                }
                Ok(ContractTypeKey::Record {
                    fields: canonical_fields,
                })
            }
            TypeRefIr::Union { items } => {
                self.canonical_union_type_ref(module_path, items, alias_stack)
            }
            TypeRefIr::Nullable { inner } => {
                let inner = self.canonical_type_ref_inner(module_path, inner, alias_stack)?;
                if matches!(inner, ContractTypeKey::Nullable { .. }) {
                    return Ok(inner);
                }
                Ok(ContractTypeKey::Nullable {
                    inner: Box::new(inner),
                })
            }
            TypeRefIr::Literal { value } => Ok(ContractTypeKey::Literal(literal_key(value))),
            TypeRefIr::TypeParam { name } => Ok(ContractTypeKey::TypeParam { name: name.clone() }),
            TypeRefIr::AnyInterface { interface } => {
                let interface_ty = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                    .map_err(|_| {
                    ContractTypeCanonicalizationError::InvalidAnyInterfaceIdentity {
                        interface_abi_id: interface.interface_abi_id.clone(),
                    }
                })?;
                let interface_key =
                    self.canonical_type_ref_inner(module_path, &interface_ty, alias_stack)?;
                let canonical_type_args = interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| self.canonical_type_ref_inner(module_path, arg, alias_stack))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(ContractTypeKey::AnyInterface {
                    interface: Box::new(interface_key),
                    canonical_type_args,
                })
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                let params = params
                    .iter()
                    .map(|param| {
                        Ok(ContractFunctionTypeParamKey {
                            name: param.name.clone(),
                            ty: self.canonical_type_ref_inner(
                                module_path,
                                &param.ty,
                                alias_stack,
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, ContractTypeCanonicalizationError>>()?;
                Ok(ContractTypeKey::Function {
                    params,
                    return_type: Box::new(self.canonical_type_ref_inner(
                        module_path,
                        return_type,
                        alias_stack,
                    )?),
                })
            }
        }
    }

    fn canonical_type_refs(
        &self,
        module_path: &str,
        items: &[TypeRefIr],
        alias_stack: &mut BTreeSet<(String, u32)>,
    ) -> Result<Vec<ContractTypeKey>, ContractTypeCanonicalizationError> {
        items
            .iter()
            .map(|item| self.canonical_type_ref_inner(module_path, item, alias_stack))
            .collect()
    }

    fn canonical_local_type(
        &self,
        module_path: &str,
        type_index: u32,
        alias_stack: &mut BTreeSet<(String, u32)>,
    ) -> Result<ContractTypeKey, ContractTypeCanonicalizationError> {
        let unit = self.unit_by_module_path(module_path).ok_or_else(|| {
            ContractTypeCanonicalizationError::MissingModule {
                module_path: module_path.to_string(),
            }
        })?;
        let type_decl = unit.type_table.get(type_index as usize).ok_or_else(|| {
            ContractTypeCanonicalizationError::MissingLocalType {
                module_path: module_path.to_string(),
                type_index,
            }
        })?;

        if let TypeDescriptorIr::Alias { target } = &type_decl.descriptor {
            if self.type_decl_is_transparent_alias(module_path, &type_decl.name) {
                let marker = (module_path.to_string(), type_index);
                if !alias_stack.insert(marker.clone()) {
                    return Err(ContractTypeCanonicalizationError::AliasCycle {
                        module_path: module_path.to_string(),
                        type_index,
                    });
                }
                let key = self.canonical_type_ref_inner(module_path, target, alias_stack);
                alias_stack.remove(&marker);
                return key;
            }
        }

        Ok(ContractTypeKey::Named(self.named_type_key_for_source_key(
            &ProjectionSourceSymbolKey::new(module_path, &type_decl.name),
        )))
    }

    fn canonical_service_symbol(
        &self,
        symbol: &ServiceSymbolRef,
        alias_stack: &mut BTreeSet<(String, u32)>,
    ) -> Result<ContractTypeKey, ContractTypeCanonicalizationError> {
        if let Some(source_key) =
            self.source_key_for_reference_symbol(&symbol.module_path, &symbol.symbol)
        {
            if let Some(declaration) = self
                .unit_by_module_path(source_key.module_path())
                .and_then(|unit| unit.declarations.types.get(source_key.symbol()))
            {
                return self.canonical_local_type(
                    source_key.module_path(),
                    declaration.type_index,
                    alias_stack,
                );
            }
        }
        let module_path = self.source_module_for_reference_module(&symbol.module_path);
        if let Some(declaration) = self
            .unit_by_module_path(module_path)
            .and_then(|unit| unit.declarations.types.get(&symbol.symbol))
        {
            return self.canonical_local_type(module_path, declaration.type_index, alias_stack);
        }

        let symbol_path = service_symbol_path(symbol);
        if let Some(standard_symbol) = self
            .prelude()
            .and_then(|prelude| prelude.known_type_symbol(&symbol_path))
            .or_else(|| compiler_owned_type_symbol(&symbol_path).map(str::to_string))
            .or_else(|| compiler_owned_type_symbol(&symbol.symbol).map(str::to_string))
        {
            if !symbol.module_path.is_empty()
                || self.prelude().is_none_or(|prelude| {
                    !prelude.package_schema_type_requires_import(&symbol.symbol)
                })
                || self
                    .prelude()
                    .is_some_and(|prelude| prelude.is_bare_raw_http_envelope_type(&symbol.symbol))
                || is_bare_compiler_owned_standard_type(&symbol.symbol)
            {
                return Ok(ContractTypeKey::Named(ContractNamedTypeKey::Public {
                    symbol: standard_symbol,
                }));
            }
        }

        Ok(ContractTypeKey::Named(
            self.named_type_key_for_service_symbol(symbol),
        ))
    }

    fn named_type_key_for_source_key(
        &self,
        source_key: &ProjectionSourceSymbolKey,
    ) -> ContractNamedTypeKey {
        self.public_schema_bindings()
            .find(|binding| {
                binding.source_module == source_key.module_path()
                    && binding.source_symbol == source_key.symbol()
            })
            .map(|binding| binding.public_path.as_str())
            .map(|symbol| ContractNamedTypeKey::Public {
                symbol: symbol.to_string(),
            })
            .unwrap_or_else(|| ContractNamedTypeKey::Source {
                source: source_key.clone(),
            })
    }

    fn named_type_key_for_service_symbol(&self, symbol: &ServiceSymbolRef) -> ContractNamedTypeKey {
        ContractNamedTypeKey::Source {
            source: ProjectionSourceSymbolKey::new(&symbol.module_path, &symbol.symbol),
        }
    }

    fn canonical_union_type_ref(
        &self,
        module_path: &str,
        items: &[TypeRefIr],
        alias_stack: &mut BTreeSet<(String, u32)>,
    ) -> Result<ContractTypeKey, ContractTypeCanonicalizationError> {
        let keys = self.canonical_type_refs(module_path, items, alias_stack)?;
        let mut non_null = Vec::new();
        let mut nulls = Vec::new();
        for key in keys {
            if is_null_type_key(&key) {
                nulls.push(key);
            } else {
                non_null.push(key);
            }
        }
        if !nulls.is_empty() && non_null.len() == 1 {
            return Ok(ContractTypeKey::Nullable {
                inner: Box::new(non_null.remove(0)),
            });
        }
        non_null.extend(nulls);
        Ok(ContractTypeKey::Union { items: non_null })
    }
}

impl ContractProjectionIndex<'_> {
    fn type_decl_is_transparent_alias(&self, module_path: &str, local_name: &str) -> bool {
        self.unit_by_module_path(module_path).is_none_or(|unit| {
            !unit
                .source_map
                .spans
                .iter()
                .any(|span| span.name.as_deref() == Some(local_name) && span.kind == "type")
        })
    }
}

fn service_symbol_path(symbol: &ServiceSymbolRef) -> String {
    if symbol.module_path.is_empty() {
        symbol.symbol.clone()
    } else {
        symbol.symbol_path()
    }
}

fn package_symbol_key(symbol: &PackageSymbolRef) -> ContractTypeKey {
    ContractTypeKey::PackageSymbol {
        package: package_ref_key(&symbol.package),
        symbol_path: symbol.symbol_path.clone(),
        abi_expectation: symbol.abi_expectation.clone(),
    }
}

fn package_ref_key(package: &PackageRefIr) -> ContractPackageRefKey {
    match package {
        PackageRefIr::PackageId { package_id } => ContractPackageRefKey::PackageId {
            package_id: package_id.clone(),
        },
        PackageRefIr::Dependency { dependency_ref } => ContractPackageRefKey::Dependency {
            dependency_ref: dependency_ref.clone(),
        },
    }
}

fn literal_key(value: &LiteralIr) -> ContractLiteralKey {
    match value {
        LiteralIr::Null => ContractLiteralKey::Null,
        LiteralIr::Bool { value } => ContractLiteralKey::Bool(*value),
        LiteralIr::Number { value } => ContractLiteralKey::Number(value.to_string()),
        LiteralIr::String { value } => ContractLiteralKey::String(value.clone()),
    }
}

fn is_null_type_key(key: &ContractTypeKey) -> bool {
    matches!(
        key,
        ContractTypeKey::Builtin { name, args } if args.is_empty() && name == "null"
    ) || matches!(key, ContractTypeKey::Literal(ContractLiteralKey::Null))
}

fn is_bare_compiler_owned_standard_type(name: &str) -> bool {
    matches!(
        name,
        "WebSocketConnectResult"
            | "WebSocketConnection"
            | "WebSocketReceiveEvent"
            | "ConnectionMessage"
            | "TextConnectionMessage"
            | "BinaryConnectionMessage"
    )
}
