use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    engine::{CollectionInventory, CollectionScan},
    model::ValidatedCollectionMapping,
    MigrationToolError, STAGING_PREFIX,
};

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MigrationStatus {
    Planned,
    Staged,
    Committed,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MigrationReceipt {
    schema_version: String,
    plan_commitment: String,
    keyring_fingerprint: String,
    mappings: Vec<CollectionReceipt>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CollectionReceipt {
    pub(crate) mapping_id: String,
    pub(crate) staging_collection: String,
    pub(crate) status: MigrationStatus,
    source_count: Option<u64>,
    source_semantic_hash: Option<String>,
    source_index_hash: Option<String>,
    source_index_count: Option<u64>,
    target_index_hash: Option<String>,
    target_index_count: Option<u64>,
}

impl MigrationReceipt {
    pub(crate) fn new(
        schema: &str,
        plan_commitment: &str,
        keyring_fingerprint: &str,
        mappings: &[ValidatedCollectionMapping],
    ) -> Self {
        Self {
            schema_version: schema.to_owned(),
            plan_commitment: plan_commitment.to_owned(),
            keyring_fingerprint: keyring_fingerprint.to_owned(),
            mappings: mappings
                .iter()
                .map(|mapping| CollectionReceipt {
                    mapping_id: mapping.mapping_id.clone(),
                    staging_collection: staging_name(plan_commitment, &mapping.mapping_id),
                    status: MigrationStatus::Planned,
                    source_count: None,
                    source_semantic_hash: None,
                    source_index_hash: None,
                    source_index_count: None,
                    target_index_hash: None,
                    target_index_count: None,
                })
                .collect(),
        }
    }

    pub(crate) fn validate_resume(
        &self,
        schema: &str,
        plan_commitment: &str,
        keyring_fingerprint: &str,
        mappings: &[ValidatedCollectionMapping],
    ) -> Result<(), MigrationToolError> {
        if self.schema_version != schema
            || self.plan_commitment != plan_commitment
            || self.keyring_fingerprint != keyring_fingerprint
            || self.mappings.len() != mappings.len()
        {
            return Err(MigrationToolError::Receipt);
        }
        for (entry, mapping) in self.mappings.iter().zip(mappings) {
            if entry.mapping_id != mapping.mapping_id
                || entry.staging_collection != staging_name(plan_commitment, &mapping.mapping_id)
            {
                return Err(MigrationToolError::Receipt);
            }
        }
        Ok(())
    }

    pub(crate) fn entry(&self, mapping_id: &str) -> Result<&CollectionReceipt, MigrationToolError> {
        self.mappings
            .iter()
            .find(|entry| entry.mapping_id == mapping_id)
            .ok_or(MigrationToolError::Receipt)
    }

    pub(crate) fn entry_mut(
        &mut self,
        mapping_id: &str,
    ) -> Result<&mut CollectionReceipt, MigrationToolError> {
        self.mappings
            .iter_mut()
            .find(|entry| entry.mapping_id == mapping_id)
            .ok_or(MigrationToolError::Receipt)
    }
}

impl CollectionReceipt {
    pub(crate) fn bind_inventory(
        &mut self,
        inventory: &CollectionInventory,
    ) -> Result<(), MigrationToolError> {
        if self.mapping_id != inventory.mapping_id {
            return Err(MigrationToolError::Receipt);
        }
        let incoming = (
            inventory.source_count,
            inventory.source_semantic_hash.as_str(),
            inventory.source_index_hash.as_str(),
            inventory.source_index_count,
            inventory.target_index_hash.as_str(),
            inventory.target_index_count,
        );
        if let (
            Some(count),
            Some(semantic),
            Some(source_index),
            Some(source_index_count),
            Some(target_index),
            Some(target_index_count),
        ) = (
            self.source_count,
            self.source_semantic_hash.as_deref(),
            self.source_index_hash.as_deref(),
            self.source_index_count,
            self.target_index_hash.as_deref(),
            self.target_index_count,
        ) {
            if (
                count,
                semantic,
                source_index,
                source_index_count,
                target_index,
                target_index_count,
            ) != incoming
            {
                return Err(MigrationToolError::Verification(self.mapping_id.clone()));
            }
            return Ok(());
        }
        if self.source_count.is_some()
            || self.source_semantic_hash.is_some()
            || self.source_index_hash.is_some()
            || self.source_index_count.is_some()
            || self.target_index_hash.is_some()
            || self.target_index_count.is_some()
        {
            return Err(MigrationToolError::Receipt);
        }
        self.source_count = Some(inventory.source_count);
        self.source_semantic_hash = Some(inventory.source_semantic_hash.clone());
        self.source_index_hash = Some(inventory.source_index_hash.clone());
        self.source_index_count = Some(inventory.source_index_count);
        self.target_index_hash = Some(inventory.target_index_hash.clone());
        self.target_index_count = Some(inventory.target_index_count);
        Ok(())
    }

    pub(crate) fn assert_source_unchanged(
        &self,
        scan: &CollectionScan,
    ) -> Result<(), MigrationToolError> {
        if self.source_count != Some(scan.count)
            || self.source_semantic_hash.as_deref() != Some(scan.semantic_hash.as_str())
            || self.source_index_hash.as_deref() != Some(scan.index_hash.as_str())
            || self.source_index_count != Some(scan.index_count)
        {
            return Err(MigrationToolError::Verification(self.mapping_id.clone()));
        }
        Ok(())
    }

    pub(crate) fn assert_verified(&self, scan: &CollectionScan) -> Result<(), MigrationToolError> {
        if self.source_count != Some(scan.count)
            || self.source_semantic_hash.as_deref() != Some(scan.semantic_hash.as_str())
            || self.target_index_hash.as_deref() != Some(scan.index_hash.as_str())
            || self.target_index_count != Some(scan.index_count)
        {
            return Err(MigrationToolError::Verification(self.mapping_id.clone()));
        }
        Ok(())
    }
}

pub(crate) struct SecureReceiptStore {
    path: PathBuf,
}

impl SecureReceiptStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(crate) fn load(&self) -> Result<Option<MigrationReceipt>, MigrationToolError> {
        let file = match secure_open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(MigrationToolError::Receipt),
        };
        validate_secure_file(&file)?;
        let receipt = serde_json::from_reader(file).map_err(|_| MigrationToolError::Receipt)?;
        Ok(Some(receipt))
    }

    pub(crate) fn store(&self, receipt: &MigrationReceipt) -> Result<(), MigrationToolError> {
        let parent = self.path.parent().ok_or(MigrationToolError::Receipt)?;
        fs::create_dir_all(parent).map_err(|_| MigrationToolError::Receipt)?;
        let temporary = parent.join(format!(
            ".skiff-service-db-migration-receipt-{}.tmp",
            uuid::Uuid::new_v4()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|_| MigrationToolError::Receipt)?;
        let result = (|| {
            serde_json::to_writer(&mut file, receipt).map_err(|_| MigrationToolError::Receipt)?;
            file.write_all(b"\n")
                .map_err(|_| MigrationToolError::Receipt)?;
            file.sync_all().map_err(|_| MigrationToolError::Receipt)?;
            fs::rename(&temporary, &self.path).map_err(|_| MigrationToolError::Receipt)?;
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| MigrationToolError::Receipt)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn staging_name(plan_commitment: &str, mapping_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"skiff-service-db-hardcut-staging-v1");
    hasher.update(plan_commitment.as_bytes());
    hasher.update(mapping_id.as_bytes());
    format!("{STAGING_PREFIX}{}", &hex::encode(hasher.finalize())[..32])
}

fn secure_open(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    options.open(path)
}

fn validate_secure_file(file: &File) -> Result<(), MigrationToolError> {
    let metadata = file.metadata().map_err(|_| MigrationToolError::Receipt)?;
    if !metadata.file_type().is_file() {
        return Err(MigrationToolError::Receipt);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(MigrationToolError::Receipt);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
