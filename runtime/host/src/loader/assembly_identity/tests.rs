use serde_json::{json, Value};

use super::*;

#[test]
fn service_assembly_hash_input_includes_package_configs() {
    let assembly = assembly_with_package_configs();

    assert_eq!(
        service_assembly_hash_input(&assembly).expect("hash input should build"),
        router_equivalent_hash_input()
    );
}

#[test]
fn service_assembly_hash_input_matches_compiler_service_shape() {
    let assembly = assembly_with_package_configs();
    let input = service_assembly_hash_input(&assembly).expect("hash input should build");

    assert_eq!(
        input.pointer("/service/access"),
        assembly.pointer("/service/access")
    );
    assert_eq!(
        input.pointer("/service/api"),
        assembly.pointer("/service/api")
    );
    assert_eq!(
        input.pointer("/serviceUnit"),
        assembly.pointer("/serviceUnit")
    );
    assert!(input.pointer("/service/assemblyIdentity").is_none());
}

#[test]
fn service_assembly_content_identity_counts_package_configs() {
    let assembly = assembly_with_package_configs();
    let expected_hash =
        value_sha256(&router_equivalent_hash_input()).expect("hash input should hash");
    let identity = format!("{SERVICE_ASSEMBLY_IDENTITY_PREFIX}:sha256:{expected_hash}");

    validate_service_assembly_content_identity(&assembly, &identity)
        .expect("identity should match router-equivalent hash input");
}

#[test]
fn service_assembly_hash_input_ignores_legacy_interfaces_without_api() {
    let mut assembly = assembly_with_package_configs();
    let without_api_input = {
        let service = assembly
            .pointer_mut("/service")
            .and_then(Value::as_object_mut)
            .expect("service should be an object");
        service.remove("api").expect("api should exist");
        service_assembly_hash_input(&assembly).expect("hash input should build")
    };
    assembly
        .pointer_mut("/service")
        .and_then(Value::as_object_mut)
        .expect("service should be an object")
        .insert(
            "interfaces".to_string(),
            json!({
                "entries": [
                    {
                        "module": "legacy.api",
                        "path": ""
                    }
                ]
            }),
        );

    let input = service_assembly_hash_input(&assembly).expect("hash input should build");

    assert_eq!(input, without_api_input);
    assert_eq!(input.pointer("/service/api"), Some(&Value::Null));
    assert!(input.pointer("/service/interfaces").is_none());
}

fn assembly_with_package_configs() -> Value {
    json!({
        "schemaVersion": "skiff-service-assembly-v1",
        "kind": "service",
        "service": {
            "id": "app.example",
            "revisionId": "rev-1",
            "protocolIdentity": "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "assemblyIdentity": "skiff-service-assembly-v1:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            "access": {
                "visibility": "public"
            },
            "http": {
                "response": {
                    "maxBytes": 1024
                }
            },
            "api": {
                "bindings": {
                    "AppApi": {
                        "sourceModule": "api",
                        "sourceSymbol": "AppApi"
                    }
                },
                "interfaces": {
                    "AppApi": {
                        "modulePath": "api",
                        "name": "AppApi"
                    }
                }
            }
        },
        "files": [],
        "preludeIdentity": "skiff-prelude-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "prelude": {
            "identity": "skiff-prelude-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        },
        "packageConfigs": {
            "skiff.run/llm": {
                "packageId": "skiff.run/llm",
                "version": "1.0.0",
                "config": {
                    "dashscope": {
                        "model": "qwen-plus"
                    }
                }
            }
        },
        "configShape": {
            "entries": []
        },
        "configUses": [
            "dashscope.model"
        ],
        "configActivation": {
            "schemaVersion": "skiff-config-activation-v1",
            "hasPaths": [
                "dashscope.model"
            ]
        },
        "configRequirements": {
            "own": [],
            "dependency": [],
            "effective": []
        },
        "db": {
            "collections": []
        },
        "operations": [],
        "gateway": null,
        "timeout": {
            "ms": 5000
        },
        "dependencyLock": [],
        "serviceUnit": {
            "schemaVersion": "skiff-service-unit-v1",
            "unitIdentity": "skiff-service-unit-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "unitHash": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "unitPath": "units/services/app.example.json"
        },
        "sourceMap": {
            "files": []
        }
    })
}

fn router_equivalent_hash_input() -> Value {
    json!({
        "schemaVersion": "skiff-service-assembly-v1",
        "kind": "service",
        "service": {
            "id": "app.example",
            "revisionId": "rev-1",
            "protocolIdentity": "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "access": {
                "visibility": "public"
            },
            "http": {
                "response": {
                    "maxBytes": 1024
                }
            },
            "api": {
                "bindings": {
                    "AppApi": {
                        "sourceModule": "api",
                        "sourceSymbol": "AppApi"
                    }
                },
                "interfaces": {
                    "AppApi": {
                        "modulePath": "api",
                        "name": "AppApi"
                    }
                }
            }
        },
        "files": [],
        "preludeIdentity": "skiff-prelude-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "prelude": {
            "identity": "skiff-prelude-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        },
        "packageConfigs": {
            "skiff.run/llm": {
                "packageId": "skiff.run/llm",
                "version": "1.0.0",
                "config": {
                    "dashscope": {
                        "model": "qwen-plus"
                    }
                }
            }
        },
        "configShape": {
            "entries": []
        },
        "configUses": [
            "dashscope.model"
        ],
        "configActivation": {
            "schemaVersion": "skiff-config-activation-v1",
            "hasPaths": [
                "dashscope.model"
            ]
        },
        "configRequirements": {
            "own": [],
            "dependency": [],
            "effective": []
        },
        "db": {
            "collections": []
        },
        "operations": [],
        "gateway": null,
        "timeout": {
            "ms": 5000
        },
        "dependencyLock": [],
        "serviceUnit": {
            "schemaVersion": "skiff-service-unit-v1",
            "unitIdentity": "skiff-service-unit-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "unitHash": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "unitPath": "units/services/app.example.json"
        },
        "sourceMap": {
            "files": []
        }
    })
}
