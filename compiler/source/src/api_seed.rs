use std::collections::{BTreeMap, BTreeSet};

use crate::shared::publication_error::PublicationError;
use crate::source_graph::CompilerSourceFile;
use compiler_input_model::{PublicationApiSpec, PublicationApiSpecEntry};

use super::api::{
    PublicCallable, PublicInstance, PublicModuleExport, PublicSymbol, PublicType, PublicationApi,
    SourceSymbolKey,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublicationApiSeed {
    pub public_modules: BTreeMap<String, String>,
    pub public_symbols: BTreeMap<String, PublicSymbol>,
    pub public_callables: BTreeMap<String, PublicCallable>,
    pub public_schema_types: BTreeMap<String, PublicType>,
    pub public_instances: BTreeMap<String, PublicInstance>,
    pub module_exports: Vec<PublicModuleExport>,
    pub publication_schema_symbols: BTreeMap<SourceSymbolKey, String>,
    pub publication_callable_symbols: BTreeSet<SourceSymbolKey>,
    pub publication_public_instance_symbols: BTreeSet<SourceSymbolKey>,
}

impl PublicationApiSeed {
    pub fn from_publication_sources_with_resolved_modules<'a>(
        spec: &PublicationApiSpec,
        sources: impl IntoIterator<Item = &'a CompilerSourceFile>,
        source_module_for_entry: impl Fn(&PublicationApiSpecEntry) -> String,
    ) -> Result<Self, PublicationError> {
        let api = PublicationApi::build_from_publication_sources_with_resolved_modules(
            spec,
            sources,
            source_module_for_entry,
        )?;
        Ok(Self::from_publication_api(&api))
    }

    pub fn from_publication_api(api: &PublicationApi) -> Self {
        Self {
            public_modules: api.public_modules.clone(),
            public_symbols: api.public_symbols.clone(),
            public_callables: api.callables.clone(),
            public_schema_types: api.schema_types.clone(),
            public_instances: api.public_instances.clone(),
            module_exports: api.module_exports().to_vec(),
            publication_schema_symbols: api.schema_public_symbols_by_source(),
            publication_callable_symbols: api.callable_source_symbols(),
            publication_public_instance_symbols: api.public_instance_source_symbols(),
        }
    }

    pub fn no_publication_api() -> Self {
        Self::default()
    }
}
