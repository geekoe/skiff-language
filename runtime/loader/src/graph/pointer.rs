use std::path::PathBuf;

use crate::{
    pointer_files::{
        load_dev_reload_pointers_from_roots, load_service_version_build_pointers_from_roots,
    },
    types::RootedArtifactPointerFile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeProgramArtifactSelection {
    pub service_id: String,
    pub service_version: Option<String>,
    pub build_id: Option<String>,
    pub dev_reload: bool,
}

impl RuntimeProgramArtifactSelection {
    pub fn release(service_id: impl Into<String>, service_version: impl Into<String>) -> Self {
        Self {
            service_id: service_id.into(),
            service_version: Some(service_version.into()),
            build_id: None,
            dev_reload: false,
        }
    }

    pub fn release_build(service_id: impl Into<String>, build_id: impl Into<String>) -> Self {
        Self {
            service_id: service_id.into(),
            service_version: None,
            build_id: Some(build_id.into()),
            dev_reload: false,
        }
    }

    pub fn dev(service_id: impl Into<String>) -> Self {
        Self {
            service_id: service_id.into(),
            service_version: None,
            build_id: None,
            dev_reload: true,
        }
    }
}

pub fn select_runtime_program_pointer_from_roots(
    artifact_roots: &[PathBuf],
    selection: &RuntimeProgramArtifactSelection,
) -> anyhow::Result<RootedArtifactPointerFile> {
    let pointers = if selection.dev_reload {
        load_dev_reload_pointers_from_roots(artifact_roots)?
    } else {
        load_service_version_build_pointers_from_roots(artifact_roots)?
    };

    let mut matches = pointers
        .into_iter()
        .filter(|pointer| pointer.entry.service_id == selection.service_id)
        .filter(|pointer| {
            selection
                .build_id
                .as_ref()
                .is_none_or(|build_id| pointer.entry.build_id == *build_id)
        })
        .filter(|pointer| {
            selection.service_version.as_ref().is_none_or(|version| {
                pointer
                    .entry
                    .service_version
                    .as_ref()
                    .is_some_and(|pointer_version| pointer_version == version)
            })
        })
        .collect::<Vec<_>>();

    matches.sort_by(|left, right| {
        left.entry
            .service_version
            .cmp(&right.entry.service_version)
            .then(left.entry.build_id.cmp(&right.entry.build_id))
            .then(left.artifact_root.cmp(&right.artifact_root))
    });

    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => anyhow::bail!(
            "no runtime program artifact pointer matched serviceId {} version {:?} build {:?}",
            selection.service_id,
            selection.service_version,
            selection.build_id
        ),
        count => anyhow::bail!(
            "{count} runtime program artifact pointers matched serviceId {} version {:?} build {:?}; provide a version or buildId",
            selection.service_id,
            selection.service_version,
            selection.build_id
        ),
    }
}
