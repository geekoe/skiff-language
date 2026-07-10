use std::{collections::BTreeMap, fmt, sync::Arc};

use skiff_artifact_model::PublicationResourceRef;

use crate::addr::UnitAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationResourcePath {
    canonical: String,
}

impl PublicationResourcePath {
    pub fn parse(path: &str) -> Result<Self, PublicationResourcePathError> {
        validate_publication_resource_path(path)?;
        Ok(Self {
            canonical: path.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }
}

impl fmt::Display for PublicationResourcePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.canonical)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationResourcePathError {
    message: String,
}

impl PublicationResourcePathError {
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PublicationResourcePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for PublicationResourcePathError {}

fn validate_publication_resource_path(path: &str) -> Result<(), PublicationResourcePathError> {
    if path.is_empty() {
        return Err(invalid_resource_path("resource path must not be empty"));
    }
    if path.starts_with('/') {
        return Err(invalid_resource_path("resource path must be relative"));
    }
    if path.contains('\\') {
        return Err(invalid_resource_path("resource path must use / separators"));
    }
    if path.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(invalid_resource_path(
            "resource path must not contain control characters",
        ));
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(invalid_resource_path(
                "resource path must not contain empty segments",
            ));
        }
        if segment == "." {
            return Err(invalid_resource_path(
                "resource path must not contain . segments",
            ));
        }
        if segment == ".." {
            return Err(invalid_resource_path(
                "resource path must not contain .. segments",
            ));
        }
    }
    Ok(())
}

fn invalid_resource_path(message: impl Into<String>) -> PublicationResourcePathError {
    PublicationResourcePathError {
        message: message.into(),
    }
}

#[derive(Debug, Clone)]
pub struct LoadedPublicationResource {
    pub meta: PublicationResourceRef,
    pub bytes: Arc<[u8]>,
}

#[derive(Debug, Clone, Default)]
pub struct PublicationResourceTable {
    pub resources_by_path: BTreeMap<String, LoadedPublicationResource>,
}

impl PublicationResourceTable {
    pub fn get(&self, path: &str) -> Option<&LoadedPublicationResource> {
        self.resources_by_path.get(path)
    }

    pub fn insert(
        &mut self,
        path: String,
        resource: LoadedPublicationResource,
    ) -> Option<LoadedPublicationResource> {
        self.resources_by_path.insert(path, resource)
    }

    pub fn is_empty(&self) -> bool {
        self.resources_by_path.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RuntimeProgramResourceView<'a> {
    service_resources: &'a PublicationResourceTable,
    package_resources: &'a [PublicationResourceTable],
}

impl<'a> RuntimeProgramResourceView<'a> {
    pub fn new(
        service_resources: &'a PublicationResourceTable,
        package_resources: &'a [PublicationResourceTable],
    ) -> Self {
        Self {
            service_resources,
            package_resources,
        }
    }

    pub fn lookup(
        &self,
        owner: &UnitAddr,
        path: &str,
    ) -> Result<Option<&'a LoadedPublicationResource>, RuntimeProgramResourceLookupError> {
        Ok(match owner {
            UnitAddr::Service => self.service_resources.get(path),
            UnitAddr::Package(slot) => self
                .package_resources
                .get(*slot)
                .ok_or(RuntimeProgramResourceLookupError::PackageSlotOutOfBounds {
                    slot: *slot,
                    package_count: self.package_resources.len(),
                })?
                .get(path),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeProgramResourceLookupError {
    PackageSlotOutOfBounds { slot: usize, package_count: usize },
}

impl fmt::Display for RuntimeProgramResourceLookupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PackageSlotOutOfBounds {
                slot,
                package_count,
            } => write!(
                formatter,
                "resource owner package slot {slot} is out of bounds for {package_count} packages"
            ),
        }
    }
}

impl std::error::Error for RuntimeProgramResourceLookupError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_resource_path_accepts_canonical_logical_paths() {
        let path = PublicationResourcePath::parse("prompts/system.md").unwrap();

        assert_eq!(path.as_str(), "prompts/system.md");
    }

    #[test]
    fn publication_resource_path_rejects_non_canonical_paths() {
        for path in [
            "", "/a", "./a", "a/./b", "a//b", "a\\b", "../a", "a/..", "a/",
        ] {
            assert!(
                PublicationResourcePath::parse(path).is_err(),
                "{path:?} should be invalid"
            );
        }
    }
}
