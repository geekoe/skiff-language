//! Eval fixture linked-program surface.
//!
//! Production eval imports linked-program DTOs and activation facts directly from
//! their contract crates. This module remains only as a compatibility namespace
//! for tests and `test-support` fixture builders.

#![cfg(any(test, feature = "test-support"))]
#![allow(unused_imports)]

pub mod types {
    pub use skiff_runtime_linked_program::types::*;
}

pub use crate::test_support::RuntimeProgram;
pub use skiff_runtime_activation::RuntimeActivation;
pub use skiff_runtime_linked_program::LinkedProgramImage as EvalProgramImage;
pub use skiff_runtime_linked_program::{
    anonymous_type_decl, type_descriptor_to_value, type_ref_to_value, AssignTargetIr, BinaryOpIr,
    BlockIr, CallIr, ConstAddr, ConstIr, DbBodyIr, DbChangeIr, DbChangeOpIr, DbIndexDirectionIr,
    DbLeaseClaimIr, DbLeaseReadIr, DbOpKindIr, DbOperationIr, DbOrderIr, DbPredicateCompareOpIr,
    DbPredicateIr, DbProjectionIr, DbQueryIr, DbSelectorIr, DbTargetIr, DbTransactionIr,
    DbTransactionModeIr, ExecutableAddr, ExecutableKind, ExprRefIr, FieldPathIr, FileAddr,
    FileDeclarations, FileLinkTargets, FunctionTypeParamIr, GatewayConfig, LinkOverlay,
    LinkedBoxSourceIr, LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedExprIr,
    LinkedFileUnit, LinkedInterfaceInstantiationRef, LinkedInterfaceMethodSlotPlanIr,
    LinkedInterfaceMethodTablePlanIr, LinkedRemoteOperationSlotPlanIr,
    LinkedRemoteOperationTablePlanIr, LinkedStmtIr, LinkedTypeDescriptor, LinkedTypeRef, LiteralIr,
    MetadataValue, NativeTarget, PackageRefIr, PackageSymbolRef, PackageUnit, ParamIr, PatternIr,
    ReceiverCallAbi, ResolvedSymbol, RuntimeTypeContext, ServiceDependencyConstraint,
    ServiceDependencySymbolRef, ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, StmtRefIr,
    TypeAddr, TypeDeclIr, UnaryOpIr, UnitAddr,
};
