use crate::error::{CompileError, Result};
use skiff_compiler_core::package_publication_abi as core_publication_abi;

pub use skiff_artifact_model::{
    CanonicalPublicCallableSignature, ExecutableSignatureIr, InterfaceInstantiationRef,
    OperationAbiRef, PublicInstanceOperation, PublicationAbiUnit, PublicationOperationAbi,
    PublicationPublicInstanceExport, SourceCallMethodIndexEntry, SourceCallOperationIndexEntry,
};
pub use skiff_compiler_core::package_publication_abi::{
    public_instance_operation_method_name, public_instance_operation_ref_method_name,
    public_signature_from_receiver_executable_signature, PackagePublicationAbiBuildError,
    PackagePublicationAbiBuilder, PackagePublicationAbiSurface, PackagePublicationOperation,
    PackagePublicationPublicFunction, PackagePublicationPublicInstance,
};

pub fn push_publication_operation_abi(
    publication_abi: &mut PublicationAbiUnit,
    source_call_path: impl Into<String>,
    operation: OperationAbiRef,
    public_signature: CanonicalPublicCallableSignature,
) -> Result<()> {
    core_publication_abi::push_publication_operation_abi(
        publication_abi,
        source_call_path,
        operation,
        public_signature,
    )
    .map_err(publication_abi_build_error)
}

pub fn publication_public_instance_export<'a>(
    public_instance_key: impl Into<String>,
    operations: impl IntoIterator<Item = (&'a PublicInstanceOperation, OperationAbiRef)>,
    conflict_context: Option<String>,
) -> Result<PublicationPublicInstanceExport> {
    core_publication_abi::publication_public_instance_export(
        public_instance_key,
        operations,
        conflict_context,
    )
    .map_err(publication_abi_build_error)
}

pub(crate) fn publication_abi_build_error(error: PackagePublicationAbiBuildError) -> CompileError {
    CompileError::Semantic(error.to_string())
}
