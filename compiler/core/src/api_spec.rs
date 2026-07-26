use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    export_config::{is_valid_dotted_module_path, package_public_path},
    id::{SKIFF_STD_PUBLICATION_ID, STD_SOURCE_ALIAS},
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PublicationApiSpec {
    pub entries: Vec<PublicationApiEntry>,
    pub public_instances: Vec<PublicationApiPublicInstanceEntry>,
    pub source: Option<PublicationApiSource>,
}

pub type PublicationApiSpecEntry = PublicationApiEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationApiEntry {
    pub public_path: Vec<String>,
    pub source_selector: SourceSymbolSelector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationApiPublicInstanceEntry {
    pub public_path: Vec<String>,
    pub const_selector: SourceSymbolSelector,
    pub interface_selectors: Vec<SourceSymbolSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSymbolSelector {
    pub module_path: String,
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationApiSource {
    pub relative_path: PathBuf,
    pub content_hash: String,
}

impl PublicationApiSpec {
    pub fn new(
        entries: Vec<PublicationApiEntry>,
        public_instances: Vec<PublicationApiPublicInstanceEntry>,
        source: Option<PublicationApiSource>,
    ) -> Self {
        Self {
            entries,
            public_instances,
            source,
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_entries(entries: Vec<PublicationApiEntry>) -> Self {
        Self {
            entries,
            public_instances: Vec::new(),
            source: None,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_public_instances(public_instances: Vec<PublicationApiPublicInstanceEntry>) -> Self {
        Self {
            entries: Vec::new(),
            public_instances,
            source: None,
        }
    }

    pub fn with_source(mut self, source: PublicationApiSource) -> Self {
        self.source = Some(source);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.public_instances.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &PublicationApiEntry> {
        self.entries.iter()
    }

    pub fn public_instances(&self) -> impl Iterator<Item = &PublicationApiPublicInstanceEntry> {
        self.public_instances.iter()
    }

    pub fn public_modules<'a>(
        &'a self,
        publication_id: &'a str,
    ) -> impl Iterator<Item = String> + 'a {
        let public_id = if publication_id == SKIFF_STD_PUBLICATION_ID {
            STD_SOURCE_ALIAS
        } else {
            publication_id
        };
        self.entries
            .iter()
            .map(move |entry| package_public_path(public_id, &entry.public_module_path_segment()))
            .chain(self.public_instances.iter().map(move |entry| {
                package_public_path(public_id, &entry.public_module_path_segment())
            }))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn module_map(&self) -> BTreeMap<String, String> {
        self.entries
            .iter()
            .map(|entry| {
                (
                    entry.public_path_string(),
                    entry.source_selector.module_path.clone(),
                )
            })
            .chain(self.public_instances.iter().map(|entry| {
                (
                    entry.public_path_string(),
                    entry.const_selector.module_path.clone(),
                )
            }))
            .collect()
    }
}

impl PublicationApiEntry {
    pub fn new(public_path: Vec<String>, source_selector: SourceSymbolSelector) -> Self {
        Self {
            public_path,
            source_selector,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn for_source(
        public_path: impl AsRef<str>,
        module_path: impl Into<String>,
        symbol: impl Into<String>,
    ) -> Self {
        Self {
            public_path: public_path_segments(public_path.as_ref()),
            source_selector: SourceSymbolSelector::new(module_path, symbol),
        }
    }

    pub fn public_path_string(&self) -> String {
        self.public_path.join(".")
    }

    pub fn public_module_path_segment(&self) -> String {
        self.public_path
            .split_last()
            .map(|(_symbol, module)| module.join("."))
            .unwrap_or_default()
    }

    pub fn source_module_hint(&self) -> &str {
        &self.source_selector.module_path
    }

    pub fn source_symbol(&self) -> &str {
        &self.source_selector.symbol
    }

    pub fn public_module_path(&self, publication_id: &str) -> String {
        package_public_path(publication_id, &self.public_path_string())
    }
}

impl PublicationApiPublicInstanceEntry {
    pub fn new(
        public_path: Vec<String>,
        const_selector: SourceSymbolSelector,
        interface_selectors: Vec<SourceSymbolSelector>,
    ) -> Self {
        Self {
            public_path,
            const_selector,
            interface_selectors,
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn for_source(
        public_path: impl AsRef<str>,
        const_selector: impl AsRef<str>,
        interface_selectors: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, String> {
        Ok(Self {
            public_path: public_path_segments(public_path.as_ref()),
            const_selector: SourceSymbolSelector::parse_api_selector(
                const_selector.as_ref(),
                true,
            )?,
            interface_selectors: interface_selectors
                .into_iter()
                .map(|selector| SourceSymbolSelector::parse_api_selector(selector.as_ref(), true))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    pub fn public_path_string(&self) -> String {
        self.public_path.join(".")
    }

    pub fn public_module_path_segment(&self) -> String {
        self.public_path
            .split_last()
            .map(|(_symbol, module)| module.join("."))
            .unwrap_or_default()
    }

    pub fn source_module_hint(&self) -> &str {
        &self.const_selector.module_path
    }

    pub fn source_symbol(&self) -> &str {
        &self.const_selector.symbol
    }
}

impl SourceSymbolSelector {
    pub fn new(module_path: impl Into<String>, symbol: impl Into<String>) -> Self {
        Self {
            module_path: module_path.into(),
            symbol: symbol.into(),
        }
    }

    pub fn parse(selector: &str) -> Result<Self, String> {
        Self::parse_api_selector(selector, false)
    }

    pub fn parse_api_selector(selector: &str, allow_root_prefix: bool) -> Result<Self, String> {
        let parts = selector.split('.').collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err("must be module.path.Symbol".to_string());
        }
        if parts.first() == Some(&"root") {
            if !allow_root_prefix {
                return Err("must not include root. prefix".to_string());
            }
            if parts.len() < 3 {
                return Err("root. prefix must be followed by module.path.Symbol".to_string());
            }
        }
        let selector_parts = if parts.first() == Some(&"root") {
            &parts[1..]
        } else {
            &parts[..]
        };
        if selector_parts
            .iter()
            .any(|part| !is_valid_identifier_segment(part))
        {
            return Err("must contain only identifier segments".to_string());
        }
        let symbol = selector_parts
            .last()
            .expect("selector has at least two parts");
        let module_path = selector_parts[..selector_parts.len() - 1].join(".");
        if !is_valid_dotted_module_path(&module_path) {
            return Err("must have a valid module path".to_string());
        }
        Ok(Self::new(module_path, *symbol))
    }

    pub fn as_dotted(&self) -> String {
        format!("{}.{}", self.module_path, self.symbol)
    }
}

impl PublicationApiSource {
    pub fn new(relative_path: impl Into<PathBuf>, content_hash: impl Into<String>) -> Self {
        Self {
            relative_path: relative_path.into(),
            content_hash: content_hash.into(),
        }
    }
}

pub fn is_valid_identifier_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn public_path_segments(path: &str) -> Vec<String> {
    if path.is_empty() {
        Vec::new()
    } else {
        path.split('.').map(ToString::to_string).collect()
    }
}
