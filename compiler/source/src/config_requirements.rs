use super::config_usage::{ConfigPresenceUse, ConfigSourceSpan, ConfigUsageSeed, ConfigUse};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigRequirementSet {
    requirements: Vec<ConfigRequirement>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirement {
    path: String,
    access: ConfigRequirementAccess,
    provenances: Vec<ConfigRequirementProvenance>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigRequirementAccess {
    Require { ty: String },
    Optional { ty: String },
    Has,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConfigRequirementProvenance {
    source_path: String,
    source_span: Option<ConfigSourceSpan>,
}

impl ConfigRequirementSet {
    pub fn from_usage_seed(seed: &ConfigUsageSeed) -> Self {
        let mut requirements = seed
            .typed
            .iter()
            .map(ConfigRequirement::from_config_use)
            .chain(
                seed.presence
                    .iter()
                    .map(ConfigRequirement::from_presence_use),
            )
            .collect::<Vec<_>>();
        requirements.sort();

        let mut canonical = Vec::<ConfigRequirement>::new();
        for requirement in requirements {
            if let Some(existing) = canonical
                .iter_mut()
                .find(|existing| existing.has_same_requirement_identity(&requirement))
            {
                existing.provenances.extend(requirement.provenances);
                existing.provenances.sort();
                existing.provenances.dedup();
            } else {
                canonical.push(requirement);
            }
        }
        Self {
            requirements: canonical,
        }
    }

    pub fn requirements(&self) -> &[ConfigRequirement] {
        &self.requirements
    }
}

impl ConfigRequirement {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn access(&self) -> &ConfigRequirementAccess {
        &self.access
    }

    pub fn source_path(&self) -> &str {
        self.provenances
            .first()
            .map(ConfigRequirementProvenance::source_path)
            .unwrap_or("")
    }

    pub fn provenances(&self) -> &[ConfigRequirementProvenance] {
        &self.provenances
    }

    fn has_same_requirement_identity(&self, other: &Self) -> bool {
        self.path == other.path && self.access == other.access
    }

    fn from_config_use(config_use: &ConfigUse) -> Self {
        let access = if config_use.required {
            ConfigRequirementAccess::Require {
                ty: config_use.ty.clone(),
            }
        } else {
            ConfigRequirementAccess::Optional {
                ty: config_use.ty.clone(),
            }
        };
        Self {
            path: config_use.path.clone(),
            access,
            provenances: vec![ConfigRequirementProvenance::new(
                config_use.source_path.clone(),
                config_use.source_span,
            )],
        }
    }

    fn from_presence_use(config_use: &ConfigPresenceUse) -> Self {
        Self {
            path: config_use.path.clone(),
            access: ConfigRequirementAccess::Has,
            provenances: vec![ConfigRequirementProvenance::new(
                config_use.source_path.clone(),
                config_use.source_span,
            )],
        }
    }
}

impl ConfigRequirementAccess {
    pub fn typed(&self) -> Option<(&str, bool)> {
        match self {
            Self::Require { ty } => Some((ty, true)),
            Self::Optional { ty } => Some((ty, false)),
            Self::Has => None,
        }
    }

    pub fn is_has(&self) -> bool {
        matches!(self, Self::Has)
    }
}

impl ConfigRequirementProvenance {
    pub fn source_path(&self) -> &str {
        &self.source_path
    }

    pub fn source_span(&self) -> Option<ConfigSourceSpan> {
        self.source_span
    }

    fn new(source_path: String, source_span: Option<ConfigSourceSpan>) -> Self {
        Self {
            source_path,
            source_span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_usage_keeps_presence_and_deduplicates_identical_reads() {
        let seed = ConfigUsageSeed {
            typed: vec![
                ConfigUse {
                    path: "provider.apiKey".to_string(),
                    ty: "string".to_string(),
                    required: true,
                    source_path: "first.skiff".to_string(),
                    source_span: None,
                },
                ConfigUse {
                    path: "provider.apiKey".to_string(),
                    ty: "string".to_string(),
                    required: true,
                    source_path: "second.skiff".to_string(),
                    source_span: None,
                },
            ],
            presence: vec![ConfigPresenceUse {
                path: "provider".to_string(),
                source_path: "first.skiff".to_string(),
                source_span: None,
            }],
        };
        let requirements = ConfigRequirementSet::from_usage_seed(&seed);
        assert_eq!(requirements.requirements().len(), 2);
        assert!(requirements
            .requirements()
            .iter()
            .any(|requirement| requirement.access().is_has()));
        assert_eq!(
            requirements
                .requirements()
                .iter()
                .find(|requirement| requirement.path() == "provider.apiKey")
                .unwrap()
                .provenances()
                .len(),
            2
        );
    }
}
