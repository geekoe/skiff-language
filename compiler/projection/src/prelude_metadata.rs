use serde::Serialize;
use skiff_artifact_model::TypeDeclIr;

use crate::prelude::PreludeProjection;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreludeMetadata {
    pub identity: String,
    pub schema_identity: String,
    pub types: Vec<String>,
    pub roots: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_declarations: Vec<TypeDeclIr>,
}

pub fn prelude_metadata_json(prelude: &PreludeProjection) -> PreludeMetadata {
    PreludeMetadata {
        identity: prelude.identity().to_string(),
        schema_identity: prelude.schema_identity().to_string(),
        types: prelude.types().to_vec(),
        roots: prelude.roots().to_vec(),
        type_declarations: prelude.type_declarations().to_vec(),
    }
}
