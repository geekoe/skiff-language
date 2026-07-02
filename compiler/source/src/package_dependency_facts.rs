use skiff_artifact_model::FileIrUnit;

use crate::SourceCompileModel;

#[derive(Debug)]
pub struct SourceCompilePackageFacts<'a> {
    id: &'a str,
    version: &'a str,
    dependencies: Vec<SourceCompilePackageDependencyFact>,
    compile_model: &'a SourceCompileModel,
    file_ir_units: &'a [FileIrUnit],
}

impl<'a> SourceCompilePackageFacts<'a> {
    pub fn new(
        id: &'a str,
        version: &'a str,
        dependencies: Vec<SourceCompilePackageDependencyFact>,
        compile_model: &'a SourceCompileModel,
        file_ir_units: &'a [FileIrUnit],
    ) -> Self {
        Self {
            id,
            version,
            dependencies,
            compile_model,
            file_ir_units,
        }
    }

    pub fn id(&self) -> &str {
        self.id
    }

    pub fn version(&self) -> &str {
        self.version
    }

    pub fn dependencies(&self) -> &[SourceCompilePackageDependencyFact] {
        &self.dependencies
    }

    pub fn compile_model(&self) -> &SourceCompileModel {
        self.compile_model
    }

    pub fn file_ir_units(&self) -> &[FileIrUnit] {
        self.file_ir_units
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCompilePackageDependencyFact {
    pub id: String,
    pub version: String,
    pub alias: Option<String>,
}

impl SourceCompilePackageDependencyFact {
    pub fn effective_alias(&self) -> &str {
        self.alias.as_deref().unwrap_or_else(|| {
            if self.id == crate::shared::id::SKIFF_STD_PUBLICATION_ID {
                "std"
            } else {
                &self.id
            }
        })
    }
}
