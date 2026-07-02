use std::collections::{BTreeMap, BTreeSet};

use crate::projection_source_symbol_text;
use skiff_artifact_model::{
    ExecutableIr, FunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr, TypeDescriptorIr,
    TypeRefIr,
};
use skiff_compiler_projection_input::{ProjectionSourceSymbolKey, ProjectionView};

use super::{
    abi_type_id_for_named_key,
    model::{
        ContractAliasProjection, ContractApiBindingProjection, ContractFunctionParamProjection,
        ContractInterfaceOperationProjection, ContractInterfaceProjection,
        ContractOperationBindingProjection, ContractProjection, ContractProjectionTypeBinding,
        ContractTypeDescriptorProjection, ContractTypeKind, ContractTypeProjection,
    },
    ContractNamedTypeKey, ContractProjectionIndex, ContractTypeCanonicalizationError,
    ContractTypeKey,
};
use crate::prelude::PreludeProjection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContractProjectionError {
    MissingApiSourceModule {
        public_path: String,
        source_module: String,
    },
    DuplicatePublicSymbol {
        public_name: String,
        existing_source: String,
        duplicate_source: String,
        duplicate_kind: ContractProjectionSymbolKind,
    },
    DuplicateInterfaceOperation {
        interface_public_name: String,
        operation_name: String,
    },
    MissingImplementationBinding {
        interface_public_name: String,
        operation_name: String,
        implementation_module_path: String,
        implementation_type_name: String,
    },
    DuplicateInterfaceBinding {
        interface_public_name: String,
        existing_implementation_source: String,
        duplicate_implementation_source: String,
    },
    TypeCanonicalization {
        context: String,
        source: ContractTypeCanonicalizationError,
    },
    PrivateContractTypeReference {
        context: String,
        source_symbol: String,
    },
    UnexpectedAliasDescriptor {
        module_path: String,
        local_name: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractProjectionSymbolKind {
    Type,
    Alias,
    Interface,
}

pub fn project_contract_projection(
    input: ProjectionView<'_>,
    prelude: &PreludeProjection,
) -> Result<ContractProjection, ContractProjectionError> {
    ContractProjectionProjector::new(
        ContractProjectionIndex::from_projection_input_with_prelude(input, Some(prelude)),
        prelude,
    )
    .project()
}

struct ContractProjectionProjector<'a> {
    index: ContractProjectionIndex<'a>,
    projection: ContractProjection,
}

impl<'a> ContractProjectionProjector<'a> {
    fn new(index: ContractProjectionIndex<'a>, prelude: &PreludeProjection) -> Self {
        Self {
            index,
            projection: ContractProjection {
                prelude: prelude.clone(),
                ..ContractProjection::default()
            },
        }
    }

    fn project(mut self) -> Result<ContractProjection, ContractProjectionError> {
        // TODO: Later projection slices should keep tightening signature closure,
        // duplicate detection, and contract violation reporting parity.
        let modules = self
            .index
            .public_schema_bindings()
            .map(|binding| binding.source_module.clone())
            .collect::<BTreeSet<_>>();

        for module_path in modules {
            if self.index.unit_by_module_path(&module_path).is_none() {
                let public_path = self
                    .index
                    .public_schema_bindings()
                    .find(|binding| binding.source_module == module_path)
                    .map(|binding| binding.public_path.clone())
                    .unwrap_or_default();
                return Err(ContractProjectionError::MissingApiSourceModule {
                    public_path,
                    source_module: module_path,
                });
            }
            self.extend_module(&module_path)?;
        }

        self.extend_projected_type_closure()?;
        self.validate_public_contract_projection()?;
        Ok(self.projection)
    }

    fn extend_module(&mut self, module_path: &str) -> Result<(), ContractProjectionError> {
        let Some(unit) = self.index.unit_by_module_path(module_path) else {
            return Ok(());
        };

        for local_name in unit.declarations.types.keys() {
            let Some(public_name) = self.public_schema_symbol(module_path, local_name) else {
                continue;
            };
            let Some(binding) = self
                .index
                .type_binding_by_module_local_name(module_path, local_name)
            else {
                continue;
            };

            match binding.kind {
                ContractTypeKind::Alias => self.extend_alias(&binding, &public_name)?,
                ContractTypeKind::Interface => {
                    let interface_public_name = self
                        .public_interface_symbol(module_path, local_name)
                        .unwrap_or(public_name);
                    self.extend_interface_binding(
                        module_path,
                        local_name,
                        &interface_public_name,
                        None,
                    )?;
                }
                ContractTypeKind::Type => self.extend_type_binding(&binding, &public_name)?,
            }
        }

        Ok(())
    }

    fn extend_type_binding(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        public_name: &str,
    ) -> Result<(), ContractProjectionError> {
        let implemented_interfaces = self.implemented_public_interfaces(binding);
        if !implemented_interfaces.is_empty() {
            for implemented in implemented_interfaces {
                let interface_public_name = implemented.public_name.clone();
                self.extend_interface_binding(
                    &implemented.module_path,
                    &implemented.local_name,
                    &interface_public_name,
                    Some(binding),
                )?;
            }
            return Ok(());
        }

        let public_callable_methods =
            self.public_callable_methods_for_type(binding.module_path, binding.local_name);
        if !public_callable_methods.is_empty() {
            self.extend_synthetic_interface(binding, public_name, public_callable_methods)?;
            return Ok(());
        }

        self.extend_schema_type(binding, public_name)
    }

    fn extend_schema_type(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        public_name: &str,
    ) -> Result<(), ContractProjectionError> {
        let descriptor = self.project_type_descriptor(
            binding.module_path,
            binding.local_name,
            &binding.type_decl.descriptor,
        )?;
        let implements = binding
            .implements()
            .iter()
            .map(|implemented| {
                self.canonical_type_ref(
                    binding.module_path,
                    implemented,
                    format!(
                        "type {}.{} implements",
                        binding.module_path, binding.local_name
                    ),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.ensure_public_symbol_available(
            public_name,
            binding.module_path,
            binding.local_name,
            ContractProjectionSymbolKind::Type,
        )?;
        self.record_type_symbol(public_name, binding.module_path, binding.local_name);
        self.projection.types.insert(
            public_name.to_string(),
            ContractTypeProjection {
                public_name: public_name.to_string(),
                source_module: binding.module_path.to_string(),
                source_name: binding.local_name.to_string(),
                type_params: binding.type_decl.type_params.clone(),
                descriptor,
                discriminator: binding.discriminator().map(str::to_string),
                implements,
            },
        );
        Ok(())
    }

    fn extend_alias(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        public_name: &str,
    ) -> Result<(), ContractProjectionError> {
        let TypeDescriptorIr::Alias { target } = &binding.type_decl.descriptor else {
            return Err(ContractProjectionError::UnexpectedAliasDescriptor {
                module_path: binding.module_path.to_string(),
                local_name: binding.local_name.to_string(),
            });
        };
        let target = self.canonical_type_ref(
            binding.module_path,
            target,
            format!("alias {}.{}", binding.module_path, binding.local_name),
        )?;
        self.ensure_public_symbol_available(
            public_name,
            binding.module_path,
            binding.local_name,
            ContractProjectionSymbolKind::Alias,
        )?;
        self.record_type_symbol(public_name, binding.module_path, binding.local_name);
        self.projection.aliases.insert(
            public_name.to_string(),
            ContractAliasProjection {
                public_name: public_name.to_string(),
                source_module: binding.module_path.to_string(),
                source_name: binding.local_name.to_string(),
                type_params: binding.type_decl.type_params.clone(),
                target,
                discriminator: binding.type_decl.discriminator.clone(),
                transparent: binding.transparent_alias,
            },
        );
        Ok(())
    }

    fn extend_interface_binding(
        &mut self,
        module_path: &str,
        local_name: &str,
        public_name: &str,
        implementation: Option<&ContractProjectionTypeBinding<'a>>,
    ) -> Result<(), ContractProjectionError> {
        if !self.projection.interfaces.contains_key(public_name) {
            let interface = self
                .index
                .interface_decl_by_module_local_name(module_path, local_name);
            if let Some(interface) = interface {
                self.ensure_public_symbol_available(
                    public_name,
                    module_path,
                    local_name,
                    ContractProjectionSymbolKind::Interface,
                )?;
                let projected =
                    self.project_interface(module_path, local_name, public_name, interface)?;
                self.record_interface_symbol(public_name, module_path, local_name);
                self.projection
                    .interfaces
                    .insert(public_name.to_string(), projected);
            }
        }

        if let Some(implementation) = implementation {
            self.ensure_interface_binding_available(public_name, implementation)?;
            let binding = self.interface_api_binding(
                module_path,
                public_name,
                self.index
                    .interface_decl_by_module_local_name(module_path, local_name),
                implementation,
            )?;
            self.projection
                .api_bindings
                .insert(public_name.to_string(), binding);
        }

        Ok(())
    }

    fn extend_synthetic_interface(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        public_name: &str,
        method_names: Vec<String>,
    ) -> Result<(), ContractProjectionError> {
        let mut operations = Vec::new();
        let mut operation_bindings = BTreeMap::new();
        for method_name in method_names {
            let executable_symbol = format!("{}.{}", binding.local_name, method_name);
            let Some(executable) = self
                .index
                .executable_by_module_symbol(binding.module_path, &executable_symbol)
            else {
                continue;
            };
            operations.push(self.project_executable_operation(
                binding,
                public_name,
                &method_name,
                executable,
            )?);
            operation_bindings.insert(
                method_name.clone(),
                ContractOperationBindingProjection {
                    module_path: binding.module_path.to_string(),
                    type_name: binding.local_name.to_string(),
                    method_name,
                    executable_symbol,
                    signature_module_path: None,
                },
            );
        }

        self.ensure_public_symbol_available(
            public_name,
            binding.module_path,
            binding.local_name,
            ContractProjectionSymbolKind::Interface,
        )?;
        self.record_interface_symbol(public_name, binding.module_path, binding.local_name);
        self.projection.interfaces.insert(
            public_name.to_string(),
            ContractInterfaceProjection {
                public_name: public_name.to_string(),
                source_module: binding.module_path.to_string(),
                source_name: binding.local_name.to_string(),
                operations,
            },
        );
        self.projection.api_bindings.insert(
            public_name.to_string(),
            ContractApiBindingProjection {
                source_module: binding.module_path.to_string(),
                source_symbol: binding.local_name.to_string(),
                operations: operation_bindings,
            },
        );
        Ok(())
    }

    fn interface_api_binding(
        &self,
        interface_module_path: &str,
        interface_public_name: &str,
        interface: Option<&InterfaceDeclIr>,
        implementation: &ContractProjectionTypeBinding<'a>,
    ) -> Result<ContractApiBindingProjection, ContractProjectionError> {
        let mut operations = BTreeMap::new();
        if let Some(interface) = interface {
            let mut seen_operation_names = BTreeSet::new();
            for operation in &interface.operations {
                if !seen_operation_names.insert(operation.name.as_str()) {
                    return Err(ContractProjectionError::DuplicateInterfaceOperation {
                        interface_public_name: interface_public_name.to_string(),
                        operation_name: operation.name.clone(),
                    });
                }
                let executable_symbol = format!("{}.{}", implementation.local_name, operation.name);
                let source_key =
                    ProjectionSourceSymbolKey::new(implementation.module_path, &executable_symbol);
                if self.index.is_public_callable_source_key(&source_key)
                    && self
                        .index
                        .executable_by_module_symbol(implementation.module_path, &executable_symbol)
                        .is_some()
                {
                    operations.insert(
                        operation.name.clone(),
                        ContractOperationBindingProjection {
                            module_path: implementation.module_path.to_string(),
                            type_name: implementation.local_name.to_string(),
                            method_name: operation.name.clone(),
                            executable_symbol,
                            signature_module_path: Some(interface_module_path.to_string()),
                        },
                    );
                } else {
                    return Err(ContractProjectionError::MissingImplementationBinding {
                        interface_public_name: interface_public_name.to_string(),
                        operation_name: operation.name.clone(),
                        implementation_module_path: implementation.module_path.to_string(),
                        implementation_type_name: implementation.local_name.to_string(),
                    });
                }
            }
        }

        Ok(ContractApiBindingProjection {
            source_module: implementation.module_path.to_string(),
            source_symbol: implementation.local_name.to_string(),
            operations,
        })
    }

    fn project_interface(
        &self,
        module_path: &str,
        local_name: &str,
        public_name: &str,
        interface: &InterfaceDeclIr,
    ) -> Result<ContractInterfaceProjection, ContractProjectionError> {
        let mut seen_operation_names = BTreeSet::new();
        for operation in &interface.operations {
            if !seen_operation_names.insert(operation.name.as_str()) {
                return Err(ContractProjectionError::DuplicateInterfaceOperation {
                    interface_public_name: public_name.to_string(),
                    operation_name: operation.name.clone(),
                });
            }
        }
        let operations = interface
            .operations
            .iter()
            .map(|operation| self.project_interface_operation(module_path, operation))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ContractInterfaceProjection {
            public_name: public_name.to_string(),
            source_module: module_path.to_string(),
            source_name: local_name.to_string(),
            operations,
        })
    }

    fn project_interface_operation(
        &self,
        module_path: &str,
        operation: &InterfaceOperationIr,
    ) -> Result<ContractInterfaceOperationProjection, ContractProjectionError> {
        Ok(ContractInterfaceOperationProjection {
            name: operation.name.clone(),
            type_params: operation.type_params.clone(),
            params: self.project_function_params(
                module_path,
                &operation.params,
                format!("interface operation {} params", operation.name),
            )?,
            return_type: self.canonical_type_ref(
                module_path,
                &operation.return_type,
                format!("interface operation {} return", operation.name),
            )?,
            is_native: operation.is_native,
            is_provider: operation.is_provider,
            is_static: operation.is_static,
            implicit_self: operation
                .implicit_self
                .as_ref()
                .map(|ty| {
                    self.canonical_type_ref(
                        module_path,
                        ty,
                        format!("interface operation {} self", operation.name),
                    )
                })
                .transpose()?,
        })
    }

    fn project_executable_operation(
        &self,
        binding: &ContractProjectionTypeBinding<'a>,
        public_name: &str,
        method_name: &str,
        executable: &ExecutableIr,
    ) -> Result<ContractInterfaceOperationProjection, ContractProjectionError> {
        let params =
            self.executable_protocol_params(binding, public_name, method_name, executable)?;
        Ok(ContractInterfaceOperationProjection {
            name: method_name.to_string(),
            // TODO: ExecutableIr currently carries lowered signature types but not source-level
            // method generic parameter names or native/provider/static flags. Preserve the
            // callable signature here and extend this projection once that metadata is seeded.
            type_params: Vec::new(),
            params,
            return_type: self.canonical_type_ref(
                binding.module_path,
                &executable.return_type,
                format!("executable operation {method_name} return"),
            )?,
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: executable
                .self_type
                .as_ref()
                .map(|ty| {
                    self.canonical_type_ref(
                        binding.module_path,
                        ty,
                        format!("executable operation {method_name} self"),
                    )
                })
                .transpose()?,
        })
    }

    fn executable_protocol_params(
        &self,
        binding: &ContractProjectionTypeBinding<'a>,
        public_name: &str,
        method_name: &str,
        executable: &ExecutableIr,
    ) -> Result<Vec<ContractFunctionParamProjection>, ContractProjectionError> {
        let params = match executable.params.first() {
            Some(first_param)
                if first_param.name == "self"
                    && self.is_explicit_self_receiver(
                        binding.module_path,
                        public_name,
                        method_name,
                        &first_param.ty,
                    )? =>
            {
                &executable.params[1..]
            }
            _ => executable.params.as_slice(),
        };

        params
            .iter()
            .map(|param| {
                Ok(ContractFunctionParamProjection {
                    name: param.name.clone(),
                    ty: self.canonical_type_ref(
                        binding.module_path,
                        &param.ty,
                        format!("executable operation {method_name} param {}", param.name),
                    )?,
                })
            })
            .collect()
    }

    fn is_explicit_self_receiver(
        &self,
        module_path: &str,
        public_name: &str,
        method_name: &str,
        ty: &TypeRefIr,
    ) -> Result<bool, ContractProjectionError> {
        let receiver = self.canonical_type_ref(
            module_path,
            ty,
            format!("executable operation {method_name} explicit self"),
        )?;

        // Only a Named type can be the self-receiver of a public type's method.
        let ContractTypeKey::Named(ref receiver_named) = receiver else {
            return Ok(false);
        };

        // The expected identity: the public named key for `public_name`.
        let expected_named = ContractNamedTypeKey::Public {
            symbol: public_name.to_string(),
        };

        // T-P3a-switch: compare by AbiTypeId (nominal identity) rather than by string.
        // If both sides have an anchor-derived AbiTypeId, use that.  If either is missing
        // (external/std symbol without a declaration anchor), fall back to structural equality
        // so the existing behaviour for std types is preserved.
        let receiver_id = abi_type_id_for_named_key(receiver_named, &self.index);
        let expected_id = abi_type_id_for_named_key(&expected_named, &self.index);
        match (receiver_id, expected_id) {
            (Some(r_id), Some(e_id)) => Ok(r_id == e_id),
            _ => Ok(receiver == ContractTypeKey::Named(expected_named)),
        }
    }

    fn project_function_params(
        &self,
        module_path: &str,
        params: &[FunctionTypeParamIr],
        context: String,
    ) -> Result<Vec<ContractFunctionParamProjection>, ContractProjectionError> {
        params
            .iter()
            .map(|param| {
                Ok(ContractFunctionParamProjection {
                    name: param.name.clone(),
                    ty: self.canonical_type_ref(
                        module_path,
                        &param.ty,
                        format!("{context} param {}", param.name),
                    )?,
                })
            })
            .collect()
    }

    fn project_type_descriptor(
        &self,
        module_path: &str,
        local_name: &str,
        descriptor: &TypeDescriptorIr,
    ) -> Result<ContractTypeDescriptorProjection, ContractProjectionError> {
        match descriptor {
            TypeDescriptorIr::Record { fields } => {
                let mut projected_fields = BTreeMap::new();
                for (field_name, ty) in fields {
                    projected_fields.insert(
                        field_name.clone(),
                        self.canonical_type_ref(
                            module_path,
                            ty,
                            format!("type {module_path}.{local_name} field {field_name}"),
                        )?,
                    );
                }
                Ok(ContractTypeDescriptorProjection::Record {
                    fields: projected_fields,
                })
            }
            TypeDescriptorIr::Union { variants } => Ok(ContractTypeDescriptorProjection::Union {
                variants: variants
                    .iter()
                    .map(|variant| {
                        self.canonical_type_ref(
                            module_path,
                            variant,
                            format!("type {module_path}.{local_name} union variant"),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeDescriptorIr::Native { symbol } => Ok(ContractTypeDescriptorProjection::Native {
                symbol: symbol.clone(),
            }),
            TypeDescriptorIr::Alias { .. } => {
                Err(ContractProjectionError::UnexpectedAliasDescriptor {
                    module_path: module_path.to_string(),
                    local_name: local_name.to_string(),
                })
            }
        }
    }

    fn implemented_public_interfaces(
        &self,
        binding: &ContractProjectionTypeBinding<'a>,
    ) -> Vec<ImplementedInterface> {
        binding
            .implements()
            .iter()
            .filter_map(|implemented| {
                let source_key = self
                    .index
                    .source_key_for_type_ref(binding.module_path, implemented)?;
                let schema_public_name = self.index.public_symbol_for_source_key(&source_key)?;
                let module_path = source_key.module_path();
                let local_name = source_key.symbol();
                self.index
                    .interface_decl_by_module_local_name(module_path, local_name)?;
                let public_name = self
                    .public_interface_symbol(module_path, local_name)
                    .unwrap_or_else(|| schema_public_name.to_string());
                Some(ImplementedInterface {
                    module_path: module_path.to_string(),
                    local_name: local_name.to_string(),
                    public_name,
                })
            })
            .collect()
    }

    fn public_callable_methods_for_type(&self, module_path: &str, local_name: &str) -> Vec<String> {
        let prefix = format!("{local_name}.");
        self.index
            .public_callable_bindings()
            .filter_map(|source_key| {
                (source_key.source_module == module_path)
                    .then_some(source_key.source_symbol.as_str())
                    .and_then(|symbol| symbol.strip_prefix(&prefix))
                    .map(str::to_string)
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn public_schema_symbol(&self, module_path: &str, local_name: &str) -> Option<String> {
        self.index
            .public_schema_bindings()
            .find(|binding| {
                binding.source_module == module_path && binding.source_symbol == local_name
            })
            .map(|binding| binding.public_path.clone())
    }

    fn public_interface_symbol(&self, module_path: &str, local_name: &str) -> Option<String> {
        self.index
            .interface_decl_by_module_local_name(module_path, local_name)?;
        self.index
            .public_symbol_for_source_key(&ProjectionSourceSymbolKey::new(module_path, local_name))
    }

    fn extend_projected_type_closure(&mut self) -> Result<(), ContractProjectionError> {
        let mut queued_sources = BTreeSet::new();
        let mut worklist = Vec::new();
        self.enqueue_projected_source_type_keys(&mut queued_sources, &mut worklist);

        while let Some(source) = worklist.pop() {
            self.extend_closure_source_type(&source)?;
            self.enqueue_projected_source_type_keys(&mut queued_sources, &mut worklist);
        }

        Ok(())
    }

    fn enqueue_projected_source_type_keys(
        &self,
        queued_sources: &mut BTreeSet<ProjectionSourceSymbolKey>,
        worklist: &mut Vec<ProjectionSourceSymbolKey>,
    ) {
        for ty in self.projection.types.values() {
            match &ty.descriptor {
                ContractTypeDescriptorProjection::Record { fields } => {
                    for field_ty in fields.values() {
                        enqueue_source_type_keys(field_ty, queued_sources, worklist);
                    }
                }
                ContractTypeDescriptorProjection::Union { variants } => {
                    for variant in variants {
                        enqueue_source_type_keys(variant, queued_sources, worklist);
                    }
                }
                ContractTypeDescriptorProjection::Native { .. } => {}
            }
            for implemented in &ty.implements {
                enqueue_source_type_keys(implemented, queued_sources, worklist);
            }
        }

        for alias in self.projection.aliases.values() {
            enqueue_source_type_keys(&alias.target, queued_sources, worklist);
        }

        for interface in self.projection.interfaces.values() {
            for operation in &interface.operations {
                for param in &operation.params {
                    enqueue_source_type_keys(&param.ty, queued_sources, worklist);
                }
                enqueue_source_type_keys(&operation.return_type, queued_sources, worklist);
                if let Some(implicit_self) = &operation.implicit_self {
                    enqueue_source_type_keys(implicit_self, queued_sources, worklist);
                }
            }
        }
    }

    fn extend_closure_source_type(
        &mut self,
        source: &ProjectionSourceSymbolKey,
    ) -> Result<(), ContractProjectionError> {
        if self.index.public_symbol_for_source_key(source).is_some() {
            return Ok(());
        }
        let Some(binding) = self.index.type_binding_by_source_key(source) else {
            return Ok(());
        };
        let symbol = source_key_symbol(source);
        if self.projection.types.contains_key(&symbol)
            || self.projection.aliases.contains_key(&symbol)
            || self.projection.interfaces.contains_key(&symbol)
        {
            return Ok(());
        }

        match binding.kind {
            ContractTypeKind::Alias => self.extend_closure_alias(&binding, &symbol),
            ContractTypeKind::Interface => self.extend_closure_interface_binding(&binding, &symbol),
            ContractTypeKind::Type => self.extend_closure_schema_type(&binding, &symbol),
        }
    }

    fn extend_closure_schema_type(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        symbol: &str,
    ) -> Result<(), ContractProjectionError> {
        let descriptor = self.project_type_descriptor(
            binding.module_path,
            binding.local_name,
            &binding.type_decl.descriptor,
        )?;
        let implements = binding
            .implements()
            .iter()
            .map(|implemented| {
                self.canonical_type_ref(
                    binding.module_path,
                    implemented,
                    format!(
                        "type {}.{} implements",
                        binding.module_path, binding.local_name
                    ),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.projection.types.insert(
            symbol.to_string(),
            ContractTypeProjection {
                public_name: symbol.to_string(),
                source_module: binding.module_path.to_string(),
                source_name: binding.local_name.to_string(),
                type_params: binding.type_decl.type_params.clone(),
                descriptor,
                discriminator: binding.discriminator().map(str::to_string),
                implements,
            },
        );
        Ok(())
    }

    fn extend_closure_alias(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        symbol: &str,
    ) -> Result<(), ContractProjectionError> {
        let TypeDescriptorIr::Alias { target } = &binding.type_decl.descriptor else {
            return Err(ContractProjectionError::UnexpectedAliasDescriptor {
                module_path: binding.module_path.to_string(),
                local_name: binding.local_name.to_string(),
            });
        };
        let target = self.canonical_type_ref(
            binding.module_path,
            target,
            format!("alias {}.{}", binding.module_path, binding.local_name),
        )?;
        self.projection.aliases.insert(
            symbol.to_string(),
            ContractAliasProjection {
                public_name: symbol.to_string(),
                source_module: binding.module_path.to_string(),
                source_name: binding.local_name.to_string(),
                type_params: binding.type_decl.type_params.clone(),
                target,
                discriminator: binding.type_decl.discriminator.clone(),
                transparent: binding.transparent_alias,
            },
        );
        Ok(())
    }

    fn extend_closure_interface_binding(
        &mut self,
        binding: &ContractProjectionTypeBinding<'a>,
        symbol: &str,
    ) -> Result<(), ContractProjectionError> {
        let Some(interface) = self
            .index
            .interface_decl_by_module_local_name(binding.module_path, binding.local_name)
        else {
            return Ok(());
        };
        let projected =
            self.project_interface(binding.module_path, binding.local_name, symbol, interface)?;
        self.projection
            .interfaces
            .insert(symbol.to_string(), projected);
        Ok(())
    }

    fn canonical_type_ref(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        context: String,
    ) -> Result<ContractTypeKey, ContractProjectionError> {
        self.index
            .canonical_type_ref(module_path, ty)
            .map_err(|source| ContractProjectionError::TypeCanonicalization { context, source })
    }

    fn validate_public_contract_projection(&self) -> Result<(), ContractProjectionError> {
        for ty in self.projection.types.values() {
            match &ty.descriptor {
                ContractTypeDescriptorProjection::Record { fields } => {
                    for (field_name, field_ty) in fields {
                        validate_public_contract_type_key(
                            field_ty,
                            format!("type {} field {field_name}", ty.public_name),
                        )?;
                    }
                }
                ContractTypeDescriptorProjection::Union { variants } => {
                    for (index, variant) in variants.iter().enumerate() {
                        validate_public_contract_type_key(
                            variant,
                            format!("type {} union variant {index}", ty.public_name),
                        )?;
                    }
                }
                ContractTypeDescriptorProjection::Native { .. } => {}
            }
        }

        for alias in self.projection.aliases.values() {
            validate_public_contract_type_key(
                &alias.target,
                format!("alias {} target", alias.public_name),
            )?;
        }

        for interface in self.projection.interfaces.values() {
            for operation in &interface.operations {
                let operation_context = format!(
                    "interface {} operation {}",
                    interface.public_name, operation.name
                );
                for param in &operation.params {
                    validate_public_contract_type_key(
                        &param.ty,
                        format!("{operation_context} param {}", param.name),
                    )?;
                }
                validate_public_contract_type_key(
                    &operation.return_type,
                    format!("{operation_context} return"),
                )?;
                if let Some(implicit_self) = &operation.implicit_self {
                    validate_public_contract_type_key(
                        implicit_self,
                        format!("{operation_context} self"),
                    )?;
                }
            }
        }

        Ok(())
    }

    fn ensure_interface_binding_available(
        &self,
        interface_public_name: &str,
        implementation: &ContractProjectionTypeBinding<'a>,
    ) -> Result<(), ContractProjectionError> {
        let Some(existing) = self.projection.api_bindings.get(interface_public_name) else {
            return Ok(());
        };

        let duplicate_source = source_symbol(implementation.module_path, implementation.local_name);
        let existing_source = source_symbol(&existing.source_module, &existing.source_symbol);
        if existing_source == duplicate_source {
            return Ok(());
        }

        Err(ContractProjectionError::DuplicateInterfaceBinding {
            interface_public_name: interface_public_name.to_string(),
            existing_implementation_source: existing_source,
            duplicate_implementation_source: duplicate_source,
        })
    }

    fn ensure_public_symbol_available(
        &self,
        public_name: &str,
        module_path: &str,
        local_name: &str,
        duplicate_kind: ContractProjectionSymbolKind,
    ) -> Result<(), ContractProjectionError> {
        if !self.projection.types.contains_key(public_name)
            && !self.projection.aliases.contains_key(public_name)
            && !self.projection.interfaces.contains_key(public_name)
        {
            return Ok(());
        }

        let duplicate_source = source_symbol(module_path, local_name);
        let existing_source = self
            .projection
            .public_symbols_by_source
            .iter()
            .find_map(|(source, projected_public_name)| {
                (projected_public_name == public_name).then(|| source.clone())
            })
            .unwrap_or_else(|| public_name.to_string());
        if existing_source == duplicate_source {
            return Ok(());
        }

        Err(ContractProjectionError::DuplicatePublicSymbol {
            public_name: public_name.to_string(),
            existing_source,
            duplicate_source,
            duplicate_kind,
        })
    }

    fn record_type_symbol(&mut self, public_name: &str, module_path: &str, local_name: &str) {
        self.projection
            .type_symbols
            .insert(public_name.to_string(), public_name.to_string());
        self.projection.public_symbols_by_source.insert(
            source_symbol(module_path, local_name),
            public_name.to_string(),
        );
    }

    fn record_interface_symbol(&mut self, public_name: &str, module_path: &str, local_name: &str) {
        self.projection
            .interface_symbols
            .insert(public_name.to_string(), public_name.to_string());
        self.projection.interface_source_symbols.insert(
            public_name.to_string(),
            source_symbol(module_path, local_name),
        );
        self.projection.public_symbols_by_source.insert(
            source_symbol(module_path, local_name),
            public_name.to_string(),
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImplementedInterface {
    module_path: String,
    local_name: String,
    public_name: String,
}

fn source_symbol(module_path: &str, local_name: &str) -> String {
    format!("{module_path}.{local_name}")
}

fn source_key_symbol(source: &ProjectionSourceSymbolKey) -> String {
    if source.module_path().is_empty() {
        source.symbol().to_string()
    } else {
        projection_source_symbol_text(source)
    }
}

fn enqueue_source_type_keys(
    key: &ContractTypeKey,
    queued_sources: &mut BTreeSet<ProjectionSourceSymbolKey>,
    worklist: &mut Vec<ProjectionSourceSymbolKey>,
) {
    match key {
        ContractTypeKey::Builtin { args, .. } => {
            for arg in args {
                enqueue_source_type_keys(arg, queued_sources, worklist);
            }
        }
        ContractTypeKey::Named(ContractNamedTypeKey::Source { source }) => {
            if queued_sources.insert(source.clone()) {
                worklist.push(source.clone());
            }
        }
        ContractTypeKey::Named(ContractNamedTypeKey::Public { .. })
        | ContractTypeKey::PackageSymbol { .. }
        | ContractTypeKey::DbObjectSymbol { .. }
        | ContractTypeKey::Literal(_)
        | ContractTypeKey::TypeParam { .. } => {}
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } => {
            enqueue_source_type_keys(interface, queued_sources, worklist);
            for arg in canonical_type_args {
                enqueue_source_type_keys(arg, queued_sources, worklist);
            }
        }
        ContractTypeKey::Record { fields } => {
            for field_ty in fields.values() {
                enqueue_source_type_keys(field_ty, queued_sources, worklist);
            }
        }
        ContractTypeKey::Union { items } => {
            for item in items {
                enqueue_source_type_keys(item, queued_sources, worklist);
            }
        }
        ContractTypeKey::Nullable { inner } => {
            enqueue_source_type_keys(inner, queued_sources, worklist);
        }
        ContractTypeKey::Function {
            params,
            return_type,
        } => {
            for param in params {
                enqueue_source_type_keys(&param.ty, queued_sources, worklist);
            }
            enqueue_source_type_keys(return_type, queued_sources, worklist);
        }
    }
}

fn validate_public_contract_type_key(
    key: &ContractTypeKey,
    context: String,
) -> Result<(), ContractProjectionError> {
    match key {
        ContractTypeKey::Builtin { name, args } => {
            for (index, arg) in args.iter().enumerate() {
                validate_public_contract_type_key(
                    arg,
                    format!("{context} builtin {name} arg {index}"),
                )?;
            }
            Ok(())
        }
        ContractTypeKey::Named(ContractNamedTypeKey::Public { .. }) => Ok(()),
        ContractTypeKey::Named(ContractNamedTypeKey::Source { .. }) => Ok(()),
        ContractTypeKey::PackageSymbol { .. }
        | ContractTypeKey::DbObjectSymbol { .. }
        | ContractTypeKey::Literal(_)
        | ContractTypeKey::TypeParam { .. } => Ok(()),
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } => {
            validate_public_contract_type_key(interface, format!("{context} any interface"))?;
            for (index, arg) in canonical_type_args.iter().enumerate() {
                validate_public_contract_type_key(
                    arg,
                    format!("{context} any interface type arg {index}"),
                )?;
            }
            Ok(())
        }
        ContractTypeKey::Record { fields } => {
            for (field_name, field_ty) in fields {
                validate_public_contract_type_key(
                    field_ty,
                    format!("{context} record field {field_name}"),
                )?;
            }
            Ok(())
        }
        ContractTypeKey::Union { items } => {
            for (index, item) in items.iter().enumerate() {
                validate_public_contract_type_key(item, format!("{context} union item {index}"))?;
            }
            Ok(())
        }
        ContractTypeKey::Nullable { inner } => {
            validate_public_contract_type_key(inner, format!("{context} nullable inner"))
        }
        ContractTypeKey::Function {
            params,
            return_type,
        } => {
            for param in params {
                validate_public_contract_type_key(
                    &param.ty,
                    format!("{context} function param {}", param.name),
                )?;
            }
            validate_public_contract_type_key(return_type, format!("{context} function return"))
        }
    }
}

fn is_compiler_owned_boundary_type_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "ConnectionMessage"
            | "TextConnectionMessage"
            | "BinaryConnectionMessage"
            | "std.websocket.ConnectionMessage"
            | "std.websocket.TextConnectionMessage"
            | "std.websocket.BinaryConnectionMessage"
    )
}
