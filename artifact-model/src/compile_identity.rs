use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_identity {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

// These identities are intentionally distinct Rust types even though their
// artifact representation is a framed string. In particular, ContractTypeId
// is not an AbiTypeId alias: contract nominal equality and package-local ABI
// nominal equality have different owners and inputs.
string_identity!(ContractTypeId);
string_identity!(ContractOperationId);
string_identity!(ServiceProtocolIdentity);
string_identity!(PackageBuildId);
string_identity!(PackageLocalAbiIdentity);
string_identity!(PackageCallableId);
string_identity!(DeploymentRevision);
string_identity!(DeploymentArtifactIdentity);
string_identity!(AssemblyIdentity);
