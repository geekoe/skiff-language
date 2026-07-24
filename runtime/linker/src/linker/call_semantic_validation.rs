use std::collections::BTreeMap;

use skiff_runtime_native_contract::{
    native_target_name, NativeCallValidation, NativeSignatureRegistry, NativeTypeArgRef,
};

use super::link_diagnostics::{
    canonical_linked_interface_method_abi_id, interface_declaration_abi_id,
    interface_instantiation_symbol, interface_method_call_symbol, linked_type_ref_abi_key,
    package_interface_declaration_abi_id, substitute_interface_method_type,
    unresolved_type_param_name, validate_interface_operation_explicit_self,
};
use crate::{
    program::{
        CallIr, ConstAddr, ExecutableAddr, InterfaceDeclIr, LinkedCallTarget, LinkedFileUnit,
        LinkedFunctionTypeParamIr, LinkedInterfaceInstantiationRef, LinkedTypeRef, PackageRefIr,
        ReceiverCallAbi,
    },
    resolver::{ProgramError, ProgramResult},
};

/// Supplies traversal-specific address and declaration lookup only. Target-kind semantic rules
/// stay in [`validate_call_semantics`] so legacy and assembly linking cannot drift.
pub(crate) trait CallSemanticValidationDelegate {
    fn validate_const_target(&self, context: &str, addr: &ConstAddr) -> ProgramResult<()>;

    fn validate_executable_target(&self, context: &str, addr: &ExecutableAddr)
        -> ProgramResult<()>;

    fn link_interface_declaration(
        &self,
        context: &str,
        interface: &mut LinkedInterfaceInstantiationRef,
    ) -> ProgramResult<InterfaceDeclIr>;
}

/// The single semantic validation entry for every linked call traversal.
pub(crate) fn validate_call_semantics(
    delegate: &impl CallSemanticValidationDelegate,
    context: &str,
    enclosing_type_params: &[String],
    call: &mut CallIr,
) -> ProgramResult<()> {
    match &mut call.target {
        LinkedCallTarget::Native { target } => {
            let type_args = call.type_args.iter().map(|(key, ty)| {
                NativeTypeArgRef::new(
                    key.as_str(),
                    unresolved_type_param_name(ty, Some(enclosing_type_params)),
                )
            });
            match NativeSignatureRegistry::builtins().validate_native_call_artifact(
                target,
                call.args.len(),
                type_args,
            ) {
                NativeCallValidation::Known | NativeCallValidation::External => Ok(()),
                NativeCallValidation::Invalid(message) => Err(ProgramError::InvalidNativeCall {
                    context: context.to_string(),
                    target: native_target_name(target),
                    message,
                }),
            }
        }
        LinkedCallTarget::InterfaceMethod {
            interface,
            method_abi_id,
            slot,
        } => {
            let unresolved_interface = interface.clone();
            let declaration = delegate.link_interface_declaration(context, interface)?;
            validate_interface_method_call_target(
                context,
                &unresolved_interface,
                interface,
                &declaration,
                method_abi_id,
                *slot,
            )
        }
        LinkedCallTarget::LocalConstReceiverExecutable {
            const_addr,
            executable_addr,
            method_abi_id,
            receiver_call_abi,
        } => {
            delegate.validate_const_target(context, const_addr)?;
            delegate.validate_executable_target(context, executable_addr)?;
            validate_local_receiver_call_abi(context, method_abi_id, *receiver_call_abi)
        }
        _ => Ok(()),
    }
}

pub(super) fn validate_local_receiver_call_abi(
    context: &str,
    method_abi_id: &str,
    receiver_call_abi: ReceiverCallAbi,
) -> ProgramResult<()> {
    if method_abi_id.is_empty() {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: method_abi_id.to_string(),
            expected_kind: "non-empty local receiver executable methodAbiId",
        });
    }
    match receiver_call_abi {
        ReceiverCallAbi::ExplicitSelfFirst => Ok(()),
    }
}

pub(crate) fn local_interface_declaration_abi_ids(
    context: &str,
    file: &LinkedFileUnit,
    declaration_name: &str,
) -> ProgramResult<Vec<String>> {
    let mut abi_ids = vec![interface_declaration_abi_id(
        context,
        file,
        declaration_name,
    )?];
    if let Some(declaration) = file.declarations.types.get(declaration_name) {
        let publication_id = linked_type_ref_abi_key(
            context,
            &LinkedTypeRef::PublicationType {
                module_path: file.module_path.clone(),
                type_index: declaration.type_index,
            },
        )?;
        if !abi_ids.contains(&publication_id) {
            abi_ids.push(publication_id);
        }
    }
    Ok(abi_ids)
}

pub(crate) fn package_interface_declaration_id(
    context: &str,
    package: PackageRefIr,
    symbol_path: &str,
) -> ProgramResult<String> {
    package_interface_declaration_abi_id(context, package, symbol_path)
}

fn validate_interface_method_call_target(
    context: &str,
    unresolved_interface: &LinkedInterfaceInstantiationRef,
    interface: &LinkedInterfaceInstantiationRef,
    declaration: &InterfaceDeclIr,
    method_abi_id: &mut String,
    slot: u32,
) -> ProgramResult<()> {
    if method_abi_id.is_empty() {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: interface_method_call_symbol(interface, method_abi_id, slot),
            expected_kind: "non-empty interface method call methodAbiId",
        });
    }
    let expected_slots = interface_method_slot_specs(
        context,
        interface,
        interface,
        declaration,
        None,
        InterfaceSlotSignatureShape::LocalReceiver,
    )?;
    let unresolved_slots = interface_method_slot_specs(
        context,
        interface,
        unresolved_interface,
        declaration,
        None,
        InterfaceSlotSignatureShape::LocalReceiver,
    )?;
    let slot_index = slot as usize;
    let expected =
        expected_slots
            .get(slot_index)
            .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                context: context.to_string(),
                symbol: interface_method_call_symbol(interface, method_abi_id, slot),
                expected_kind: "interface method call target slot from interface declaration",
            })?;
    if let Some(unresolved) = unresolved_slots.get(slot_index) {
        if method_abi_id == &unresolved.method_abi_id {
            method_abi_id.clone_from(&expected.method_abi_id);
        }
    }
    if expected.slot != slot || expected.method_abi_id != *method_abi_id {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: interface_method_call_symbol(interface, method_abi_id, slot),
            expected_kind: "interface method call target matching interface declaration",
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(super) enum InterfaceSlotSignatureShape {
    LocalReceiver,
    RemotePublicOperation,
}

#[derive(Debug, Clone)]
pub(super) struct InterfaceMethodSlotSpec {
    pub(super) slot: u32,
    pub(super) method_name: String,
    pub(super) method_abi_id: String,
    pub(super) params: Vec<LinkedFunctionTypeParamIr>,
    pub(super) return_type: LinkedTypeRef,
}

pub(super) fn interface_method_slot_specs(
    context: &str,
    linked_interface: &LinkedInterfaceInstantiationRef,
    method_identity_interface: &LinkedInterfaceInstantiationRef,
    declaration: &InterfaceDeclIr,
    concrete_type: Option<&LinkedTypeRef>,
    signature_shape: InterfaceSlotSignatureShape,
) -> ProgramResult<Vec<InterfaceMethodSlotSpec>> {
    if declaration.type_params.len() != linked_interface.canonical_type_args.len() {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: interface_instantiation_symbol(linked_interface),
            expected_kind: "interface type argument arity matching declaration",
        });
    }
    let substitutions = declaration
        .type_params
        .iter()
        .cloned()
        .zip(linked_interface.canonical_type_args.iter().cloned())
        .collect::<BTreeMap<_, _>>();

    declaration
        .operations
        .iter()
        .enumerate()
        .map(|(slot, operation)| {
            if !operation.type_params.is_empty()
                || operation.is_native
                || operation.is_provider
                || operation.is_static
            {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!(
                        "{}.{}",
                        interface_instantiation_symbol(linked_interface),
                        operation.name
                    ),
                    expected_kind: "object-safe interface method declaration",
                });
            }
            validate_interface_operation_explicit_self(context, linked_interface, operation)?;
            let params = operation
                .params
                .iter()
                .enumerate()
                .filter_map(|(param_index, param)| {
                    if matches!(
                        signature_shape,
                        InterfaceSlotSignatureShape::RemotePublicOperation
                    ) && param_index == 0
                        && param.name == "self"
                    {
                        return None;
                    }
                    let ty = if param.name == "self" {
                        concrete_type.cloned().unwrap_or_else(|| param.ty.clone())
                    } else {
                        match substitute_interface_method_type(
                            &param.ty,
                            &substitutions,
                            concrete_type,
                        ) {
                            Ok(ty) => ty,
                            Err(error) => return Some(Err(error)),
                        }
                    };
                    Some(Ok(LinkedFunctionTypeParamIr {
                        name: param.name.clone(),
                        ty,
                    }))
                })
                .collect::<ProgramResult<Vec<_>>>()?;
            let return_type = substitute_interface_method_type(
                &operation.return_type,
                &substitutions,
                concrete_type,
            )?;
            Ok(InterfaceMethodSlotSpec {
                slot: slot as u32,
                method_name: operation.name.clone(),
                method_abi_id: canonical_linked_interface_method_abi_id(
                    method_identity_interface,
                    &operation.name,
                ),
                params,
                return_type,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::BTreeMap};

    use skiff_artifact_model::NativeTarget;

    use super::*;
    use crate::program::{
        ExprRefIr, FileAddr, FunctionTypeParamIr, InterfaceOperationIr, UnitAddr,
    };

    struct FakeDelegate {
        declaration: InterfaceDeclIr,
        reject_const: bool,
        reject_executable: bool,
        const_checks: Cell<usize>,
        executable_checks: Cell<usize>,
    }

    impl FakeDelegate {
        fn valid() -> Self {
            Self {
                declaration: InterfaceDeclIr {
                    name: "Reader".to_string(),
                    type_params: Vec::new(),
                    operations: vec![InterfaceOperationIr {
                        name: "read".to_string(),
                        type_params: Vec::new(),
                        params: vec![FunctionTypeParamIr {
                            name: "self".to_string(),
                            ty: LinkedTypeRef::TypeParam {
                                name: "Self".to_string(),
                            },
                        }],
                        return_type: LinkedTypeRef::Native {
                            name: "string".to_string(),
                            args: Vec::new(),
                        },
                        is_native: false,
                        is_provider: false,
                        is_static: false,
                        implicit_self: None,
                    }],
                    source_span: None,
                },
                reject_const: false,
                reject_executable: false,
                const_checks: Cell::new(0),
                executable_checks: Cell::new(0),
            }
        }
    }

    impl CallSemanticValidationDelegate for FakeDelegate {
        fn validate_const_target(&self, context: &str, _addr: &ConstAddr) -> ProgramResult<()> {
            self.const_checks.set(self.const_checks.get() + 1);
            if self.reject_const {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: "const".to_string(),
                    expected_kind: "valid const target",
                });
            }
            Ok(())
        }

        fn validate_executable_target(
            &self,
            context: &str,
            _addr: &ExecutableAddr,
        ) -> ProgramResult<()> {
            self.executable_checks.set(self.executable_checks.get() + 1);
            if self.reject_executable {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: "executable".to_string(),
                    expected_kind: "valid executable target",
                });
            }
            Ok(())
        }

        fn link_interface_declaration(
            &self,
            _context: &str,
            _interface: &mut LinkedInterfaceInstantiationRef,
        ) -> ProgramResult<InterfaceDeclIr> {
            Ok(self.declaration.clone())
        }
    }

    fn call(target: LinkedCallTarget, arg_count: usize) -> CallIr {
        CallIr {
            target,
            args: (0..arg_count)
                .map(|expression| ExprRefIr {
                    expression: expression as u32,
                })
                .collect(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
            actor_metadata: None,
        }
    }

    #[test]
    fn call_semantic_validation_rejects_malformed_native_signature() {
        let mut call = call(
            LinkedCallTarget::Native {
                target: NativeTarget {
                    namespace: "std.http".to_string(),
                    symbol: "json".to_string(),
                    binding_key: Some("std.http.response.json".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            1,
        );

        let error = validate_call_semantics(&FakeDelegate::valid(), "native", &[], &mut call)
            .expect_err("known native arity must fail closed");

        assert!(matches!(error, ProgramError::InvalidNativeCall { .. }));
        assert!(error.to_string().contains("expected 2 args, got 1"));
    }

    #[test]
    fn call_semantic_validation_rejects_interface_slot_and_abi_mismatch() {
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: "interface:reader".to_string(),
            canonical_type_args: Vec::new(),
        };
        let mut wrong_slot = call(
            LinkedCallTarget::InterfaceMethod {
                method_abi_id: canonical_linked_interface_method_abi_id(&interface, "read"),
                interface: interface.clone(),
                slot: 1,
            },
            0,
        );
        assert!(validate_call_semantics(
            &FakeDelegate::valid(),
            "interface slot",
            &[],
            &mut wrong_slot,
        )
        .is_err());

        let mut wrong_abi = call(
            LinkedCallTarget::InterfaceMethod {
                interface,
                method_abi_id: "method:tampered".to_string(),
                slot: 0,
            },
            0,
        );
        assert!(validate_call_semantics(
            &FakeDelegate::valid(),
            "interface ABI",
            &[],
            &mut wrong_abi,
        )
        .is_err());
    }

    #[test]
    fn call_semantic_validation_checks_receiver_targets_before_abi() {
        let mut delegate = FakeDelegate::valid();
        delegate.reject_executable = true;
        let mut call = call(
            LinkedCallTarget::LocalConstReceiverExecutable {
                const_addr: ConstAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::LoadedFileIndex(0),
                    const_index: 0,
                },
                executable_addr: ExecutableAddr::package(0, 0, 0),
                method_abi_id: "method:read".to_string(),
                receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
            },
            0,
        );

        let error = validate_call_semantics(&delegate, "receiver", &[], &mut call)
            .expect_err("malformed receiver target must fail closed");

        assert!(error.to_string().contains("valid executable target"));
        assert_eq!(delegate.const_checks.get(), 1);
        assert_eq!(delegate.executable_checks.get(), 1);
    }

    #[test]
    fn call_semantic_validation_rejects_empty_receiver_method_abi() {
        let mut call = call(
            LinkedCallTarget::LocalConstReceiverExecutable {
                const_addr: ConstAddr {
                    unit: UnitAddr::Package(0),
                    file: FileAddr::LoadedFileIndex(0),
                    const_index: 0,
                },
                executable_addr: ExecutableAddr::package(0, 0, 0),
                method_abi_id: String::new(),
                receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
            },
            0,
        );

        let error = validate_call_semantics(&FakeDelegate::valid(), "receiver ABI", &[], &mut call)
            .expect_err("empty receiver ABI must fail closed");

        assert!(error.to_string().contains("non-empty local receiver"));
    }
}
