use std::path::Path;

use serde_json::Value;

pub(super) fn reject_removed_service_assembly_fields(
    assembly: &Value,
    assembly_relative_path: &Path,
) -> anyhow::Result<()> {
    for field in [
        "providerRequirements",
        "transportSelection",
        "effectSummaries",
    ] {
        if assembly.get(field).is_some() {
            anyhow::bail!(
                "serviceAssembly {} contains removed field {field}",
                assembly_relative_path.display()
            );
        }
    }
    if assembly.pointer("/service/exports").is_some() {
        anyhow::bail!(
            "serviceAssembly {} contains removed field service.exports",
            assembly_relative_path.display()
        );
    }
    Ok(())
}
