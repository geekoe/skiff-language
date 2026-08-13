use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity,
    validate_current_platform_error_projection_registry_ref, BytecodeArtifactRef,
    BytecodeFunctionStatementManifest, PackageArtifact, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_compiled::{
    BytecodeCompilationHandoff, BytecodeCompilationOutcome, BytecodeCompilationReceipt,
    CompiledPackage,
};
use skiff_compiler_contract::ServicePublicInstanceOperationFacts;
use skiff_compiler_emission::bytecode::{
    admit_phase_1_bytecode_mir, derive_bytecode_value_transfer_plans, emit_bytecode_artifact,
};
use skiff_compiler_emission::package_artifact::PublishedPackageArtifact;
use skiff_compiler_lowering::{mir::MirUnit, Bounds, ConstEvaluator};
use skiff_compiler_projection::package_artifact::{
    attach_package_execution as attach_projected_package_execution, PackageExecutionAttachment,
    ProjectedPackageArtifact,
};
use skiff_compiler_source::{
    source_value_transfer_plan, SourceValueTransferFacts, SourceValueTransferNominalFact,
    SourceValueTransferNominalId, SourceValueTransferNominalSemantics,
    SourceValueTransferPackageRef, SourceValueTransferPlanInput,
};

use crate::shared::package_compile_error::PackageCompileError;

/// Successful bytecode state for one package compilation.
///
/// An enabled failure never appears here: emission and handoff admission are
/// converted to the outer `PackageCompileError` before a package candidate is
/// returned. The enabled variant therefore always carries the exact admitted
/// artifact, path-free reference, and receipt as one typed handoff.
#[derive(Debug, Clone, PartialEq)]
pub enum PackageBytecodeLane {
    Disabled,
    Enabled(Box<BytecodeCompilationHandoff>),
}

impl PackageBytecodeLane {
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled(_))
    }

    pub fn handoff(&self) -> Option<&BytecodeCompilationHandoff> {
        match self {
            Self::Disabled => None,
            Self::Enabled(handoff) => Some(handoff),
        }
    }

    pub fn receipt(&self) -> Option<&BytecodeCompilationReceipt> {
        self.handoff().map(BytecodeCompilationHandoff::receipt)
    }
}

/// Complete in-memory output of one package compilation.
///
/// The package candidate, bytecode lane, and checked source-owned service
/// contract facts are intentionally kept together. Service compilation
/// consumes the exact public-instance facts before a bytecode-aware
/// publication owner consumes [`Self::into_parts`], writes the handoff's
/// bytecode record first, attaches the canonical returned path, and writes the
/// PackageArtifact record last. This type performs no store I/O.
#[must_use = "an enabled handoff must be published before its PackageArtifact record"]
#[derive(Debug, Clone, PartialEq)]
pub struct PackageCompileOutput {
    package: PublishedPackageArtifact,
    bytecode: PackageBytecodeLane,
    public_instance_operations: ServicePublicInstanceOperationFacts,
}

impl PackageCompileOutput {
    pub(super) fn try_new(
        package: PublishedPackageArtifact,
        bytecode: PackageBytecodeLane,
        public_instance_operations: ServicePublicInstanceOperationFacts,
    ) -> Result<Self, PackageCompileError> {
        validate_package_execution_state(&package.artifact, &bytecode)?;
        Ok(Self {
            package,
            bytecode,
            public_instance_operations,
        })
    }

    pub fn package(&self) -> &PublishedPackageArtifact {
        &self.package
    }

    pub fn bytecode(&self) -> &PackageBytecodeLane {
        &self.bytecode
    }

    pub fn bytecode_handoff(&self) -> Option<&BytecodeCompilationHandoff> {
        self.bytecode.handoff()
    }

    pub fn bytecode_receipt(&self) -> Option<&BytecodeCompilationReceipt> {
        self.bytecode.receipt()
    }

    pub(super) fn public_instance_operations(&self) -> &ServicePublicInstanceOperationFacts {
        &self.public_instance_operations
    }

    /// Splits the complete candidate for bytecode-aware publication planning.
    pub fn into_parts(self) -> (PublishedPackageArtifact, PackageBytecodeLane) {
        (self.package, self.bytecode)
    }

    /// Extracts the old publication payload only when this exact request was
    /// explicitly bytecode-disabled. An enabled handoff is returned intact in
    /// `Err` and can never be silently discarded as a legacy result.
    pub fn into_disabled_package(self) -> Result<PublishedPackageArtifact, Box<Self>> {
        if matches!(&self.bytecode, PackageBytecodeLane::Disabled) {
            Ok(self.package)
        } else {
            Err(Box::new(self))
        }
    }
}

/// Runs the bytecode lane after source compilation has produced typed MIR.
///
/// The frozen outcome type makes the only legal disabled case explicit and
/// turns every enabled error into the outer package compile failure.
pub(super) fn compile_bytecode_lane(
    emit_bytecode: bool,
    compiled: &CompiledPackage,
) -> Result<PackageBytecodeLane, PackageCompileError> {
    let outcome: BytecodeCompilationOutcome<PackageCompileError> = if emit_bytecode {
        BytecodeCompilationOutcome::from_enabled_result(emit_enabled_bytecode(compiled))
    } else {
        BytecodeCompilationOutcome::disabled()
    };
    finish_bytecode_lane(outcome)
}

fn finish_bytecode_lane(
    outcome: BytecodeCompilationOutcome<PackageCompileError>,
) -> Result<PackageBytecodeLane, PackageCompileError> {
    match outcome.into_result()? {
        None => Ok(PackageBytecodeLane::Disabled),
        Some(handoff) => Ok(PackageBytecodeLane::Enabled(Box::new(handoff))),
    }
}

fn emit_enabled_bytecode(
    compiled: &CompiledPackage,
) -> Result<BytecodeCompilationHandoff, PackageCompileError> {
    let package_id = compiled.compile_model().policy().package_id().to_string();
    let units = compiled.lowered().mir_units();
    let admitted = admit_phase_1_bytecode_mir(units)?;
    let mut bundles = Vec::new();
    for unit in compiled.lowered().file_ir_units() {
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(unit)
            .map_err(|error| PackageCompileError::ContractValidation {
                message: format!("frozen constant evaluation failed: {error}"),
            })?;
        bundles.push(bundle);
    }
    let facts = source_value_transfer_facts_for_units(units);
    let plans = derive_bytecode_value_transfer_plans(&admitted, |module_path, ty| {
        source_value_transfer_plan(
            &facts,
            SourceValueTransferPlanInput::concrete(module_path, ty),
        )
        .map_err(|error| error.to_string())
    })?;
    let artifact = emit_bytecode_artifact(&admitted, &bundles, &plans)?;
    let mut statement_manifest = artifact
        .image
        .functions
        .values()
        .map(|function| {
            BytecodeFunctionStatementManifest::new(
                function.origin.clone(),
                function.statement_entries.clone(),
            )
        })
        .collect::<Vec<_>>();
    statement_manifest.sort_by(|left, right| left.origin.cmp(&right.origin));
    let manifest_identity =
        derive_bytecode_statement_manifest_identity(&package_id, &statement_manifest).map_err(
            |error| PackageCompileError::ContractValidation {
                message: format!("bytecode statement manifest derivation failed: {error}"),
            },
        )?;
    let reference = BytecodeArtifactRef::new(artifact.bytecode_identity.clone());
    Ok(BytecodeCompilationHandoff::try_new(
        package_id,
        statement_manifest,
        manifest_identity,
        artifact,
        reference,
    )?)
}

/// Builds the source-owned exact nominal facts from the lowered type tables.
///
/// This is the pipeline's single injection of source value-transfer facts into
/// bytecode emission. Each module contributes both its `Local` and
/// `Publication` nominal identities, so in-module and cross-module references
/// resolve through the same exact declarations.
fn source_value_transfer_facts_for_units(units: &[MirUnit]) -> SourceValueTransferFacts {
    let mut facts = SourceValueTransferFacts::new();
    // The Phase 4 pinned sleep argument is the std transparent alias
    // `Duration = integer`. User packages reference it as a package symbol and
    // therefore do not contribute a local/publication type-table fact for it.
    facts.insert_nominal(
        SourceValueTransferNominalId::PackageSymbol {
            package: SourceValueTransferPackageRef::PackageId("skiff.run/std".to_string()),
            symbol_path: "std.time.Duration".to_string(),
            abi_expectation: None,
        },
        SourceValueTransferNominalFact {
            declaration_module: "std.time".to_string(),
            type_parameters: Vec::new(),
            semantics: SourceValueTransferNominalSemantics::Ordinary(TypeDescriptorIr::Alias {
                target: TypeRefIr::builtin("integer"),
            }),
        },
    );
    for unit in units {
        for (type_index, declaration) in unit.type_table.iter().enumerate() {
            let fact = SourceValueTransferNominalFact {
                declaration_module: unit.module_path.clone(),
                type_parameters: declaration.type_params.clone(),
                semantics: SourceValueTransferNominalSemantics::Ordinary(
                    declaration.descriptor.clone(),
                ),
            };
            let type_index = u32::try_from(type_index).expect("MIR type table index fits in u32");
            facts.insert_nominal(
                SourceValueTransferNominalId::Local {
                    module_path: unit.module_path.clone(),
                    type_index,
                },
                fact.clone(),
            );
            facts.insert_nominal(
                SourceValueTransferNominalId::Publication {
                    module_path: unit.module_path.clone(),
                    type_index,
                },
                fact,
            );
        }
    }
    facts
}

/// Attaches one exact admitted execution handoff without mutating the source
/// projection.
///
/// The projection boundary treats both attachment fields as untrusted and
/// returns a newly identity-assigned value. This driver then checks the
/// returned package id, bytecode reference, and statement manifest against the
/// same handoff. Every error therefore leaves `projected` unchanged.
pub(super) fn attach_bytecode_execution(
    projected: &ProjectedPackageArtifact,
    bytecode: &PackageBytecodeLane,
) -> Result<ProjectedPackageArtifact, PackageCompileError> {
    let PackageBytecodeLane::Enabled(handoff) = bytecode else {
        validate_package_execution_state(&projected.artifact, bytecode)?;
        return Ok(projected.clone());
    };

    let manifest_receipt = handoff.statement_manifest_receipt();
    let attached = attach_projected_package_execution(
        projected,
        PackageExecutionAttachment {
            bytecode: handoff.reference().clone(),
            statement_manifest_identity: manifest_receipt.identity().clone(),
        },
    )
    .map_err(|error| bytecode_projection_error(error.to_string()))?;
    validate_package_execution_state(&attached.artifact, bytecode)?;
    Ok(attached)
}

fn validate_package_execution_state(
    artifact: &PackageArtifact,
    bytecode: &PackageBytecodeLane,
) -> Result<(), PackageCompileError> {
    match bytecode {
        PackageBytecodeLane::Disabled => {
            validate_current_package_registry(artifact)?;
            validate_disabled_execution_state(artifact)
        }
        PackageBytecodeLane::Enabled(handoff) => {
            if &artifact.platform_error_projection_registry
                != handoff
                    .receipt()
                    .authorities()
                    .platform_error_projection_registry()
            {
                return Err(bytecode_projection_error(
                    "PackageArtifact platform error projection registry mismatch with admitted bytecode handoff",
                ));
            }
            validate_current_package_registry(artifact)?;
            let manifest_receipt = handoff.statement_manifest_receipt();
            if artifact.package_id != manifest_receipt.package_id() {
                return Err(bytecode_projection_error(format!(
                    "PackageArtifact package id {} does not match admitted statement manifest package id {}",
                    artifact.package_id,
                    manifest_receipt.package_id()
                )));
            }
            if artifact.bytecode.as_ref() != Some(handoff.reference()) {
                return Err(bytecode_projection_error(format!(
                    "PackageArtifact bytecode reference does not exactly match admitted handoff {}",
                    handoff.reference().bytecode_identity
                )));
            }
            if &artifact.bytecode_statement_manifest_identity != manifest_receipt.identity() {
                return Err(bytecode_projection_error(format!(
                    "PackageArtifact statement manifest {} does not exactly match admitted handoff {}",
                    artifact.bytecode_statement_manifest_identity,
                    manifest_receipt.identity()
                )));
            }
            Ok(())
        }
    }
}

fn validate_current_package_registry(
    artifact: &PackageArtifact,
) -> Result<(), PackageCompileError> {
    validate_current_platform_error_projection_registry_ref(
        &artifact.platform_error_projection_registry,
    )
    .map_err(|_| {
        bytecode_projection_error(
            "PackageArtifact platform error projection registry mismatch with current compiler authority",
        )
    })
}

fn validate_disabled_execution_state(
    artifact: &PackageArtifact,
) -> Result<(), PackageCompileError> {
    if let Some(reference) = artifact.bytecode.as_ref() {
        return Err(bytecode_projection_error(format!(
            "disabled bytecode lane produced PackageArtifact reference {}",
            reference.bytecode_identity
        )));
    }
    let expected = derive_bytecode_statement_manifest_identity(&artifact.package_id, &[]).map_err(
        |error| {
            bytecode_projection_error(format!(
                "failed to derive canonical empty statement manifest for package {}: {error}",
                artifact.package_id
            ))
        },
    )?;
    if artifact.bytecode_statement_manifest_identity != expected {
        return Err(bytecode_projection_error(format!(
            "disabled bytecode lane produced statement manifest {}, expected package-specific canonical empty manifest {}",
            artifact.bytecode_statement_manifest_identity, expected
        )));
    }
    Ok(())
}

fn bytecode_projection_error(message: impl Into<String>) -> PackageCompileError {
    PackageCompileError::BytecodeProjection {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
