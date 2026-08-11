use std::collections::BTreeMap;

use skiff_artifact_model::{
    native_value_lifecycle_registry, NativeResourceDropPlan, NativeValueDropPlan,
    NativeValueEmbedding, NativeValueLifecycleConcrete, NativeValueLifecycleLookupError,
    NativeValueLifecycleResolution, NominalTypeRefBaseIr, ResourceDropPlan, TypeRefIr,
    ValueDropPlan, ValueTransferPlan,
};
use skiff_compiler_lowering::mir::{MirSlot, MirUnit};

use super::{
    inputs::{canonical_function_key, is_void},
    BytecodeEmissionError,
};

/// Derives explicit transfer plans from MIR and the pinned native lifecycle
/// registry.
///
/// Constants retain their owner-qualified `FromType` plan. Function slots and
/// results are resolved to concrete lifecycle plans from the pinned native
/// registry. Ordinary snapshot values use `SnapshotShare`; exact authoritative
/// `Stream<T>` endpoints use their affine resource plan. Missing or
/// non-snapshot lifecycle facts fail closed instead of being inferred.
pub fn derive_bytecode_value_transfer_plans(
    units: &[MirUnit],
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    let mut functions = BTreeMap::new();
    for unit in units {
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            let mut slot_plans = Vec::with_capacity(function.slots.len());
            for slot in &function.slots {
                let ty = slot
                    .ty
                    .as_ref()
                    .ok_or_else(|| unsupported_slot_type(&function_key, slot))?;
                slot_plans.push(concrete_value_plan(
                    units,
                    &unit.module_path,
                    &function_key,
                    &format!("slot `{}`", slot.name),
                    ty,
                )?);
            }
            let result_plans =
                if is_void(&function.return_type) || function.stream_result.is_some() {
                Vec::new()
            } else {
                vec![concrete_value_plan(
                    units,
                    &unit.module_path,
                    &function_key,
                    "return value",
                    &function.return_type,
                )?]
            };
            functions.insert(
                function_key,
                FunctionValueTransferPlans {
                    slot_plans,
                    result_plans,
                },
            );
        }
    }
    let constants = units
        .iter()
        .flat_map(|unit| &unit.constants)
        .map(|constant| {
            (
                constant.symbol.clone(),
                ValueTransferPlan::FromType {
                    ty: constant.ty.clone(),
                },
            )
        })
        .collect();
    Ok(BytecodeValueTransferPlans::new(functions, constants))
}

fn concrete_value_plan(
    units: &[MirUnit],
    module_path: &str,
    function_key: &str,
    location: &str,
    ty: &skiff_artifact_model::TypeRefIr,
) -> Result<ValueTransferPlan, BytecodeEmissionError> {
    if is_record_aggregate(units, module_path, ty)? {
        return Ok(snapshot_release_plan());
    }
    if is_never_type(ty) {
        return Ok(ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::Trivial,
        });
    }
    let resolution = match native_value_lifecycle_registry().lookup(ty) {
        Ok(resolution) => resolution,
        Err(NativeValueLifecycleLookupError::Missing { .. }) if is_package_symbol_type(ty) => {
            return Ok(snapshot_release_plan());
        }
        Err(error) => {
            return Err(BytecodeEmissionError::UnsupportedConstruct {
                function_key: function_key.to_string(),
                construct: "value lifecycle lookup",
                location: format!(" {location}: {error}"),
            });
        }
    };
    concrete_lifecycle_plan(function_key, location, ty, resolution)
}

fn snapshot_release_plan() -> ValueTransferPlan {
    ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    }
}

fn is_never_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty()
    )
}

fn is_package_symbol_type(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::PackageSymbol { .. })
        || matches!(
            ty,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol { .. },
                ..
            }
        )
}

fn concrete_lifecycle_plan(
    function_key: &str,
    location: &str,
    ty: &TypeRefIr,
    resolution: NativeValueLifecycleResolution,
) -> Result<ValueTransferPlan, BytecodeEmissionError> {
    match &resolution.lifecycle {
        NativeValueLifecycleConcrete::SnapshotShare { drop } => {
            let drop = match drop {
                NativeValueDropPlan::Trivial => ValueDropPlan::Trivial,
                NativeValueDropPlan::SnapshotRelease => ValueDropPlan::SnapshotRelease,
                NativeValueDropPlan::NativeAdapter { .. } => {
                    return Err(BytecodeEmissionError::UnsupportedConstruct {
                        function_key: function_key.to_string(),
                        construct: "native adapter value drop",
                        location: format!(" {location}"),
                    });
                }
            };
            Ok(ValueTransferPlan::SnapshotShare { drop })
        }
        NativeValueLifecycleConcrete::AffineResource { drop } => {
            if !is_authoritative_stream(ty, &resolution) {
                return Err(BytecodeEmissionError::UnsupportedConstruct {
                    function_key: function_key.to_string(),
                    construct: "non-Stream affine value lifecycle",
                    location: format!(" {location}"),
                });
            }
            let drop = match drop {
                NativeResourceDropPlan::ResourceTableRelease => {
                    ResourceDropPlan::ResourceTableRelease
                }
                NativeResourceDropPlan::NativeAdapter { .. } => {
                    return Err(BytecodeEmissionError::UnsupportedConstruct {
                        function_key: function_key.to_string(),
                        construct: "native stream resource drop",
                        location: format!(" {location}"),
                    });
                }
            };
            Ok(ValueTransferPlan::AffineResource { drop })
        }
        NativeValueLifecycleConcrete::MoveOnly { .. }
        | NativeValueLifecycleConcrete::ExplicitCloneLease { .. } => {
            Err(BytecodeEmissionError::UnsupportedConstruct {
                function_key: function_key.to_string(),
                construct: "non-snapshot value lifecycle",
                location: format!(" {location}"),
            })
        }
    }
}

fn is_authoritative_stream(
    ty: &TypeRefIr,
    resolution: &NativeValueLifecycleResolution,
) -> bool {
    matches!(
        ty,
        skiff_artifact_model::TypeRefIr::Builtin { name, args }
            if name == "Stream" && args.len() == 1
    ) && resolution.embedding == NativeValueEmbedding::Forbidden
}

fn is_record_aggregate(
    units: &[MirUnit],
    module_path: &str,
    ty: &skiff_artifact_model::TypeRefIr,
) -> Result<bool, BytecodeEmissionError> {
    match ty {
        skiff_artifact_model::TypeRefIr::Record { .. } => Ok(true),
        skiff_artifact_model::TypeRefIr::LocalType { type_index } => {
            let unit = units
                .iter()
                .find(|unit| unit.module_path == module_path)
                .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
                    context: format!("value lifecycle plan for module `{module_path}`"),
                    message: "owning MIR unit disappeared".to_string(),
                })?;
            record_declaration(unit, *type_index)
        }
        skiff_artifact_model::TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => {
            let unit = units
                .iter()
                .find(|unit| unit.module_path == *module_path)
                .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
                    context: format!("value lifecycle plan for publication module `{module_path}`"),
                    message: "publication MIR unit disappeared".to_string(),
                })?;
            record_declaration(unit, *type_index)
        }
        skiff_artifact_model::TypeRefIr::AppliedNominal {
            base: skiff_artifact_model::NominalTypeRefBaseIr::LocalType { type_index },
            arguments,
        } if arguments.is_empty() => {
            let unit = units
                .iter()
                .find(|unit| unit.module_path == module_path)
                .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
                    context: format!("value lifecycle plan for module `{module_path}`"),
                    message: "owning MIR unit disappeared".to_string(),
                })?;
            record_declaration(unit, *type_index)
        }
        skiff_artifact_model::TypeRefIr::AppliedNominal {
            base:
                skiff_artifact_model::NominalTypeRefBaseIr::PublicationType {
                    module_path,
                    type_index,
                },
            arguments,
        } if arguments.is_empty() => {
            let unit = units
                .iter()
                .find(|unit| unit.module_path == *module_path)
                .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
                    context: format!("value lifecycle plan for publication module `{module_path}`"),
                    message: "publication MIR unit disappeared".to_string(),
                })?;
            record_declaration(unit, *type_index)
        }
        _ => Ok(false),
    }
}

fn record_declaration(unit: &MirUnit, type_index: u32) -> Result<bool, BytecodeEmissionError> {
    let declaration = unit.type_table.get(type_index as usize).ok_or_else(|| {
        BytecodeEmissionError::MissingLocalType {
            module_path: unit.module_path.clone(),
            location: "value lifecycle record shape".to_string(),
            type_index,
            type_count: unit.type_table.len(),
        }
    })?;
    if !declaration.type_params.is_empty() {
        return Ok(false);
    }
    Ok(matches!(
        declaration.descriptor,
        skiff_artifact_model::TypeDescriptorIr::Record { .. }
    ))
}

fn unsupported_slot_type(function_key: &str, slot: &MirSlot) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "slot without an exact type",
        location: format!(" slot `{}`", slot.name),
    }
}

/// Explicit source-owned transfer facts for every bytecode function and
/// implementation constant.
///
/// The emitter never derives a plan from a MIR slot kind or type. Function
/// Keys use the canonical `"{module_path}::{declaration}"` image spelling:
/// the emitter first requires MIR `symbol` to start with the exact
/// `"{module_path}."` owner prefix, strips that prefix once, and rejects an
/// empty declaration. It never appends the still-qualified MIR symbol to the
/// module a second time. This map must cover that canonical MIR function set
/// exactly. Constant keys use the canonical `"{module_path}.{declaration}"`
/// spelling retained by [`skiff_compiler_lowering::mir::MirConst`].
#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeValueTransferPlans {
    functions: BTreeMap<String, FunctionValueTransferPlans>,
    constants: BTreeMap<String, ValueTransferPlan>,
}

impl BytecodeValueTransferPlans {
    /// Creates one explicit, package-wide plan bundle.
    ///
    /// Both maps are exact-coverage inputs. Emission rejects missing and
    /// extra rows; this constructor never fills a plan from a type or slot
    /// kind.
    pub fn new(
        functions: BTreeMap<String, FunctionValueTransferPlans>,
        constants: BTreeMap<String, ValueTransferPlan>,
    ) -> Self {
        Self {
            functions,
            constants,
        }
    }

    /// Explicit empty coverage for a package with no functions or constants.
    pub fn empty() -> Self {
        Self::new(BTreeMap::new(), BTreeMap::new())
    }

    pub fn functions(&self) -> &BTreeMap<String, FunctionValueTransferPlans> {
        &self.functions
    }

    pub fn function(&self, function_key: &str) -> Option<&FunctionValueTransferPlans> {
        self.functions.get(function_key)
    }

    pub fn constants(&self) -> &BTreeMap<String, ValueTransferPlan> {
        &self.constants
    }

    pub fn constant(&self, symbol: &str) -> Option<&ValueTransferPlan> {
        self.constants.get(symbol)
    }
}

/// Dense transfer plans for one function frame.
///
/// `slot_plans` is indexed by MIR slot. `result_plans` is in result order
/// (zero entries for `void`, one for every other Phase 2 return type). The
/// emitter rejects missing, extra or differently-sized vectors rather than
/// defaulting any entry to `SnapshotShare`.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionValueTransferPlans {
    pub slot_plans: Vec<ValueTransferPlan>,
    pub result_plans: Vec<ValueTransferPlan>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_authoritative_stream_uses_affine_resource_plan() {
        let ty = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        let resolution = NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::AffineResource {
                drop: NativeResourceDropPlan::ResourceTableRelease,
            },
            embedding: NativeValueEmbedding::Forbidden,
        };

        assert_eq!(
            concrete_lifecycle_plan("streams::run", " return value", &ty, resolution).unwrap(),
            ValueTransferPlan::AffineResource {
                drop: ResourceDropPlan::ResourceTableRelease,
            }
        );
    }

    #[test]
    fn non_stream_affine_lifecycle_fails_closed() {
        let ty = TypeRefIr::builtin("file");
        let resolution = NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::AffineResource {
                drop: NativeResourceDropPlan::ResourceTableRelease,
            },
            embedding: NativeValueEmbedding::Forbidden,
        };

        let error =
            concrete_lifecycle_plan("io::read", " slot `handle`", &ty, resolution).unwrap_err();
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                construct: "non-Stream affine value lifecycle",
                ..
            }
        ));
    }

    #[test]
    fn never_and_unregistered_package_symbols_receive_ordinary_snapshot_plans() {
        assert_eq!(
            concrete_value_plan(&[], "m", "f", " slot `never`", &TypeRefIr::builtin("never"))
                .unwrap(),
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            }
        );
        let symbol = skiff_artifact_model::PackageSymbolRef {
            package: skiff_artifact_model::PackageRefIr::PackageId {
                package_id: "skiff.run/std".to_string(),
            },
            symbol_path: "std.websocket.WebSocketConnectRequest".to_string(),
            abi_expectation: Some("abi".to_string()),
        };
        assert_eq!(
            concrete_value_plan(
                &[],
                "m",
                "f",
                " slot `request`",
                &TypeRefIr::PackageSymbol { symbol },
            )
            .unwrap(),
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }
        );
    }

    #[test]
    fn stream_spelling_without_forbidden_embedding_fails_closed() {
        let ty = TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        let resolution = NativeValueLifecycleResolution {
            lifecycle: NativeValueLifecycleConcrete::AffineResource {
                drop: NativeResourceDropPlan::ResourceTableRelease,
            },
            embedding: NativeValueEmbedding::Ordinary,
        };

        let error =
            concrete_lifecycle_plan("streams::run", " return value", &ty, resolution).unwrap_err();
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                construct: "non-Stream affine value lifecycle",
                ..
            }
        ));
    }
}
