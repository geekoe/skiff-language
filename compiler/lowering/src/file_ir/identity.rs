use skiff_artifact_model::FileIrUnit;

pub fn file_ir_identity(unit: &FileIrUnit) -> String {
    skiff_artifact_identity::file_ir_identity(unit)
        .expect("lowered File IR must serialize for canonical artifact identity")
}

pub fn assign_file_ir_identity(unit: &mut FileIrUnit) -> String {
    let computed = file_ir_identity(unit);
    unit.file_ir_identity = computed.clone();
    computed
}

#[cfg(test)]
pub fn canonical_file_ir_identity_value(unit: &FileIrUnit) -> serde_json::Value {
    skiff_artifact_identity::canonical_file_ir_identity_value(unit)
        .expect("lowered File IR must serialize for canonical artifact identity")
}

#[cfg(test)]
mod tests;
