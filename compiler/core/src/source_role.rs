#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationSourceRole {
    Contract,
    Implementation,
    Package,
}

impl PublicationSourceRole {
    pub fn from_api_flag(is_api: bool) -> Self {
        if is_api {
            Self::Contract
        } else {
            Self::Implementation
        }
    }

    pub fn is_contract(self) -> bool {
        matches!(self, Self::Contract)
    }
}

impl serde::Serialize for PublicationSourceRole {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let role = match self {
            Self::Contract => "contract",
            Self::Implementation => "implementation",
            Self::Package => "package",
        };
        serializer.serialize_str(role)
    }
}

#[cfg(test)]
mod tests {
    use super::PublicationSourceRole;

    #[test]
    fn serializes_publication_source_roles_with_stable_wire_strings() {
        assert_eq!(
            serde_json::to_string(&PublicationSourceRole::Contract).unwrap(),
            "\"contract\""
        );
        assert_eq!(
            serde_json::to_string(&PublicationSourceRole::Implementation).unwrap(),
            "\"implementation\""
        );
        assert_eq!(
            serde_json::to_string(&PublicationSourceRole::Package).unwrap(),
            "\"package\""
        );
    }
}
