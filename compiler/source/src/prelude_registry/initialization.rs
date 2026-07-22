use std::{path::PathBuf, sync::OnceLock};

use skiff_compiler_input::{CompilerPlatformSources, CompilerPlatformSourcesError};
use thiserror::Error;

use super::PreludeRegistry;

struct InitializedPreludeRegistry {
    platform_root: PathBuf,
    registry: PreludeRegistry,
}

static REGISTRY: OnceLock<InitializedPreludeRegistry> = OnceLock::new();

#[derive(Debug, Error)]
pub enum PreludeRegistryInitializationError {
    #[error(
        "prelude registry is already initialized from platform root {initialized_root}; requested {requested_root}"
    )]
    DifferentPlatformRoot {
        initialized_root: PathBuf,
        requested_root: PathBuf,
    },
    #[error("invalid compiler platform sources at {root}: {source}")]
    PlatformSources {
        root: PathBuf,
        #[source]
        source: CompilerPlatformSourcesError,
    },
    #[error("failed to load prelude registry from platform root {root}: {message}")]
    Load { root: PathBuf, message: String },
}

pub fn initialize_prelude_registry(
    platform_sources: &CompilerPlatformSources,
) -> Result<&'static PreludeRegistry, PreludeRegistryInitializationError> {
    if let Some(initialized) = REGISTRY.get() {
        return registry_for_root(initialized, platform_sources.root());
    }

    platform_sources.revalidate().map_err(|source| {
        PreludeRegistryInitializationError::PlatformSources {
            root: platform_sources.root().to_path_buf(),
            source,
        }
    })?;
    let mut registry =
        PreludeRegistry::try_from_platform_sources(platform_sources).map_err(|message| {
            PreludeRegistryInitializationError::Load {
                root: platform_sources.root().to_path_buf(),
                message,
            }
        })?;
    registry.prelude_identity_parts = platform_sources
        .read_prelude_sources()
        .map_err(
            |source| PreludeRegistryInitializationError::PlatformSources {
                root: platform_sources.root().to_path_buf(),
                source,
            },
        )?
        .into_iter()
        .flat_map(|(relative, text)| [relative.to_string_lossy().into_owned(), text])
        .collect();

    let candidate = InitializedPreludeRegistry {
        platform_root: platform_sources.root().to_path_buf(),
        registry,
    };
    if REGISTRY.set(candidate).is_err() {
        let initialized = REGISTRY
            .get()
            .expect("prelude registry was initialized by a concurrent caller");
        return registry_for_root(initialized, platform_sources.root());
    }
    Ok(&REGISTRY
        .get()
        .expect("prelude registry was just initialized")
        .registry)
}

pub fn prelude_registry() -> &'static PreludeRegistry {
    &REGISTRY
        .get()
        .expect(
            "prelude registry is not initialized; compiler pipeline must provide CompilerPlatformSources",
        )
        .registry
}

fn registry_for_root(
    initialized: &'static InitializedPreludeRegistry,
    requested_root: &std::path::Path,
) -> Result<&'static PreludeRegistry, PreludeRegistryInitializationError> {
    if initialized.platform_root == requested_root {
        return Ok(&initialized.registry);
    }
    Err(PreludeRegistryInitializationError::DifferentPlatformRoot {
        initialized_root: initialized.platform_root.clone(),
        requested_root: requested_root.to_path_buf(),
    })
}
