use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use skiff_artifact_model::NativeTypeExprDef;
use skiff_compiler_input_model::{CompilerRawSourceFile, CompilerSourceRole, RawSourceFileMeta};
use skiff_trusted_registry_contract::{
    TRUSTED_REGISTRY_NATIVE_SIGNATURES, TRUSTED_REGISTRY_PACKAGE_ID,
};

use crate::{
    package_sources::{PackageSourceVisibility, RawPackagePublicationSources},
    CompilerPlatformPackageAuthority, InputAssemblyError,
};

pub fn trusted_registry_native_sources(
    authority: &CompilerPlatformPackageAuthority,
) -> Result<RawPackagePublicationSources, InputAssemblyError> {
    assert_eq!(authority.package_id(), TRUSTED_REGISTRY_PACKAGE_ID);
    let root = authority
        .platform_root()
        .join(".compiler-platform/skiff.run/registry");
    let mut files = Vec::new();
    let mut visibility = BTreeMap::new();
    let types = TRUSTED_REGISTRY_NATIVE_SIGNATURES
        .iter()
        .flat_map(|signature| {
            signature
                .params
                .iter()
                .chain(std::iter::once(&signature.return_type))
        })
        .map(registry_type_name)
        .collect::<BTreeSet<_>>();
    push_source(
        &mut files,
        &mut visibility,
        "types.skiff",
        "skiff.registry",
        types
            .into_iter()
            .map(|name| format!("native type {name}\n"))
            .collect(),
    );

    for signature in TRUSTED_REGISTRY_NATIVE_SIGNATURES {
        let (module, symbol) = signature.target.rsplit_once('.').unwrap();
        let relative_path = format!(
            "{}.skiff",
            module
                .strip_prefix("skiff.registry.")
                .unwrap()
                .replace('.', "/")
        );
        push_source(
            &mut files,
            &mut visibility,
            &relative_path,
            module,
            format!(
                "native function {symbol}(request: {}) -> {}\n",
                registry_type_name(&signature.params[0]),
                registry_type_name(&signature.return_type)
            ),
        );
    }
    RawPackagePublicationSources::with_visibility(root, files, visibility)
}

fn registry_type_name(ty: &NativeTypeExprDef) -> &str {
    match ty {
        NativeTypeExprDef::Builtin(name) => name.strip_prefix("skiff.registry.").unwrap(),
        _ => panic!("trusted registry signatures must use builtin DTO types"),
    }
}

fn push_source(
    files: &mut Vec<CompilerRawSourceFile>,
    visibility: &mut BTreeMap<PathBuf, PackageSourceVisibility>,
    relative_path: &str,
    module_path: &str,
    text: String,
) {
    let relative_path = PathBuf::from(relative_path);
    visibility.insert(
        relative_path.clone(),
        PackageSourceVisibility::Export {
            public_module_path: module_path.to_string(),
        },
    );
    if let Some(existing) = files
        .iter_mut()
        .find(|source| source.meta.relative_path == relative_path)
    {
        existing.text.push_str(&text);
        return;
    }
    files.push(CompilerRawSourceFile {
        meta: RawSourceFileMeta {
            relative_path,
            module_path: module_path.to_string(),
            is_test_file: false,
            is_generated: true,
        },
        text,
        role: CompilerSourceRole::Package,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CompilerPlatformSources;

    #[test]
    fn authority_injects_exact_declaration_only_slice() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let platform = CompilerPlatformSources::new(&root).unwrap();
        let snapshot_before = platform.prelude_registry_snapshot().unwrap();
        let authority = platform.trusted_registry_package_authority().unwrap();
        let sources = trusted_registry_native_sources(&authority).unwrap();
        let snapshot_after = platform.prelude_registry_snapshot().unwrap();
        assert_eq!(authority.package_id(), TRUSTED_REGISTRY_PACKAGE_ID);
        assert_eq!(
            snapshot_before, snapshot_after,
            "registry authority must not mutate std/prelude inputs"
        );
        assert!(sources
            .files()
            .iter()
            .all(|source| source.meta.is_generated));
        assert_eq!(
            sources
                .files()
                .iter()
                .map(|source| source.text.matches("native function ").count())
                .sum::<usize>(),
            21
        );
        assert!(sources.files().iter().all(|source| {
            !source.text.contains('{')
                && !source.text.contains("provider ")
                && !source.text.contains("function prepare")
                && !source.text.contains("function commit")
                && !source.text.contains("function abort")
        }));
        for signature in TRUSTED_REGISTRY_NATIVE_SIGNATURES {
            let (module, symbol) = signature.target.rsplit_once('.').unwrap();
            let source = sources
                .files()
                .iter()
                .find(|source| source.meta.module_path == module)
                .expect("canonical target module must be injected");
            assert!(
                source.text.contains(&format!("native function {symbol}(")),
                "missing generated declaration for {}",
                signature.target
            );
        }
    }
}
