use std::collections::{BTreeMap, BTreeSet};

use crate::shared::ast::SourceFile;

use crate::shared::publication_error::PublicationError;
use crate::source_graph::CompilerSourceFile;
use compiler_input_model::{
    PublicationApiPublicInstanceEntry, PublicationApiSpec, PublicationApiSpecEntry,
    SourceSymbolSelector,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourceSymbolKey {
    module_path: String,
    symbol: String,
}

impl SourceSymbolKey {
    pub fn new(module_path: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            module_path: module_path.into(),
            symbol: symbol.into(),
        }
    }

    pub fn module_path(&self) -> &str {
        &self.module_path
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn to_source_symbol(&self) -> String {
        format!("{}.{}", self.module_path, self.symbol)
    }
}

impl std::fmt::Display for SourceSymbolKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_source_symbol())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationApi {
    pub public_modules: BTreeMap<String, String>,
    pub public_symbols: BTreeMap<String, PublicSymbol>,
    pub callables: BTreeMap<String, PublicCallable>,
    pub schema_types: BTreeMap<String, PublicType>,
    pub public_instances: BTreeMap<String, PublicInstance>,
    module_exports: Vec<PublicModuleExport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicModuleExport {
    pub public_path: String,
    pub source_module: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicSymbol {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicSymbolKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PublicSymbolKind {
    Type,
    Alias,
    Interface,
    Function,
    Const,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCallable {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicCallableKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCallableKind {
    Function,
    Method,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicType {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub kind: PublicTypeKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInstance {
    pub public_path: String,
    pub source_module: String,
    pub source_symbol: String,
    pub interfaces: Vec<PublicInstanceInterface>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicInstanceInterface {
    pub source_module: String,
    pub source_symbol: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicTypeKind {
    Type,
    Alias,
    Interface,
}

#[derive(Clone, Copy)]
struct PublicationApiSource<'a> {
    module_path: &'a str,
    is_test_file: bool,
    ast: &'a SourceFile,
}

impl<'a> From<&'a CompilerSourceFile> for PublicationApiSource<'a> {
    fn from(source: &'a CompilerSourceFile) -> Self {
        Self {
            module_path: &source.module_path,
            is_test_file: source.is_test_file,
            ast: &source.ast,
        }
    }
}

impl PublicationApi {
    #[cfg(test)]
    pub fn build_from_publication_sources<'a>(
        spec: &PublicationApiSpec,
        sources: impl IntoIterator<Item = &'a CompilerSourceFile>,
    ) -> Result<Self, PublicationError> {
        Self::build_from_publication_sources_with_resolved_modules(spec, sources, |entry| {
            entry.source_module_hint().to_string()
        })
    }

    pub fn build_from_publication_sources_with_resolved_modules<'a>(
        spec: &PublicationApiSpec,
        sources: impl IntoIterator<Item = &'a CompilerSourceFile>,
        source_module_for_entry: impl Fn(&PublicationApiSpecEntry) -> String,
    ) -> Result<Self, PublicationError> {
        Self::build(
            spec,
            sources.into_iter().map(PublicationApiSource::from),
            source_module_for_entry,
        )
    }

    fn build<'a>(
        spec: &PublicationApiSpec,
        sources: impl IntoIterator<Item = PublicationApiSource<'a>>,
        source_module_for_entry: impl Fn(&PublicationApiSpecEntry) -> String,
    ) -> Result<Self, PublicationError> {
        let sources_by_module = sources
            .into_iter()
            .map(|source| (source.module_path, source))
            .collect::<BTreeMap<_, _>>();
        let production_sources_by_module = sources_by_module
            .iter()
            .filter_map(|(module, source)| (!source.is_test_file).then_some((*module, *source)))
            .collect::<BTreeMap<_, _>>();

        // Index every non-test source symbol by (module_path, symbol).
        let symbol_index = build_symbol_index(&production_sources_by_module);
        let test_symbol_index = build_test_symbol_index(&sources_by_module);
        let impl_method_index = build_impl_method_index(&production_sources_by_module);

        let mut api = Self {
            public_modules: BTreeMap::new(),
            public_symbols: BTreeMap::new(),
            callables: BTreeMap::new(),
            schema_types: BTreeMap::new(),
            public_instances: BTreeMap::new(),
            module_exports: Vec::new(),
        };
        let mut duplicates = BTreeSet::new();

        for entry in spec.entries() {
            let resolved_module = source_module_for_entry(entry);
            let source_symbol = entry.source_symbol().to_string();
            let selector_label = entry.source_selector.as_dotted();
            if test_symbol_index.contains(&(resolved_module.clone(), source_symbol.clone())) {
                return Err(PublicationError::ContractValidation {
                    message: format!(
                        "api.yml selector {selector_label} resolves to a test source symbol"
                    ),
                });
            }
            if impl_method_index.contains(&selector_label) {
                return Err(PublicationError::ContractValidation {
                    message: format!(
                        "api.yml selector {selector_label} points to an impl method; publish the receiver type instead"
                    ),
                });
            }
            let Some(declarations) =
                symbol_index.get(&(resolved_module.clone(), source_symbol.clone()))
            else {
                return Err(PublicationError::ContractValidation {
                    message: format!(
                        "api.yml selector {selector_label} not found in publication sources"
                    ),
                });
            };
            let Some(kind) = declarations.unique_kind() else {
                return Err(PublicationError::ContractValidation {
                    message: duplicate_source_symbol_message(entry, declarations),
                });
            };

            let public_path = entry.public_path_string();
            api.insert_resolved_export(
                public_path.clone(),
                &resolved_module,
                &source_symbol,
                kind,
                &mut duplicates,
            );

            // A public type carries its impl methods into the public contract:
            // any method on an impl targeting that type becomes a public
            // callable under `<public_path>.<method>`. Impl methods still
            // cannot be listed as api.yml selectors on their own.
            if kind == PublicSymbolKind::Type {
                if let Some(source) = production_sources_by_module.get(resolved_module.as_str()) {
                    for implementation in &source.ast.impls {
                        let Some(local_target) =
                            local_implementation_target(&implementation.target, &resolved_module)
                        else {
                            continue;
                        };
                        if local_target != source_symbol {
                            continue;
                        }
                        for method in &implementation.methods {
                            let method_public_path = join_public_path(&public_path, &method.name);
                            api.insert_public_callable(
                                method_public_path,
                                &resolved_module,
                                &format!("{local_target}.{}", method.name),
                                PublicCallableKind::Method,
                                &mut duplicates,
                            );
                        }
                    }
                }
            }
        }

        for entry in spec.public_instances() {
            let public_path = entry.public_path_string();
            let const_module = entry.source_module_hint().to_string();
            let const_symbol = entry.source_symbol().to_string();
            validate_public_instance_selector(
                entry,
                "const",
                &entry.const_selector,
                &const_module,
                &const_symbol,
                PublicSymbolKind::Const,
                &symbol_index,
                &test_symbol_index,
                &impl_method_index,
            )?;

            let interfaces = entry
                .interface_selectors
                .iter()
                .map(|selector| {
                    validate_public_instance_selector(
                        entry,
                        "interface",
                        selector,
                        &selector.module_path,
                        &selector.symbol,
                        PublicSymbolKind::Interface,
                        &symbol_index,
                        &test_symbol_index,
                        &impl_method_index,
                    )?;
                    Ok(PublicInstanceInterface {
                        source_module: selector.module_path.clone(),
                        source_symbol: selector.symbol.clone(),
                    })
                })
                .collect::<Result<Vec<_>, PublicationError>>()?;
            api.insert_public_instance(
                public_path,
                &const_module,
                &const_symbol,
                interfaces,
                &mut duplicates,
            );
        }
        api.rebuild_compat_module_exports();

        if !duplicates.is_empty() {
            return Err(PublicationError::ContractValidation {
                message: duplicates
                    .into_iter()
                    .map(|symbol| format!("- duplicate publication api {symbol}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            });
        }

        Ok(api)
    }

    fn rebuild_compat_module_exports(&mut self) {
        self.public_modules.clear();
        self.module_exports.clear();
        let mut exports = BTreeSet::<(String, String)>::new();
        for symbol in self.public_symbols.values() {
            let public_module = public_module_path_from_public_symbol(&symbol.public_path);
            self.public_modules
                .entry(public_module.clone())
                .or_insert_with(|| symbol.source_module.clone());
            exports.insert((public_module, symbol.source_module.clone()));
        }
        for public_instance in self.public_instances.values() {
            let public_module = public_module_path_from_public_symbol(&public_instance.public_path);
            self.public_modules
                .entry(public_module.clone())
                .or_insert_with(|| public_instance.source_module.clone());
            exports.insert((public_module, public_instance.source_module.clone()));
        }
        self.module_exports = exports
            .into_iter()
            .map(|(public_path, source_module)| PublicModuleExport {
                public_path,
                source_module,
            })
            .collect();
    }

    fn insert_resolved_export(
        &mut self,
        public_path: String,
        source_module: &str,
        source_symbol: &str,
        kind: PublicSymbolKind,
        duplicates: &mut BTreeSet<String>,
    ) {
        self.insert_public_symbol(
            public_path.clone(),
            source_module,
            source_symbol,
            kind,
            duplicates,
        );
        match kind {
            PublicSymbolKind::Type => self.insert_public_type(
                public_path,
                source_module,
                source_symbol,
                PublicTypeKind::Type,
                duplicates,
            ),
            PublicSymbolKind::Alias => self.insert_public_type(
                public_path,
                source_module,
                source_symbol,
                PublicTypeKind::Alias,
                duplicates,
            ),
            PublicSymbolKind::Interface => self.insert_public_type(
                public_path,
                source_module,
                source_symbol,
                PublicTypeKind::Interface,
                duplicates,
            ),
            PublicSymbolKind::Function => self.insert_public_callable(
                public_path,
                source_module,
                source_symbol,
                PublicCallableKind::Function,
                duplicates,
            ),
            PublicSymbolKind::Const => {}
        }
    }

    pub fn module_exports(&self) -> &[PublicModuleExport] {
        &self.module_exports
    }

    #[cfg(test)]
    pub fn public_symbol_for_source_key(&self, source_key: &SourceSymbolKey) -> Option<&str> {
        self.public_symbols
            .values()
            .find(|symbol| {
                source_key.module_path() == symbol.source_module
                    && source_key.symbol() == symbol.source_symbol
            })
            .map(|symbol| symbol.public_path.as_str())
    }

    #[cfg(test)]
    pub fn is_public_schema_source_key(&self, source_key: &SourceSymbolKey) -> bool {
        self.schema_types.values().any(|symbol| {
            source_key.module_path() == symbol.source_module
                && source_key.symbol() == symbol.source_symbol
        })
    }

    #[cfg(test)]
    pub fn api_source_modules(&self) -> BTreeSet<String> {
        self.public_modules.values().cloned().collect()
    }

    pub fn schema_public_symbols_by_source(&self) -> BTreeMap<SourceSymbolKey, String> {
        self.schema_types
            .values()
            .map(|symbol| {
                (
                    SourceSymbolKey::new(&symbol.source_module, &symbol.source_symbol),
                    symbol.public_path.clone(),
                )
            })
            .collect()
    }

    pub fn callable_source_symbols(&self) -> BTreeSet<SourceSymbolKey> {
        self.callables
            .values()
            .map(|callable| SourceSymbolKey::new(&callable.source_module, &callable.source_symbol))
            .collect()
    }

    pub fn public_instance_source_symbols(&self) -> BTreeSet<SourceSymbolKey> {
        self.public_instances
            .values()
            .map(|public_instance| {
                SourceSymbolKey::new(
                    &public_instance.source_module,
                    &public_instance.source_symbol,
                )
            })
            .collect()
    }

    fn insert_public_symbol(
        &mut self,
        public_path: String,
        source_module: &str,
        source_symbol: &str,
        kind: PublicSymbolKind,
        duplicates: &mut BTreeSet<String>,
    ) {
        if self
            .public_symbols
            .insert(
                public_path.clone(),
                PublicSymbol {
                    public_path: public_path.clone(),
                    source_module: source_module.to_string(),
                    source_symbol: source_symbol.to_string(),
                    kind,
                },
            )
            .is_some()
        {
            duplicates.insert(format!("symbol {public_path}"));
        }
    }

    fn insert_public_callable(
        &mut self,
        public_path: String,
        source_module: &str,
        source_symbol: &str,
        kind: PublicCallableKind,
        duplicates: &mut BTreeSet<String>,
    ) {
        if self
            .callables
            .insert(
                public_path.clone(),
                PublicCallable {
                    public_path: public_path.clone(),
                    source_module: source_module.to_string(),
                    source_symbol: source_symbol.to_string(),
                    kind,
                },
            )
            .is_some()
        {
            duplicates.insert(format!("callable {public_path}"));
        }
    }

    fn insert_public_type(
        &mut self,
        public_path: String,
        source_module: &str,
        source_symbol: &str,
        kind: PublicTypeKind,
        duplicates: &mut BTreeSet<String>,
    ) {
        if self
            .schema_types
            .insert(
                public_path.clone(),
                PublicType {
                    public_path: public_path.clone(),
                    source_module: source_module.to_string(),
                    source_symbol: source_symbol.to_string(),
                    kind,
                },
            )
            .is_some()
        {
            duplicates.insert(format!("schema type {public_path}"));
        }
    }

    fn insert_public_instance(
        &mut self,
        public_path: String,
        source_module: &str,
        source_symbol: &str,
        interfaces: Vec<PublicInstanceInterface>,
        duplicates: &mut BTreeSet<String>,
    ) {
        if self.public_symbols.contains_key(&public_path)
            || self.callables.contains_key(&public_path)
            || self.schema_types.contains_key(&public_path)
        {
            duplicates.insert(format!("public instance {public_path}"));
            return;
        }
        if self
            .public_instances
            .insert(
                public_path.clone(),
                PublicInstance {
                    public_path: public_path.clone(),
                    source_module: source_module.to_string(),
                    source_symbol: source_symbol.to_string(),
                    interfaces,
                },
            )
            .is_some()
        {
            duplicates.insert(format!("public instance {public_path}"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_public_instance_selector(
    entry: &PublicationApiPublicInstanceEntry,
    label: &str,
    selector: &SourceSymbolSelector,
    resolved_module: &str,
    source_symbol: &str,
    expected_kind: PublicSymbolKind,
    symbol_index: &BTreeMap<(String, String), SourceSymbolDeclarations>,
    test_symbol_index: &BTreeSet<(String, String)>,
    impl_method_index: &BTreeSet<String>,
) -> Result<(), PublicationError> {
    let selector_label = selector.as_dotted();
    if test_symbol_index.contains(&(resolved_module.to_string(), source_symbol.to_string())) {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "api.yml public instance {} {label} selector {selector_label} resolves to a test source symbol",
                entry.public_path_string()
            ),
        });
    }
    if impl_method_index.contains(&selector_label) {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "api.yml public instance {} {label} selector {selector_label} points to an impl method",
                entry.public_path_string()
            ),
        });
    }
    let Some(declarations) =
        symbol_index.get(&(resolved_module.to_string(), source_symbol.to_string()))
    else {
        if expected_kind == PublicSymbolKind::Interface {
            return Ok(());
        }
        return Err(PublicationError::ContractValidation {
            message: format!(
                "api.yml public instance {} {label} selector {selector_label} not found in publication sources",
                entry.public_path_string()
            ),
        });
    };
    let Some(kind) = declarations.unique_kind() else {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "api.yml public instance {} {label} selector {selector_label} resolves to multiple source declarations",
                entry.public_path_string()
            ),
        });
    };
    if kind != expected_kind {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "api.yml public instance {} {label} selector {selector_label} must resolve to a {:?}, got {:?}",
                entry.public_path_string(),
                expected_kind,
                kind
            ),
        });
    }
    Ok(())
}

/// Index every top-level symbol in every non-test source by
/// `(module_path, symbol_name)`, retaining declaration counts so api.yml
/// selectors cannot silently pick one declaration when the source module
/// declares the same symbol more than once.
fn build_symbol_index(
    sources_by_module: &BTreeMap<&str, PublicationApiSource<'_>>,
) -> BTreeMap<(String, String), SourceSymbolDeclarations> {
    let mut index = BTreeMap::new();
    for source in sources_by_module.values() {
        let module = source.module_path.to_string();
        for ty in &source.ast.types {
            record_source_declaration(&mut index, &module, &ty.name, PublicSymbolKind::Type);
        }
        for alias in &source.ast.aliases {
            record_source_declaration(&mut index, &module, &alias.name, PublicSymbolKind::Alias);
        }
        for interface in &source.ast.interfaces {
            record_source_declaration(
                &mut index,
                &module,
                &interface.name,
                PublicSymbolKind::Interface,
            );
        }
        for function in &source.ast.functions {
            record_source_declaration(
                &mut index,
                &module,
                &function.name,
                PublicSymbolKind::Function,
            );
        }
        for constant in &source.ast.consts {
            record_source_declaration(&mut index, &module, &constant.name, PublicSymbolKind::Const);
        }
    }
    index
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SourceSymbolDeclarations {
    declaration_count: usize,
    kinds: BTreeSet<PublicSymbolKind>,
}

impl SourceSymbolDeclarations {
    fn record(&mut self, kind: PublicSymbolKind) {
        self.declaration_count += 1;
        self.kinds.insert(kind);
    }

    fn unique_kind(&self) -> Option<PublicSymbolKind> {
        (self.declaration_count == 1)
            .then(|| self.kinds.iter().next().copied())
            .flatten()
    }

    fn publication_duplicate_label(&self) -> &'static str {
        if self.kinds.iter().any(PublicSymbolKind::is_schema_kind) {
            "schema type"
        } else if self.kinds.contains(&PublicSymbolKind::Function) {
            "callable"
        } else {
            "symbol"
        }
    }
}

impl PublicSymbolKind {
    fn is_schema_kind(kind: &PublicSymbolKind) -> bool {
        matches!(
            kind,
            PublicSymbolKind::Type | PublicSymbolKind::Alias | PublicSymbolKind::Interface
        )
    }
}

fn record_source_declaration(
    index: &mut BTreeMap<(String, String), SourceSymbolDeclarations>,
    module: &str,
    symbol: &str,
    kind: PublicSymbolKind,
) {
    index
        .entry((module.to_string(), symbol.to_string()))
        .or_default()
        .record(kind);
}

fn duplicate_source_symbol_message(
    entry: &PublicationApiSpecEntry,
    declarations: &SourceSymbolDeclarations,
) -> String {
    let selector = entry.source_selector.as_dotted();
    let public_path = entry.public_path_string();
    let label = declarations.publication_duplicate_label();
    format!(
        "api.yml selector {selector} resolves to multiple source declarations; duplicate publication api {label} {public_path}"
    )
}

fn build_test_symbol_index(
    sources_by_module: &BTreeMap<&str, PublicationApiSource<'_>>,
) -> BTreeSet<(String, String)> {
    let test_sources = sources_by_module
        .iter()
        .filter_map(|(module, source)| source.is_test_file.then_some((*module, *source)))
        .collect::<BTreeMap<_, _>>();
    build_symbol_index(&test_sources).into_keys().collect()
}

fn build_impl_method_index(
    sources_by_module: &BTreeMap<&str, PublicationApiSource<'_>>,
) -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for source in sources_by_module.values() {
        for implementation in &source.ast.impls {
            let Some(local_target) =
                local_implementation_target(&implementation.target, source.module_path)
            else {
                continue;
            };
            for method in &implementation.methods {
                methods.insert(format!(
                    "{}.{}.{}",
                    source.module_path, local_target, method.name
                ));
            }
        }
    }
    methods
}

/// Join a surface `path` prefix with an export's local public path.
fn join_public_path(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_string()
    } else if local.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}.{local}")
    }
}

fn public_module_path_from_public_symbol(public_path: &str) -> String {
    public_path
        .rsplit_once('.')
        .map(|(module, _symbol)| module.to_string())
        .unwrap_or_default()
}

/// Resolve an impl `target` to its local type name within `module_path`,
/// stripping the `root.` and module qualifiers.
fn local_implementation_target<'a>(target: &'a str, module_path: &str) -> Option<&'a str> {
    let target = target.strip_prefix("root.").unwrap_or(target);
    if let Some(local) = target.strip_prefix(&format!("{module_path}.")) {
        return Some(local);
    }
    (!target.contains('.')).then_some(target)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::PublicationApiSeed;
    use compiler_input_model::{PublicationApiEntry, PublicationApiPublicInstanceEntry};

    fn source(path: &str, module_path: &str, text: &str) -> CompilerSourceFile {
        CompilerSourceFile::parse(
            PathBuf::from(path),
            module_path.to_string(),
            false,
            false,
            text.to_string(),
            path,
        )
        .unwrap()
    }

    #[test]
    fn builds_linkage_neutral_publication_api_surface() {
        let spec = PublicationApiSpec::from_entries(vec![
            PublicationApiEntry::for_source("chat.Chat", "chat", "Chat"),
            PublicationApiEntry::for_source("chat.ChatList", "chat", "ChatList"),
            PublicationApiEntry::for_source("chat.Events", "chat", "Events"),
            PublicationApiEntry::for_source("chat.start", "chat", "start"),
            PublicationApiEntry::for_source("chat.VERSION", "chat", "VERSION"),
        ]);
        let sources = [source(
            "chat.skiff",
            "chat",
            r#"
                    type Chat {}
                    alias ChatList = Array<Chat>
                    interface Events { function sent(id: string) -> string }
                    function start() -> string { return "" }
                    const VERSION: string = "1"
                "#,
        )];

        let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

        assert_eq!(api.public_modules["chat"], "chat");
        assert_eq!(
            api.public_symbols["chat.Chat"].source_symbol.as_str(),
            "Chat"
        );
        assert_eq!(
            api.public_symbol_for_source_key(&SourceSymbolKey::new("chat", "Chat")),
            Some("chat.Chat")
        );
        assert!(api.is_public_schema_source_key(&SourceSymbolKey::new("chat", "Chat")));
        assert_eq!(
            api.api_source_modules(),
            BTreeSet::from(["chat".to_string()])
        );
        assert!(api.schema_types.contains_key("chat.ChatList"));
        assert!(api.schema_types.contains_key("chat.Events"));
        assert!(api.callables.contains_key("chat.start"));
        assert!(api.public_symbols.contains_key("chat.VERSION"));
    }

    #[test]
    fn builds_public_instance_seed_from_explicit_api_leaf() {
        let spec = PublicationApiSpec::from_public_instances(vec![
            PublicationApiPublicInstanceEntry::for_source(
                "managedLlm",
                "root.llm.managedLlm",
                ["root.llm.ManagedLlm"],
            )
            .unwrap(),
        ]);
        let sources = [source(
            "llm.skiff",
            "llm",
            r#"
                interface ManagedLlm { function sendChat(input: string) -> string }
                type ManagedLlmImpl implements ManagedLlm {}
                const managedLlm: ManagedLlmImpl = ManagedLlmImpl {}
            "#,
        )];

        let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();
        let seed = PublicationApiSeed::from_publication_api(&api);

        assert!(api.public_symbols.is_empty());
        assert_eq!(api.public_instances.len(), 1);
        let instance = &seed.public_instances["managedLlm"];
        assert_eq!(instance.source_module, "llm");
        assert_eq!(instance.source_symbol, "managedLlm");
        assert_eq!(instance.interfaces[0].source_module, "llm");
        assert_eq!(instance.interfaces[0].source_symbol, "ManagedLlm");
        assert!(seed
            .publication_public_instance_symbols
            .contains(&SourceSymbolKey::new("llm", "managedLlm")));
    }

    #[test]
    fn re_exports_apply_aliases_and_path_prefix() {
        let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "Request",
            "types",
            "HttpRequest",
        )]);
        let sources = [source(
            "types.skiff",
            "types",
            r#"
                type HttpRequest {}
            "#,
        )];

        let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

        assert!(api.schema_types.contains_key("Request"));
        assert_eq!(
            api.public_symbols["Request"].source_symbol.as_str(),
            "HttpRequest"
        );
        assert_eq!(
            api.public_symbols["Request"].source_module.as_str(),
            "types"
        );
    }

    #[test]
    fn builds_publication_api_from_resolved_source_modules() {
        let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "crypto.hash",
            "crypto",
            "hash",
        )]);
        let sources = [source(
            "crypto.skiff",
            "std.crypto",
            r#"
                function hash() -> string { return "" }
            "#,
        )];

        let api = PublicationApi::build_from_publication_sources_with_resolved_modules(
            &spec,
            sources.iter(),
            |entry| format!("std.{}", entry.source_module_hint()),
        )
        .unwrap();

        assert_eq!(api.public_modules["crypto"], "std.crypto");
        assert_eq!(
            api.public_symbols["crypto.hash"].source_module,
            "std.crypto"
        );
    }

    #[test]
    fn rejects_duplicate_final_public_paths() {
        let spec = PublicationApiSpec::from_entries(vec![
            PublicationApiEntry::for_source("chat.Chat", "model", "Chat"),
            PublicationApiEntry::for_source("chat.Chat", "model", "Chat"),
        ]);
        let sources = [source(
            "model.skiff",
            "model",
            r#"
                type Chat {}
            "#,
        )];

        let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
            .expect_err("same public path must not be re-exported twice")
            .to_string();

        assert!(
            error.contains("duplicate publication api symbol chat.Chat"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_duplicate_selected_source_type_declarations() {
        let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "ExampleService",
            "internal.example",
            "ExampleService",
        )]);
        let sources = [source(
            "internal/example.skiff",
            "internal.example",
            r#"
                type ExampleService {}
                type ExampleService {}
            "#,
        )];

        let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
            .expect_err("api.yml selector must not resolve to duplicate source declarations")
            .to_string();

        assert!(
            error.contains("api.yml selector internal.example.ExampleService resolves to multiple source declarations"),
            "unexpected error: {error}"
        );
        assert!(
            error.contains("duplicate publication api schema type ExampleService"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_cross_kind_selected_source_symbol_declarations() {
        let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "Collision",
            "model",
            "Collision",
        )]);
        let sources = [source(
            "model.skiff",
            "model",
            r#"
                type Collision {}
                alias Collision = string
            "#,
        )];

        let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
            .expect_err("api.yml selector must not resolve across duplicate source kinds")
            .to_string();

        assert!(
            error.contains(
                "api.yml selector model.Collision resolves to multiple source declarations"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn allows_same_source_symbol_name_in_different_modules() {
        let spec = PublicationApiSpec::from_entries(vec![
            PublicationApiEntry::for_source("left.Shared", "left", "Shared"),
            PublicationApiEntry::for_source("right.Shared", "right", "Shared"),
        ]);
        let sources = [
            source(
                "left.skiff",
                "left",
                r#"
                    type Shared {}
                "#,
            ),
            source(
                "right.skiff",
                "right",
                r#"
                    type Shared {}
                "#,
            ),
        ];

        let api = PublicationApi::build_from_publication_sources(&spec, sources.iter()).unwrap();

        assert_eq!(api.public_symbols["left.Shared"].source_module, "left");
        assert_eq!(api.public_symbols["right.Shared"].source_module, "right");
        assert!(api.schema_types.contains_key("left.Shared"));
        assert!(api.schema_types.contains_key("right.Shared"));
    }

    #[test]
    fn rejects_missing_re_export_target() {
        let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "Missing", "types", "Missing",
        )]);
        let sources = [source("api.skiff", "api", "")];

        let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
            .expect_err("re-export target must exist")
            .to_string();

        assert!(
            error.contains("api.yml selector types.Missing not found"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_impl_method_selector() {
        let spec = PublicationApiSpec::from_entries(vec![PublicationApiEntry::for_source(
            "run",
            "model.Chat",
            "run",
        )]);
        let sources = [source(
            "model.skiff",
            "model",
            r#"
                type Chat {}
                impl Chat {
                  function run(self: Chat) -> string { return "" }
                }
            "#,
        )];

        let error = PublicationApi::build_from_publication_sources(&spec, sources.iter())
            .expect_err("impl methods cannot be api.yml selectors")
            .to_string();

        assert!(
            error.contains("points to an impl method"),
            "unexpected error: {error}"
        );
    }
}
