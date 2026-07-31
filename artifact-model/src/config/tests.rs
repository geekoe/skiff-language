use serde_json::json;

use crate::{PackageConfigAccess, PackageConfigRequirement};

use super::{
    config_shape_from_package_requirements, ConfigShape, ConfigShapeEntry, ConfigShapeValueType,
    PackageConfigShapeError, CONFIG_SHAPE_SCHEMA_VERSION,
};

#[test]
fn config_shape_value_type_uses_canonical_wire_strings() {
    let shape = ConfigShape {
        schema_version: CONFIG_SHAPE_SCHEMA_VERSION.to_string(),
        entries: vec![
            ConfigShapeEntry {
                path: "text".to_string(),
                ty: ConfigShapeValueType::String,
                required: true,
            },
            ConfigShapeEntry {
                path: "raw".to_string(),
                ty: ConfigShapeValueType::Json,
                required: false,
            },
            ConfigShapeEntry {
                path: "object".to_string(),
                ty: ConfigShapeValueType::JsonObject,
                required: true,
            },
        ],
    };

    assert_eq!(
        serde_json::to_value(&shape).expect("config shape should serialize"),
        json!({
            "schemaVersion": "skiff-config-shape-v1",
            "entries": [
                { "path": "text", "type": "string", "required": true },
                { "path": "raw", "type": "Json", "required": false },
                { "path": "object", "type": "JsonObject", "required": true }
            ]
        })
    );
}

#[test]
fn package_requirements_produce_a_sorted_typed_config_shape() {
    let shape = config_shape_from_package_requirements(&[
        PackageConfigRequirement {
            path: "service.token".to_string(),
            access: PackageConfigAccess::Required {
                value_type: "string".to_string(),
            },
        },
        PackageConfigRequirement {
            path: "service.timeout".to_string(),
            access: PackageConfigAccess::Optional {
                value_type: "number".to_string(),
            },
        },
        PackageConfigRequirement {
            path: "service.present".to_string(),
            access: PackageConfigAccess::Presence,
        },
    ])
    .expect("valid package requirements");

    assert_eq!(
        shape.entries,
        vec![
            ConfigShapeEntry {
                path: "service.timeout".to_string(),
                ty: ConfigShapeValueType::Number,
                required: false,
            },
            ConfigShapeEntry {
                path: "service.token".to_string(),
                ty: ConfigShapeValueType::String,
                required: true,
            },
        ]
    );
}

#[test]
fn package_requirements_fail_closed_on_invalid_type_and_duplicate_path() {
    let invalid_type = config_shape_from_package_requirements(&[PackageConfigRequirement {
        path: "service.token".to_string(),
        access: PackageConfigAccess::Required {
            value_type: "bytes".to_string(),
        },
    }])
    .expect_err("unsupported value type must fail");
    assert!(matches!(
        invalid_type,
        PackageConfigShapeError::InvalidValueType { ref path, .. }
            if path == "service.token"
    ));

    let duplicate = config_shape_from_package_requirements(&[
        PackageConfigRequirement {
            path: "service.token".to_string(),
            access: PackageConfigAccess::Required {
                value_type: "string".to_string(),
            },
        },
        PackageConfigRequirement {
            path: "service.token".to_string(),
            access: PackageConfigAccess::Presence,
        },
    ])
    .expect_err("duplicate path must fail");
    assert_eq!(
        duplicate,
        PackageConfigShapeError::DuplicatePath {
            path: "service.token".to_string()
        }
    );
}
