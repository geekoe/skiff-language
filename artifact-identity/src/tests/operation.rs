use super::*;

#[test]
fn operation_abi_helpers_share_the_canonical_operation_input() {
    let public_signature = CanonicalPublicCallableSignature {
        params: Vec::new(),
        return_type: TypeRefIr::native("string"),
        may_suspend: false,
    };
    let stream_effect_throw_config = BTreeMap::new();
    let input = OperationAbiIdentityInput {
        kind: PublicationOperationKind::PublicFunction,
        public_path: "run",
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        public_signature: &public_signature,
        schema_closure: &[],
        stream_effect_throw_config: &stream_effect_throw_config,
    };

    let identity = operation_abi_identity(&input).expect("operation ABI identity");
    assert!(identity.starts_with(OPERATION_ABI_IDENTITY_PREFIX));
    assert_eq!(
        identity,
        public_function_operation_abi_id("run", &public_signature, &[], &BTreeMap::new())
            .expect("public function ABI id")
    );

    let changed_signature = CanonicalPublicCallableSignature {
        params: Vec::new(),
        return_type: TypeRefIr::native("number"),
        may_suspend: false,
    };
    assert_ne!(
        identity,
        public_function_operation_abi_id("run", &changed_signature, &[], &BTreeMap::new())
            .expect("changed public function ABI id")
    );
}
