use crate::linked::{LinkedExecutable, LinkedTypeRef};

pub fn executable_type_param_names(executable: &LinkedExecutable) -> Vec<String> {
    let mut names = Vec::new();
    for name in &executable.type_params {
        push_unique_type_param(&mut names, name);
    }
    for param in &executable.params {
        collect_type_ref_type_params(&param.ty, &mut names);
    }
    if let Some(ty) = &executable.return_type {
        collect_type_ref_type_params(ty, &mut names);
    }
    if let Some(ty) = &executable.self_type {
        collect_type_ref_type_params(ty, &mut names);
    }
    names
}

fn collect_type_ref_type_params(type_ref: &LinkedTypeRef, names: &mut Vec<String>) {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => push_unique_type_param(names, name),
        LinkedTypeRef::Native { args, .. } => {
            for arg in args {
                collect_type_ref_type_params(arg, names);
            }
        }
        LinkedTypeRef::Record { fields } => {
            for field in fields.values() {
                collect_type_ref_type_params(field, names);
            }
        }
        LinkedTypeRef::Union { items } => {
            for item in items {
                collect_type_ref_type_params(item, names);
            }
        }
        LinkedTypeRef::Nullable { inner } => collect_type_ref_type_params(inner, names),
        LinkedTypeRef::AnyInterface { interface } => {
            for arg in &interface.canonical_type_args {
                collect_type_ref_type_params(arg, names);
            }
        }
        LinkedTypeRef::Function {
            params,
            return_type,
        } => {
            for param in params {
                collect_type_ref_type_params(&param.ty, names);
            }
            collect_type_ref_type_params(return_type, names);
        }
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => {}
    }
}

fn push_unique_type_param(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|item| item == name) {
        names.push(name.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        ExecutableKind, FunctionTypeParamIr, LinkedExecutableBody, LinkedInterfaceInstantiationRef,
        LinkedTypeRef, ParamIr, SlotLayoutIr,
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
                    "c_function".to_string(),
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
}
