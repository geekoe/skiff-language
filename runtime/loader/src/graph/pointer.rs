use serde_json::Value;
use std::path::PathBuf;

use super::loader::ArtifactGraphLoader;
use crate::{
    paths::ArtifactRootRelativePath,
    pointer_files::{
        load_dev_reload_pointers_from_roots, load_service_version_build_pointers_from_roots,
    },
    types::{ArtifactIndexPointer, RootedArtifactPointerFile},
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

impl ArtifactGraphLoader<'_> {
    pub(super) fn resolve_service_unit_pointer(
        &self,
        pointer: &ArtifactIndexPointer,
    ) -> anyhow::Result<ArtifactRootRelativePath> {
        if let Some(path) = &pointer.service_unit_path {
            return ArtifactRootRelativePath::new(path, "service unit");
        }

        let service_assembly_path =
            ArtifactRootRelativePath::new(&pointer.service_assembly.path, "serviceAssembly")?;
        let assembly_value = self.read_artifact_json(&service_assembly_path, "serviceAssembly")?;
        let assembly_object = assembly_value.as_object().ok_or_else(|| {
            anyhow::anyhow!(
                "{} serviceAssembly must be an object",
                pointer.service_assembly.path.display()
            )
        })?;
        if let Some(path) = unit_ref_path(
            assembly_object.get("serviceUnit"),
            &format!(
                "{} serviceAssembly.serviceUnit",
                pointer.service_assembly.path.display()
            ),
        )? {
            return Ok(path);
        }

        anyhow::bail!(
            "artifact pointer for service {} build {} does not declare canonical serviceUnit.unitPath; old serviceAssembly request path is intentionally not used by RuntimeProgram loader",
            pointer.service_id,
            pointer.build_id
        )
    }
}

fn unit_ref_path(
    value: Option<&Value>,
    label: &str,
) -> anyhow::Result<Option<ArtifactRootRelativePath>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("{label} must be an object with unitPath"))?;
    let path = object
        .get("unitPath")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("{label} requires unitPath"))?;
    Ok(Some(ArtifactRootRelativePath::parse(path, label)?))
}
