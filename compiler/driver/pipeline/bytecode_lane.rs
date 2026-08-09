use skiff_artifact_identity::assign_package_artifact_identities;
use skiff_compiler_compiled::{
    BytecodeCompilationHandoff, BytecodeCompilationOutcome, BytecodeCompilationReceipt,
    CompiledPackage,
};
use skiff_compiler_contract::ServicePublicInstanceOperationFacts;
use skiff_compiler_emission::package_artifact::PublishedPackageArtifact;
use skiff_compiler_projection::package_artifact::ProjectedPackageArtifact;

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
        match (&bytecode, package.artifact.bytecode.as_ref()) {
            (PackageBytecodeLane::Disabled, None) => {}
            (PackageBytecodeLane::Disabled, Some(reference)) => {
                return Err(PackageCompileError::BytecodeProjection {
                    message: format!(
                        "disabled bytecode lane produced PackageArtifact reference {}",
                        reference.bytecode_identity
                    ),
                });
            }
            (PackageBytecodeLane::Enabled(handoff), Some(reference))
                if reference == handoff.reference() => {}
            (PackageBytecodeLane::Enabled(handoff), Some(reference)) => {
                return Err(PackageCompileError::BytecodeProjection {
                    message: format!(
                        "PackageArtifact bytecode reference {} does not match admitted handoff {}",
                        reference.bytecode_identity,
                        handoff.reference().bytecode_identity
                    ),
                });
            }
            (PackageBytecodeLane::Enabled(handoff), None) => {
                return Err(PackageCompileError::BytecodeProjection {
                    message: format!(
                        "enabled bytecode handoff {} is absent from PackageArtifact identity projection",
                        handoff.reference().bytecode_identity
                    ),
                });
            }
        }
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

/// Temporary fail-closed seam while the canonical emitter input is incomplete.
///
/// The emitter checkpoint requires a source-owned value-transfer plan for
/// every function, but no such bundle is exposed by source/lowering yet and
/// the emission crate has no final `emit_bytecode_artifact` entrypoint. The
/// driver therefore cannot legally construct an artifact. It reports the
/// enabled failure after typed MIR exists instead of inventing SnapshotShare
/// plans, reopening File IR, or falling back to the disabled lane.
fn emit_enabled_bytecode(
    compiled: &CompiledPackage,
) -> Result<BytecodeCompilationHandoff, PackageCompileError> {
    Err(PackageCompileError::BytecodeEmitterUnavailable {
        mir_unit_count: compiled.lowered().mir_units().len(),
    })
}

/// Binds an admitted path-free bytecode reference into package identity
/// projection before any package candidate bytes or paths are materialized.
pub(super) fn attach_bytecode_reference(
    projected: &mut ProjectedPackageArtifact,
    bytecode: &PackageBytecodeLane,
) -> Result<(), PackageCompileError> {
    if let Some(existing) = projected.artifact.bytecode.as_ref() {
        return Err(PackageCompileError::BytecodeProjection {
            message: format!(
                "PackageArtifact projection unexpectedly pre-populated bytecode reference {}",
                existing.bytecode_identity
            ),
        });
    }

    let PackageBytecodeLane::Enabled(handoff) = bytecode else {
        return Ok(());
    };
    projected.artifact.bytecode = Some(handoff.reference().clone());
    assign_package_artifact_identities(&mut projected.artifact).map_err(|error| {
        PackageCompileError::BytecodeProjection {
            message: format!(
                "failed to recompute PackageArtifact identity with bytecode {}: {error}",
                handoff.reference().bytecode_identity
            ),
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicitly_disabled_outcome_is_the_only_none_lane() {
        let lane = finish_bytecode_lane(BytecodeCompilationOutcome::disabled()).unwrap();

        assert_eq!(lane, PackageBytecodeLane::Disabled);
        assert!(!lane.is_enabled());
        assert!(lane.handoff().is_none());
        assert!(lane.receipt().is_none());
    }

    #[test]
    fn enabled_failure_is_propagated_instead_of_becoming_disabled() {
        let error = finish_bytecode_lane(BytecodeCompilationOutcome::failed(
            PackageCompileError::BytecodeEmitterUnavailable { mir_unit_count: 3 },
        ))
        .unwrap_err();

        assert!(matches!(
            error,
            PackageCompileError::BytecodeEmitterUnavailable { mir_unit_count: 3 }
        ));
    }
}
