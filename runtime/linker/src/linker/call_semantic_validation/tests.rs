use std::{cell::Cell, collections::BTreeMap};

use skiff_artifact_model::NativeTarget;

use super::*;
use crate::program::{ExprRefIr, FileAddr, FunctionTypeParamIr, InterfaceOperationIr, UnitAddr};

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
        site: skiff_artifact_model::InstructionSourceSite::Synthetic {
            reason:
                skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        },
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
    assert!(
        validate_call_semantics(&FakeDelegate::valid(), "interface ABI", &[], &mut wrong_abi,)
            .is_err()
    );
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
