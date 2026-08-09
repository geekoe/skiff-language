use skiff_artifact_model::PackageCallableId;
use thiserror::Error;

/// The source-level implementation callable forms that receive package-local identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplementationCallableKind {
    Function,
    ImplMethod,
}

/// A failure to construct a canonical package callable identity from source components.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PackageCallableIdentityError {
    #[error("package callable package id must not be empty")]
    EmptyPackageId,
    #[error("package callable module path must not be empty")]
    EmptyModulePath,
    #[error("package callable executable symbol must not be empty")]
    EmptyExecutableSymbol,
    #[error("package callable public path must not be empty")]
    EmptyPublicPath,
    #[error("implementation method `{symbol}` has no receiver owner")]
    ImplMethodMissingOwner { symbol: String },
    #[error("implementation method `{symbol}` has no method name")]
    ImplMethodMissingMethod { symbol: String },
    #[error("implementation method `{symbol}` has malformed generic owner")]
    MalformedGenericOwner { symbol: String },
}

/// Returns the canonical source path used to identify an implementation callable.
pub fn canonical_implementation_callable_source_path(
    module_path: &str,
    executable_symbol: &str,
    kind: ImplementationCallableKind,
) -> Result<String, PackageCallableIdentityError> {
    if module_path.is_empty() {
        return Err(PackageCallableIdentityError::EmptyModulePath);
    }
    if executable_symbol.is_empty() {
        return Err(PackageCallableIdentityError::EmptyExecutableSymbol);
    }

    let module_prefix = format!("{module_path}.");
    let top_level_name = executable_symbol
        .strip_prefix(&module_prefix)
        .unwrap_or(executable_symbol);
    if top_level_name.is_empty() {
        return Err(PackageCallableIdentityError::EmptyExecutableSymbol);
    }
    let top_level_name = match kind {
        ImplementationCallableKind::Function => top_level_name.to_string(),
        ImplementationCallableKind::ImplMethod => {
            canonical_impl_method_source_name(top_level_name)?
        }
    };

    Ok(format!("{module_path}.{top_level_name}"))
}

/// Constructs the canonical identity for a package implementation callable.
pub fn implementation_package_callable_id(
    package_id: &str,
    module_path: &str,
    executable_symbol: &str,
    kind: ImplementationCallableKind,
) -> Result<PackageCallableId, PackageCallableIdentityError> {
    validate_package_id(package_id)?;
    let source_path =
        canonical_implementation_callable_source_path(module_path, executable_symbol, kind)?;
    Ok(PackageCallableId::new(format!(
        "pkg-callable:{package_id}:top-level:{source_path}"
    )))
}

/// Constructs the canonical identity for a package callable exposed at a public path.
pub fn public_package_callable_id(
    package_id: &str,
    public_path: &str,
) -> Result<PackageCallableId, PackageCallableIdentityError> {
    validate_package_id(package_id)?;
    if public_path.is_empty() {
        return Err(PackageCallableIdentityError::EmptyPublicPath);
    }
    Ok(PackageCallableId::new(format!(
        "pkg-callable:{package_id}:{public_path}"
    )))
}

fn validate_package_id(package_id: &str) -> Result<(), PackageCallableIdentityError> {
    if package_id.is_empty() {
        return Err(PackageCallableIdentityError::EmptyPackageId);
    }
    Ok(())
}

fn canonical_impl_method_source_name(
    top_level_name: &str,
) -> Result<String, PackageCallableIdentityError> {
    let (owner, method) = top_level_name.rsplit_once('.').ok_or_else(|| {
        PackageCallableIdentityError::ImplMethodMissingOwner {
            symbol: top_level_name.to_string(),
        }
    })?;
    if method.is_empty() {
        return Err(PackageCallableIdentityError::ImplMethodMissingMethod {
            symbol: top_level_name.to_string(),
        });
    }
    let owner = match owner.find('<') {
        Some(start) if owner.ends_with('>') && start > 0 => &owner[..start],
        Some(_) => {
            return Err(PackageCallableIdentityError::MalformedGenericOwner {
                symbol: top_level_name.to_string(),
            });
        }
        None => owner,
    };
    if owner.is_empty() {
        return Err(PackageCallableIdentityError::ImplMethodMissingOwner {
            symbol: top_level_name.to_string(),
        });
    }
    Ok(format!("{owner}.{method}"))
}

#[cfg(test)]
mod tests {
    use super::{
        canonical_implementation_callable_source_path, implementation_package_callable_id,
        public_package_callable_id, ImplementationCallableKind, PackageCallableIdentityError,
    };

    #[test]
    fn canonicalizes_function_source_path_and_identity() {
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.run",
                ImplementationCallableKind::Function,
            ),
            Ok("api.run".to_string())
        );
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.api.run",
                ImplementationCallableKind::Function,
            ),
            Ok("api.api.run".to_string()),
            "exactly one module prefix must be removed"
        );
        assert_eq!(
            implementation_package_callable_id(
                "example.pkg",
                "api",
                "api.run",
                ImplementationCallableKind::Function,
            )
            .unwrap()
            .as_str(),
            "pkg-callable:example.pkg:top-level:api.run"
        );
    }

    #[test]
    fn canonicalizes_generic_impl_owner_without_changing_projection_bytes() {
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.Worker<T>.handle",
                ImplementationCallableKind::ImplMethod,
            ),
            Ok("api.Worker.handle".to_string())
        );
        assert_eq!(
            implementation_package_callable_id(
                "example.pkg",
                "api",
                "api.Worker<T>.handle",
                ImplementationCallableKind::ImplMethod,
            )
            .unwrap()
            .as_str(),
            "pkg-callable:example.pkg:top-level:api.Worker.handle"
        );
    }

    #[test]
    fn rejects_empty_identity_components() {
        assert_eq!(
            implementation_package_callable_id(
                "",
                "api",
                "api.run",
                ImplementationCallableKind::Function,
            ),
            Err(PackageCallableIdentityError::EmptyPackageId)
        );
        assert_eq!(
            canonical_implementation_callable_source_path(
                "",
                "api.run",
                ImplementationCallableKind::Function,
            ),
            Err(PackageCallableIdentityError::EmptyModulePath)
        );
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "",
                ImplementationCallableKind::Function,
            ),
            Err(PackageCallableIdentityError::EmptyExecutableSymbol)
        );
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.",
                ImplementationCallableKind::Function,
            ),
            Err(PackageCallableIdentityError::EmptyExecutableSymbol)
        );
        assert_eq!(
            public_package_callable_id("example.pkg", ""),
            Err(PackageCallableIdentityError::EmptyPublicPath)
        );
    }

    #[test]
    fn rejects_malformed_impl_method_symbols() {
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.handle",
                ImplementationCallableKind::ImplMethod,
            ),
            Err(PackageCallableIdentityError::ImplMethodMissingOwner {
                symbol: "handle".to_string(),
            })
        );
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.Worker.",
                ImplementationCallableKind::ImplMethod,
            ),
            Err(PackageCallableIdentityError::ImplMethodMissingMethod {
                symbol: "Worker.".to_string(),
            })
        );
        assert_eq!(
            canonical_implementation_callable_source_path(
                "api",
                "api.Worker<T.handle",
                ImplementationCallableKind::ImplMethod,
            ),
            Err(PackageCallableIdentityError::MalformedGenericOwner {
                symbol: "Worker<T.handle".to_string(),
            })
        );
    }

    #[test]
    fn public_identity_matches_projection_bytes() {
        assert_eq!(
            public_package_callable_id("example.pkg", "run")
                .unwrap()
                .as_str(),
            "pkg-callable:example.pkg:run"
        );
        assert_eq!(
            public_package_callable_id("example.pkg", "worker.handle")
                .unwrap()
                .as_str(),
            "pkg-callable:example.pkg:worker.handle"
        );
    }
}
