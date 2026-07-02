use std::collections::BTreeMap;

use skiff_artifact_model::{
    CanonicalPublicCallableSignature, InterfaceInstantiationRef, MetadataValue,
    PublicationSchemaType,
};

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
    .expect("public function operation ABI id must be derived by skiff_artifact_identity")
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
    .expect("public instance method operation ABI id must be derived by skiff_artifact_identity")
}
