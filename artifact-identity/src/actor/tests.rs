use skiff_artifact_model::{
    executable::{
        CallIr, CallTargetIr, ExecutableBody, ExecutableIr, ExecutableKind, ExprIr, SlotLayout,
    },
    file_ir::ConstIr,
    types::{LiteralIr, TypeDeclIr, TypeDescriptorIr},
    ActorCreateSignatureIr, ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr,
    ActorImplementationIdentity, ActorPublicMethodIr, FileIrUnit, FunctionTypeParamIr, TypeRefIr,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};

use super::*;

fn abi() -> ActorAbiInput {
    ActorAbiInput {
        actor_name: "DocHub".to_string(),
        actor_id_type: TypeRefIr::builtin("number"),
        key_field: "nextSeq".to_string(),
        fields: vec![ActorFieldIr {
            name: "nextSeq".to_string(),
            ty: TypeRefIr::builtin("number"),
            encoding: ActorFieldEncodingIr::CanonicalValueV1,
        }],
        create: None,
        public_methods: Vec::new(),
        actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
    }
}

#[test]
fn actor_abi_identity_covers_id_fields_and_runtime_version() {
    let base = actor_abi_identity(&abi()).unwrap();
    let mut changed_id = abi();
    changed_id.actor_id_type = TypeRefIr::builtin("integer");
    assert_ne!(base, actor_abi_identity(&changed_id).unwrap());

    let mut changed_field = abi();
    changed_field.fields[0].ty = TypeRefIr::builtin("integer");
    assert_ne!(base, actor_abi_identity(&changed_field).unwrap());

    let mut changed_runtime = abi();
    changed_runtime.actor_runtime_abi_version = "skiff-actor-runtime-abi-v2".to_string();
    assert_ne!(base, actor_abi_identity(&changed_runtime).unwrap());

    let mut changed_create = abi();
    changed_create.create = Some(ActorCreateSignatureIr {
        parameters: vec![FunctionTypeParamIr {
            name: "initialNextSeq".to_string(),
            ty: TypeRefIr::builtin("number"),
        }],
    });
    assert_ne!(base, actor_abi_identity(&changed_create).unwrap());

    let mut changed_methods = abi();
    changed_methods.public_methods.push(ActorPublicMethodIr {
        method_identity: actor_method_identity("docs", "DocHub", "append").unwrap(),
        name: "append".to_string(),
        parameters: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        may_suspend: false,
    });
    let methods_identity = actor_abi_identity(&changed_methods).unwrap();
    assert_ne!(base, methods_identity);

    let mut changed_parameter = changed_methods.clone();
    changed_parameter.public_methods[0].parameters.push(
        skiff_artifact_model::FunctionTypeParamIr {
            name: "value".to_string(),
            ty: TypeRefIr::builtin("string"),
        },
    );
    assert_ne!(
        methods_identity,
        actor_abi_identity(&changed_parameter).unwrap()
    );

    let mut changed_return = changed_methods.clone();
    changed_return.public_methods[0].return_type = TypeRefIr::builtin("string");
    assert_ne!(
        methods_identity,
        actor_abi_identity(&changed_return).unwrap()
    );

    let mut changed_suspend = changed_methods;
    changed_suspend.public_methods[0].may_suspend = true;
    assert_ne!(
        methods_identity,
        actor_abi_identity(&changed_suspend).unwrap()
    );

    assert!(base.as_str().starts_with(ACTOR_ABI_IDENTITY_PREFIX));
}

fn executable(symbol: &str, callee: Option<u32>) -> ExecutableIr {
    ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody {
                expressions: callee
                    .map(|executable_index| ExprIr::Call {
                        call: CallIr {
                            target: CallTargetIr::LocalExecutable { executable_index },
                            concrete_receiver: None,
                            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
                            },
                            args: Vec::new(),
                            inout_args: Vec::new(),
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::new(),
                        },
                    })
                    .into_iter()
                    .collect(),
                ..ExecutableBody::default()
            },
            expression_types: Vec::new(),
            statement_spans: Vec::new(),
            source_span: None,
        }
}

fn actor_unit() -> FileIrUnit {
    let mut unit = FileIrUnit::empty("docs", "source");
    let method_identity = actor_method_identity("docs", "DocHub", "append").unwrap();
    let mut actor_abi = abi();
    actor_abi.public_methods.push(ActorPublicMethodIr {
        method_identity: method_identity.clone(),
        name: "append".to_string(),
        parameters: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        may_suspend: false,
    });
    let actor_abi_identity = actor_abi_identity(&actor_abi).unwrap();
    unit.actor_declarations.push(ActorDeclarationIr {
        actor_abi_identity,
        actor_implementation_identity: ActorImplementationIdentity::new("pending"),
        abi: actor_abi,
        method_implementations: BTreeMap::from([(method_identity, 0)]),
        create_implementation: None,
    });
    unit.executables = vec![
        executable("DocHub.append", Some(1)),
        executable("reachableHelper", Some(0)),
        executable("unrelated", None),
    ];
    unit.executables[0].return_type = TypeRefIr::LocalType { type_index: 0 };
    unit.executables[0]
        .body
        .expressions
        .push(ExprIr::LoadConst { const_index: 0 });
    unit.constants = vec![
        ConstIr {
            name: "USED".to_string(),
            ty: TypeRefIr::builtin("string"),
            body: ExecutableBody {
                expressions: vec![ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "used".to_string(),
                    },
                }],
                ..ExecutableBody::default()
            },
            source_span: None,
        },
        ConstIr {
            name: "UNUSED".to_string(),
            ty: TypeRefIr::builtin("string"),
            body: ExecutableBody::default(),
            source_span: None,
        },
    ];
    unit.type_table = vec![
        TypeDeclIr {
            name: "Result".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([("value".to_string(), TypeRefIr::builtin("string"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
        TypeDeclIr {
            name: "Unused".to_string(),
            descriptor: TypeDescriptorIr::Alias {
                target: TypeRefIr::builtin("string"),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    ];
    unit
}

#[test]
fn actor_implementation_identity_hashes_reachable_scc_but_not_unrelated_code() {
    let base = actor_unit();
    let identity = actor_implementation_identity(&[base.clone()], "docs", "DocHub").unwrap();

    let mut reachable_changed = base.clone();
    reachable_changed.executables[1].may_suspend = true;
    assert_ne!(
        identity,
        actor_implementation_identity(&[reachable_changed], "docs", "DocHub").unwrap()
    );

    let mut unrelated_changed = base.clone();
    unrelated_changed.executables[2].may_suspend = true;
    assert_eq!(
        identity,
        actor_implementation_identity(&[unrelated_changed], "docs", "DocHub").unwrap()
    );
}

#[test]
fn actor_implementation_identity_normalizes_executable_indices() {
    let base = actor_unit();
    let identity = actor_implementation_identity(&[base.clone()], "docs", "DocHub").unwrap();

    let mut reordered = base;
    reordered.executables.swap(0, 1);
    reordered.actor_declarations[0]
        .method_implementations
        .values_mut()
        .for_each(|index| *index = 1);
    let ExprIr::Call { call } = &mut reordered.executables[0].body.expressions[0] else {
        panic!("helper call")
    };
    call.target = CallTargetIr::LocalExecutable {
        executable_index: 1,
    };
    let ExprIr::Call { call } = &mut reordered.executables[1].body.expressions[0] else {
        panic!("method call")
    };
    call.target = CallTargetIr::LocalExecutable {
        executable_index: 0,
    };
    assert_eq!(
        identity,
        actor_implementation_identity(&[reordered], "docs", "DocHub").unwrap()
    );
}

#[test]
fn actor_implementation_identity_tracks_only_reachable_constants_and_types() {
    let base = actor_unit();
    let identity = actor_implementation_identity(&[base.clone()], "docs", "DocHub").unwrap();

    let mut reachable_const = base.clone();
    reachable_const.constants[0].body.expressions[0] = ExprIr::Literal {
        value: LiteralIr::String {
            value: "changed".to_string(),
        },
    };
    assert_ne!(
        identity,
        actor_implementation_identity(&[reachable_const], "docs", "DocHub").unwrap()
    );

    let mut unreachable = base.clone();
    unreachable.constants[1].ty = TypeRefIr::builtin("number");
    unreachable.type_table[1].descriptor = TypeDescriptorIr::Alias {
        target: TypeRefIr::builtin("number"),
    };
    assert_eq!(
        identity,
        actor_implementation_identity(&[unreachable], "docs", "DocHub").unwrap()
    );

    let mut reachable_type = base;
    let TypeDescriptorIr::Record { fields } = &mut reachable_type.type_table[0].descriptor else {
        panic!("record")
    };
    fields.insert("extra".to_string(), TypeRefIr::builtin("number"));
    assert_ne!(
        identity,
        actor_implementation_identity(&[reachable_type], "docs", "DocHub").unwrap()
    );
}

#[test]
fn actor_implementation_identity_normalizes_constant_and_type_indices() {
    let base = actor_unit();
    let identity = actor_implementation_identity(&[base.clone()], "docs", "DocHub").unwrap();
    let mut reordered = base;
    reordered.constants.swap(0, 1);
    reordered.type_table.swap(0, 1);
    reordered.executables[0].return_type = TypeRefIr::LocalType { type_index: 1 };
    let load = reordered.executables[0]
        .body
        .expressions
        .iter_mut()
        .find(|expression| matches!(expression, ExprIr::LoadConst { .. }))
        .expect("constant load");
    *load = ExprIr::LoadConst { const_index: 1 };
    assert_eq!(
        identity,
        actor_implementation_identity(&[reordered], "docs", "DocHub").unwrap()
    );
}
