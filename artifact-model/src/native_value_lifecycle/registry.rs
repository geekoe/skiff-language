use std::{collections::BTreeMap, sync::LazyLock};

use sha2::Digest;

use crate::{PackageRefIr, TypeRefIr};

use super::contract::*;

pub const NATIVE_VALUE_LIFECYCLE_REGISTRY_ID: &str = "skiff-native-value-lifecycle";
pub const NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION: &str = "skiff-native-value-lifecycle-v1";
pub const NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT: &str =
    "4210f98d95be4eedefbe0f54520a27242d51de7b591b307fc5faa54dd6c0eda7";
pub const MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS: usize = 64;

#[derive(Debug, Clone)]
pub struct NativeValueLifecycleRegistry {
    identity: NativeValueLifecycleRegistryIdentity,
    entries: Vec<NativeValueLifecycleEntry>,
    adapters: BTreeMap<String, NativeValueLifecycleAdapter>,
}

impl NativeValueLifecycleRegistry {
    pub fn new(
        registry_id: impl Into<String>,
        version: impl Into<String>,
        mut entries: Vec<NativeValueLifecycleEntry>,
    ) -> Result<Self, NativeValueLifecycleRegistryError> {
        let registry_id = registry_id.into();
        let version = version.into();
        if registry_id.is_empty() {
            return Err(NativeValueLifecycleRegistryError::EmptyRegistryId);
        }
        if version.is_empty() {
            return Err(NativeValueLifecycleRegistryError::EmptyVersion);
        }
        entries.sort_by(compare_entries);
        let adapters = validate_entries(&entries)?;
        let fingerprint = registry_fingerprint(&registry_id, &version, &entries)?;
        Ok(Self {
            identity: NativeValueLifecycleRegistryIdentity {
                registry_id,
                version,
                fingerprint,
            },
            entries,
            adapters,
        })
    }

    pub fn identity(&self) -> &NativeValueLifecycleRegistryIdentity {
        &self.identity
    }

    pub fn entries(&self) -> &[NativeValueLifecycleEntry] {
        &self.entries
    }

    pub fn adapter(&self, binding_key: &str) -> Option<&NativeValueLifecycleAdapter> {
        self.adapters.get(binding_key)
    }

    pub fn lookup(
        &self,
        ty: &TypeRefIr,
    ) -> Result<NativeValueLifecycleResolution, NativeValueLifecycleLookupError> {
        self.lookup_at_depth(ty, 1)
    }

    fn lookup_at_depth(
        &self,
        ty: &TypeRefIr,
        depth: usize,
    ) -> Result<NativeValueLifecycleResolution, NativeValueLifecycleLookupError> {
        if depth > MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS {
            return Err(NativeValueLifecycleLookupError::NestingLimit);
        }
        let (constructor, arguments) = decompose_type(ty)?;
        let mut expected = self
            .entries
            .iter()
            .filter(|entry| entry.pattern.constructor == constructor)
            .map(|entry| entry.pattern.argument_policies.len())
            .collect::<Vec<_>>();
        if expected.is_empty() {
            return Err(NativeValueLifecycleLookupError::Missing { constructor });
        }
        expected.sort_unstable();
        let Some(entry) = self.entries.iter().find(|entry| {
            entry.pattern.constructor == constructor
                && entry.pattern.argument_policies.len() == arguments.len()
        }) else {
            return Err(NativeValueLifecycleLookupError::ArityMismatch {
                constructor,
                expected,
                actual: arguments.len(),
            });
        };
        let mut resolved_arguments = vec![None; arguments.len()];
        for (index, (argument, policy)) in arguments
            .iter()
            .zip(&entry.pattern.argument_policies)
            .enumerate()
        {
            match policy {
                NativeValueArgumentPolicy::Phantom => {}
                NativeValueArgumentPolicy::RequireSnapshotShare => {
                    let lifecycle =
                        self.lookup_at_depth(argument, depth + 1)
                            .map_err(|source| NativeValueLifecycleLookupError::Argument {
                                index,
                                source: Box::new(source),
                            })?;
                    if lifecycle.lifecycle.kind() != NativeValueLifecycleKind::SnapshotShare {
                        return Err(NativeValueLifecycleLookupError::ArgumentPolicyMismatch {
                            index,
                            policy: *policy,
                            actual: lifecycle.lifecycle.kind(),
                        });
                    }
                    resolved_arguments[index] = Some(lifecycle.lifecycle);
                }
            }
        }
        Ok(NativeValueLifecycleResolution {
            lifecycle: instantiate_template(&entry.lifecycle, &resolved_arguments)?,
            embedding: entry.embedding,
        })
    }
}

fn decompose_type(
    ty: &TypeRefIr,
) -> Result<(NativeValueTypeConstructor, &[TypeRefIr]), NativeValueLifecycleLookupError> {
    match ty {
        TypeRefIr::Builtin { name, args } => Ok((
            NativeValueTypeConstructor::Builtin { name: name.clone() },
            args,
        )),
        TypeRefIr::PackageSymbol { symbol } => Ok((package_constructor(symbol)?, &[])),
        TypeRefIr::AppliedNominal { base, arguments } => match base {
            crate::NominalTypeRefBaseIr::PackageSymbol { symbol } => {
                Ok((package_constructor(symbol)?, arguments))
            }
            _ => Err(NativeValueLifecycleLookupError::UnsupportedType {
                message: "only PackageSymbol applied nominal constructors are registry-owned"
                    .to_string(),
            }),
        },
        _ => Err(NativeValueLifecycleLookupError::UnsupportedType {
            message: "type has no Builtin/PackageSymbol native constructor".to_string(),
        }),
    }
}

fn package_constructor(
    symbol: &crate::PackageSymbolRef,
) -> Result<NativeValueTypeConstructor, NativeValueLifecycleLookupError> {
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        return Err(NativeValueLifecycleLookupError::UnsupportedType {
            message: "PackageSymbol lifecycle lookup requires a resolved package id".to_string(),
        });
    };
    let abi_identity = symbol
        .abi_expectation
        .as_deref()
        .filter(|identity| !identity.is_empty())
        .ok_or_else(|| NativeValueLifecycleLookupError::UnsupportedType {
            message: "PackageSymbol lifecycle lookup requires an exact ABI identity".to_string(),
        })?;
    Ok(NativeValueTypeConstructor::PackageSymbol {
        package_id: package_id.clone(),
        symbol_path: symbol.symbol_path.clone(),
        abi_identity: abi_identity.to_string(),
    })
}

fn instantiate_template(
    template: &NativeValueLifecycleTemplate,
    arguments: &[Option<NativeValueLifecycleConcrete>],
) -> Result<NativeValueLifecycleConcrete, NativeValueLifecycleLookupError> {
    Ok(match template {
        NativeValueLifecycleTemplate::SnapshotShare { drop } => {
            NativeValueLifecycleConcrete::SnapshotShare { drop: drop.clone() }
        }
        NativeValueLifecycleTemplate::MoveOnly { drop } => {
            NativeValueLifecycleConcrete::MoveOnly { drop: drop.clone() }
        }
        NativeValueLifecycleTemplate::AffineResource { drop } => {
            NativeValueLifecycleConcrete::AffineResource { drop: drop.clone() }
        }
        NativeValueLifecycleTemplate::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter: clone_adapter.clone(),
            drop: drop.clone(),
        },
        NativeValueLifecycleTemplate::FromType { argument_index } => arguments
            .get(*argument_index as usize)
            .and_then(Clone::clone)
            .ok_or_else(|| NativeValueLifecycleLookupError::UnsupportedType {
                message: format!(
                    "validated FromType argument {argument_index} was not instantiated"
                ),
            })?,
    })
}

fn validate_entries(
    entries: &[NativeValueLifecycleEntry],
) -> Result<BTreeMap<String, NativeValueLifecycleAdapter>, NativeValueLifecycleRegistryError> {
    let mut patterns = BTreeMap::new();
    let mut adapters = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        validate_pattern(index, &entry.pattern)?;
        let key = (
            entry.pattern.constructor.clone(),
            entry.pattern.argument_policies.len(),
        );
        if patterns.insert(key.clone(), index).is_some() {
            return Err(
                NativeValueLifecycleRegistryError::DuplicateConstructorArity {
                    constructor: key.0,
                    arity: key.1,
                },
            );
        }
        validate_template(index, &entry.pattern, &entry.lifecycle, &mut adapters)?;
    }
    Ok(adapters)
}

fn compare_entries(
    left: &NativeValueLifecycleEntry,
    right: &NativeValueLifecycleEntry,
) -> std::cmp::Ordering {
    left.pattern
        .constructor
        .cmp(&right.pattern.constructor)
        .then_with(|| {
            left.pattern
                .argument_policies
                .len()
                .cmp(&right.pattern.argument_policies.len())
        })
}

fn validate_pattern(
    entry: usize,
    pattern: &NativeValueTypePattern,
) -> Result<(), NativeValueLifecycleRegistryError> {
    match &pattern.constructor {
        NativeValueTypeConstructor::Builtin { name } if name.is_empty() => {
            return Err(NativeValueLifecycleRegistryError::EmptyConstructorField {
                entry,
                field: "name",
            });
        }
        NativeValueTypeConstructor::PackageSymbol {
            package_id,
            symbol_path,
            abi_identity,
        } if package_id.is_empty() || symbol_path.is_empty() || abi_identity.is_empty() => {
            return Err(NativeValueLifecycleRegistryError::EmptyConstructorField {
                entry,
                field: if package_id.is_empty() {
                    "packageId"
                } else if symbol_path.is_empty() {
                    "symbolPath"
                } else {
                    "abiIdentity"
                },
            });
        }
        _ => {}
    }
    if pattern.argument_policies.len() > MAX_NATIVE_VALUE_LIFECYCLE_ARGUMENTS {
        return Err(NativeValueLifecycleRegistryError::TooManyArguments {
            entry,
            actual: pattern.argument_policies.len(),
        });
    }
    Ok(())
}

fn validate_template(
    entry: usize,
    pattern: &NativeValueTypePattern,
    template: &NativeValueLifecycleTemplate,
    adapters: &mut BTreeMap<String, NativeValueLifecycleAdapter>,
) -> Result<(), NativeValueLifecycleRegistryError> {
    match template {
        NativeValueLifecycleTemplate::SnapshotShare { drop }
        | NativeValueLifecycleTemplate::MoveOnly { drop } => {
            validate_value_drop(entry, drop, adapters)?;
        }
        NativeValueLifecycleTemplate::AffineResource { drop } => {
            validate_resource_drop(entry, drop, adapters)?;
        }
        NativeValueLifecycleTemplate::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => {
            validate_adapter(
                entry,
                clone_adapter,
                NativeValueAdapterRole::CloneLease,
                adapters,
            )?;
            validate_resource_drop(entry, drop, adapters)?;
        }
        NativeValueLifecycleTemplate::FromType { argument_index } => {
            let Some(policy) = pattern.argument_policies.get(*argument_index as usize) else {
                return Err(NativeValueLifecycleRegistryError::InvalidFromType {
                    entry,
                    argument_index: *argument_index,
                    message: "argument index is out of bounds",
                });
            };
            if *policy != NativeValueArgumentPolicy::RequireSnapshotShare {
                return Err(NativeValueLifecycleRegistryError::InvalidFromType {
                    entry,
                    argument_index: *argument_index,
                    message: "FromType may not reference a phantom argument",
                });
            }
        }
    }
    Ok(())
}

fn validate_value_drop(
    entry: usize,
    drop: &NativeValueDropPlan,
    adapters: &mut BTreeMap<String, NativeValueLifecycleAdapter>,
) -> Result<(), NativeValueLifecycleRegistryError> {
    if let NativeValueDropPlan::NativeAdapter { adapter } = drop {
        validate_adapter(entry, adapter, NativeValueAdapterRole::ValueDrop, adapters)?;
    }
    Ok(())
}

fn validate_resource_drop(
    entry: usize,
    drop: &NativeResourceDropPlan,
    adapters: &mut BTreeMap<String, NativeValueLifecycleAdapter>,
) -> Result<(), NativeValueLifecycleRegistryError> {
    if let NativeResourceDropPlan::NativeAdapter { adapter } = drop {
        validate_adapter(
            entry,
            adapter,
            NativeValueAdapterRole::ResourceDrop,
            adapters,
        )?;
    }
    Ok(())
}

fn validate_adapter(
    entry: usize,
    adapter: &NativeValueLifecycleAdapter,
    expected_role: NativeValueAdapterRole,
    adapters: &mut BTreeMap<String, NativeValueLifecycleAdapter>,
) -> Result<(), NativeValueLifecycleRegistryError> {
    if adapter.binding_key.is_empty()
        || adapter.binding_key.chars().any(char::is_whitespace)
        || adapter.binding_key.chars().any(char::is_control)
        || adapter.abi_version == 0
        || adapter.role != expected_role
    {
        return Err(NativeValueLifecycleRegistryError::InvalidAdapter {
            entry,
            binding_key: adapter.binding_key.clone(),
            message: if adapter.binding_key.is_empty() {
                "binding key is empty"
            } else if adapter.binding_key.chars().any(char::is_whitespace) {
                "binding key contains whitespace"
            } else if adapter.binding_key.chars().any(char::is_control) {
                "binding key contains a control character"
            } else if adapter.abi_version == 0 {
                "ABI version must be non-zero"
            } else {
                "adapter role does not match its plan position"
            },
        });
    }
    if let Some(previous) = adapters.get(&adapter.binding_key) {
        if previous != adapter {
            return Err(NativeValueLifecycleRegistryError::ConflictingAdapter {
                binding_key: adapter.binding_key.clone(),
                first_role: previous.role,
                first_abi_version: previous.abi_version,
                next_role: adapter.role,
                next_abi_version: adapter.abi_version,
            });
        }
    } else {
        adapters.insert(adapter.binding_key.clone(), adapter.clone());
    }
    Ok(())
}

fn registry_fingerprint(
    registry_id: &str,
    version: &str,
    entries: &[NativeValueLifecycleEntry],
) -> Result<String, NativeValueLifecycleRegistryError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Projection<'a> {
        registry_id: &'a str,
        version: &'a str,
        entries: Vec<&'a NativeValueLifecycleEntry>,
    }

    let mut sorted = entries.iter().collect::<Vec<_>>();
    sorted.sort_by(|left, right| compare_entries(left, right));
    let bytes = skiff_canonical_json::canonical_json_bytes(&Projection {
        registry_id,
        version,
        entries: sorted,
    })
    .map_err(
        |error| NativeValueLifecycleRegistryError::FingerprintProjection {
            message: error.to_string(),
        },
    )?;
    Ok(hex::encode(sha2::Sha256::digest(bytes)))
}

pub(super) fn builtin_entry(
    name: &str,
    argument_policies: Vec<NativeValueArgumentPolicy>,
    lifecycle: NativeValueLifecycleTemplate,
    embedding: NativeValueEmbedding,
) -> NativeValueLifecycleEntry {
    NativeValueLifecycleEntry {
        pattern: NativeValueTypePattern {
            constructor: NativeValueTypeConstructor::Builtin {
                name: name.to_string(),
            },
            argument_policies,
        },
        lifecycle,
        embedding,
    }
}

fn initial_entries() -> Vec<NativeValueLifecycleEntry> {
    let scalar = || NativeValueLifecycleTemplate::SnapshotShare {
        drop: NativeValueDropPlan::Trivial,
    };
    let mut entries = ["null", "bool", "number", "integer", "Date"]
        .into_iter()
        .map(|name| builtin_entry(name, Vec::new(), scalar(), NativeValueEmbedding::Ordinary))
        .collect::<Vec<_>>();
    entries.push(builtin_entry(
        "Stream",
        vec![NativeValueArgumentPolicy::RequireSnapshotShare],
        NativeValueLifecycleTemplate::AffineResource {
            drop: NativeResourceDropPlan::ResourceTableRelease,
        },
        NativeValueEmbedding::Forbidden,
    ));
    entries
}

pub static NATIVE_VALUE_LIFECYCLE_REGISTRY: LazyLock<NativeValueLifecycleRegistry> =
    LazyLock::new(|| {
        let registry = NativeValueLifecycleRegistry::new(
            NATIVE_VALUE_LIFECYCLE_REGISTRY_ID,
            NATIVE_VALUE_LIFECYCLE_REGISTRY_VERSION,
            initial_entries(),
        )
        .expect("built-in native lifecycle registry is valid");
        assert_eq!(
            registry.identity().fingerprint,
            NATIVE_VALUE_LIFECYCLE_REGISTRY_FINGERPRINT,
            "built-in native lifecycle registry fingerprint changed without a version bump"
        );
        registry
    });

pub fn native_value_lifecycle_registry() -> &'static NativeValueLifecycleRegistry {
    &NATIVE_VALUE_LIFECYCLE_REGISTRY
}

pub fn native_value_lifecycle_registry_identity() -> &'static NativeValueLifecycleRegistryIdentity {
    NATIVE_VALUE_LIFECYCLE_REGISTRY.identity()
}
