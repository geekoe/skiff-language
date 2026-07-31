use std::collections::BTreeMap;

use crate::{
    ExecutableKind, FunctionTypeParamIr, LinkedExecutableBody, LinkedInterfaceInstantiationRef,
    LinkedNominalTypeRefBase, LinkedTypeRef, ParamIr, SlotLayoutIr,
};

use super::*;

#[test]
fn executable_type_param_names_includes_explicit_params_without_signature_mentions() {
    let executable = executable(Vec::from(["T".to_string(), "U".to_string()]));

    assert_eq!(
        executable_type_param_names(&executable),
        vec!["T".to_string(), "U".to_string()]
    );
}

#[test]
fn executable_type_param_names_collects_nested_structural_return_types() {
    let mut executable = executable(Vec::new());
    executable.return_type = Some(LinkedTypeRef::Record {
        fields: BTreeMap::from([
            (
                "a_native".to_string(),
                LinkedTypeRef::Native {
                    name: "Array".to_string(),
                    args: vec![type_param("TNative")],
                },
            ),
            (
                "b_union".to_string(),
                LinkedTypeRef::Union {
                    items: vec![
                        LinkedTypeRef::Nullable {
                            inner: Box::new(type_param("TNullable")),
                        },
                        LinkedTypeRef::AnyInterface {
                            interface: LinkedInterfaceInstantiationRef {
                                interface_abi_id: "iface:test".to_string(),
                                canonical_type_args: vec![type_param("TInterface")],
                            },
                        },
                    ],
                },
            ),
            (
                "c_applied".to_string(),
                LinkedTypeRef::AppliedNominal {
                    base: LinkedNominalTypeRefBase::LocalType { type_index: 0 },
                    arguments: vec![type_param("TApplied")],
                },
            ),
            (
                "d_function".to_string(),
                LinkedTypeRef::Function {
                    params: vec![FunctionTypeParamIr {
                        name: "value".to_string(),
                        ty: type_param("TFunctionParam"),
                    }],
                    return_type: Box::new(type_param("TFunctionReturn")),
                },
            ),
        ]),
    });

    assert_eq!(
        executable_type_param_names(&executable),
        vec![
            "TNative".to_string(),
            "TNullable".to_string(),
            "TInterface".to_string(),
            "TApplied".to_string(),
            "TFunctionParam".to_string(),
            "TFunctionReturn".to_string(),
        ]
    );
}

#[test]
fn executable_type_param_names_deduplicates_in_first_seen_order() {
    let mut executable = executable(Vec::from([
        "T".to_string(),
        "U".to_string(),
        "T".to_string(),
    ]));
    executable.params = vec![
        ParamIr {
            name: "first".to_string(),
            slot: 0,
            ty: type_param("U"),
        },
        ParamIr {
            name: "second".to_string(),
            slot: 1,
            ty: type_param("V"),
        },
    ];
    executable.return_type = Some(LinkedTypeRef::Record {
        fields: BTreeMap::from([
            ("a_repeat".to_string(), type_param("T")),
            ("b_new".to_string(), type_param("W")),
        ]),
    });
    executable.self_type = Some(type_param("W"));

    assert_eq!(
        executable_type_param_names(&executable),
        vec![
            "T".to_string(),
            "U".to_string(),
            "V".to_string(),
            "W".to_string(),
        ]
    );
}

fn executable(type_params: Vec<String>) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.test".to_string(),
        type_params,
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: LinkedExecutableBody::default(),
    }
}

fn type_param(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::TypeParam {
        name: name.to_string(),
    }
}
