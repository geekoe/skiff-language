#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArtifactLoadOptions {
    pub(super) source: ArtifactLoadSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ArtifactLoadSource {
    DevReload,
    Release,
}

impl ArtifactLoadOptions {
    pub(crate) fn dev_reload() -> Self {
        Self {
            source: ArtifactLoadSource::DevReload,
        }
    }

    pub(crate) fn release() -> Self {
        Self {
            source: ArtifactLoadSource::Release,
        }
    }

    pub(crate) fn from_control(dev_reload: Option<bool>) -> Self {
        if dev_reload.unwrap_or(false) {
            Self::dev_reload()
        } else {
            Self::release()
        }
    }
}
