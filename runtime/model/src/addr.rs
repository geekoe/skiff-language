use std::fmt;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub type PackageSlot = usize;
pub type LoadedFileIndex = usize;
pub type ExecutableIndex = usize;
pub type TypeIndex = usize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum UnitAddr {
    Service,
    Package(PackageSlot),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum FileAddr {
    LoadedFileIndex(LoadedFileIndex),
    FileIrIdentity(String),
}

impl<'de> Deserialize<'de> for FileAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", content = "value", rename_all = "camelCase")]
        enum TaggedFileAddr {
            LoadedFileIndex(LoadedFileIndex),
            FileIrIdentity(String),
        }

        let value = Value::deserialize(deserializer)?;
        if let Ok(tagged) = serde_json::from_value::<TaggedFileAddr>(value.clone()) {
            return Ok(match tagged {
                TaggedFileAddr::LoadedFileIndex(index) => Self::LoadedFileIndex(index),
                TaggedFileAddr::FileIrIdentity(identity) => Self::FileIrIdentity(identity),
            });
        }
        if let Some(identity) = value.get("fileIrIdentity").and_then(Value::as_str) {
            return Ok(Self::FileIrIdentity(identity.to_string()));
        }
        Err(D::Error::custom(
            "expected tagged file address or typed file IR reference",
        ))
    }
}

impl FileAddr {
    pub fn loaded_file(index: LoadedFileIndex) -> Self {
        Self::LoadedFileIndex(index)
    }

    pub fn file_ir_identity(identity: impl Into<String>) -> Self {
        Self::FileIrIdentity(identity.into())
    }
}

impl Default for FileAddr {
    fn default() -> Self {
        Self::LoadedFileIndex(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutableAddr {
    pub unit: UnitAddr,
    pub file: FileAddr,
    pub executable: ExecutableIndex,
}

impl ExecutableAddr {
    pub fn service(file: LoadedFileIndex, executable: ExecutableIndex) -> Self {
        Self {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(file),
            executable,
        }
    }

    pub fn package(
        package_slot: PackageSlot,
        file: LoadedFileIndex,
        executable: ExecutableIndex,
    ) -> Self {
        Self {
            unit: UnitAddr::Package(package_slot),
            file: FileAddr::LoadedFileIndex(file),
            executable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeAddr {
    pub unit: UnitAddr,
    pub file: FileAddr,
    pub type_index: TypeIndex,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstAddr {
    pub unit: UnitAddr,
    pub file: FileAddr,
    pub const_index: usize,
}

impl fmt::Display for UnitAddr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitAddr::Service => formatter.write_str("service"),
            UnitAddr::Package(slot) => write!(formatter, "package[{slot}]"),
        }
    }
}

impl fmt::Display for FileAddr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileAddr::LoadedFileIndex(index) => write!(formatter, "file[{index}]"),
            FileAddr::FileIrIdentity(identity) => write!(formatter, "fileIrIdentity({identity})"),
        }
    }
}

impl fmt::Display for ExecutableAddr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:executable[{}]",
            self.unit, self.file, self.executable
        )
    }
}

impl fmt::Display for TypeAddr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:type[{}]",
            self.unit, self.file, self.type_index
        )
    }
}

impl fmt::Display for ConstAddr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:const[{}]",
            self.unit, self.file, self.const_index
        )
    }
}
