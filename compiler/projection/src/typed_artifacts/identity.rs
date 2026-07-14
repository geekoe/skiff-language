use std::collections::BTreeMap;

use skiff_artifact_model::{
    CanonicalPublicCallableSignature, FileIrUnit, InterfaceInstantiationRef, MetadataValue,
    PackageUnit, PublicationAbiUnit, PublicationSchemaType, ServiceUnit,
};

pub fn file_ir_identity(unit: &FileIrUnit) -> String {
    skiff_artifact_identity::file_ir_identity(unit)
        .expect("projected File IR must serialize for canonical artifact identity")
}

pub fn assign_file_ir_identity(unit: &mut FileIrUnit) -> String {
    skiff_artifact_identity::assign_file_ir_identity(unit)
        .expect("projected File IR must serialize for canonical artifact identity")
}

pub fn service_unit_hash(unit: &ServiceUnit) -> String {
    skiff_artifact_identity::service_unit_hash(unit)
        .expect("projected service unit must serialize for canonical artifact identity")
}

pub fn service_unit_identity(unit: &ServiceUnit) -> String {
    skiff_artifact_identity::service_unit_identity(unit)
        .expect("projected service unit must serialize for canonical artifact identity")
}

pub fn package_build_hash(unit: &PackageUnit) -> String {
    skiff_artifact_identity::package_build_hash(unit)
        .expect("projected package unit must serialize for canonical artifact identity")
}

pub fn package_build_identity(unit: &PackageUnit) -> String {
    skiff_artifact_identity::package_build_identity(unit)
        .expect("projected package unit must serialize for canonical artifact identity")
}

pub fn package_abi_hash(unit: &PackageUnit) -> String {
    skiff_artifact_identity::package_abi_hash(unit)
        .expect("projected package ABI must serialize for canonical artifact identity")
}

pub fn package_abi_identity(unit: &PackageUnit) -> String {
    skiff_artifact_identity::package_abi_identity(unit)
        .expect("projected package ABI must serialize for canonical artifact identity")
}

pub fn publication_abi_hash(unit: &PublicationAbiUnit) -> String {
    skiff_artifact_identity::publication_abi_hash(unit)
        .expect("projected publication ABI must serialize for canonical artifact identity")
}

pub fn publication_abi_identity(unit: &PublicationAbiUnit) -> String {
    skiff_artifact_identity::publication_abi_identity(unit)
        .expect("projected publication ABI must serialize for canonical artifact identity")
}

pub fn public_function_operation_abi_id(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> String {
    skiff_artifact_identity::public_function_operation_abi_id(
        public_path,
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    )
    .expect("public function operation ABI must serialize for canonical artifact identity")
}

pub fn public_instance_method_operation_abi_id(
    public_path: &str,
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_abi_id: &str,
    public_signature: &CanonicalPublicCallableSignature,
    schema_closure: &[PublicationSchemaType],
    stream_effect_throw_config: &BTreeMap<String, MetadataValue>,
) -> String {
    skiff_artifact_identity::public_instance_method_operation_abi_id(
        public_path,
        public_instance_key,
        interface,
        method_abi_id,
        public_signature,
        schema_closure,
        stream_effect_throw_config,
    )
    .expect("public instance operation ABI must serialize for canonical artifact identity")
}

pub fn assign_publication_abi_identity(unit: &mut PublicationAbiUnit) -> String {
    skiff_artifact_identity::assign_publication_abi_identity(unit)
        .expect("projected publication ABI must serialize for canonical artifact identity")
}

pub fn assign_package_unit_identities(unit: &mut PackageUnit) -> (String, String) {
    skiff_artifact_identity::assign_package_unit_identities(unit)
        .expect("projected package unit must serialize for canonical artifact identity")
}
