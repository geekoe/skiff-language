use skiff_artifact_model::PackageUnit;

use super::{package::*, *};

mod canonicalization;
mod inclusion;
mod recoverable_support;

fn identities(unit: &PackageUnit) -> (String, String) {
    (local_abi_identity(unit), build_identity(unit))
}

fn assert_both_change(base: &PackageUnit, changed: &PackageUnit) {
    let (base_abi, base_build) = identities(base);
    let (changed_abi, changed_build) = identities(changed);
    assert_ne!(base_abi, changed_abi);
    assert_ne!(base_build, changed_build);
}

fn assert_build_only_change(base: &PackageUnit, changed: &PackageUnit) {
    let (base_abi, base_build) = identities(base);
    let (changed_abi, changed_build) = identities(changed);
    assert_eq!(base_abi, changed_abi);
    assert_ne!(base_build, changed_build);
}

fn file_identity(seed: char) -> String {
    format!("skiff-file-ir-v3:sha256:{}", seed.to_string().repeat(64))
}
