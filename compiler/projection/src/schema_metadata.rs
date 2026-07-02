use crate::{prelude::PreludeProjection, runtime_manifest_model::JsonSchema};
use skiff_compiler_core::prelude_registry::PRELUDE_REGISTRY_ID;

pub fn std_schema_ref(prelude: &PreludeProjection, name: &str) -> JsonSchema {
    let symbol = prelude.type_symbol(name);
    if is_prelude_schema_symbol(name) {
        return JsonSchema::reference(&symbol)
            .with_x_skiff_symbol(symbol)
            .with_x_skiff_prelude_identity(prelude.identity());
    }
    JsonSchema::reference(&symbol)
        .with_x_skiff_package(PRELUDE_REGISTRY_ID.to_string())
        .with_x_skiff_symbol(symbol)
        .with_x_skiff_schema_identity(prelude.schema_identity())
}

pub fn with_std_symbol(prelude: &PreludeProjection, schema: JsonSchema, name: &str) -> JsonSchema {
    if is_prelude_schema_symbol(name) {
        return schema
            .with_x_skiff_symbol(prelude.type_symbol(name))
            .with_x_skiff_prelude_identity(prelude.identity());
    }
    schema
        .with_x_skiff_package(PRELUDE_REGISTRY_ID.to_string())
        .with_x_skiff_symbol(prelude.type_symbol(name))
        .with_x_skiff_schema_identity(prelude.schema_identity())
}

fn is_prelude_schema_symbol(name: &str) -> bool {
    matches!(name, "Json" | "JsonObject")
}
