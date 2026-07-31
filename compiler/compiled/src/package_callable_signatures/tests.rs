use super::*;

#[test]
fn missing_module_is_a_structured_projection_input_error() {
    let error = executable_target(&BTreeMap::new(), "run", "api", "run").unwrap_err();

    assert!(matches!(
        error,
        ProjectionInputBuildError::MissingModule {
            public_path,
            module_path,
        } if public_path == "run" && module_path == "api"
    ));
}

#[test]
fn missing_executable_is_a_structured_projection_input_error() {
    let unit = FileIrUnit::empty("api", "source-hash");
    let units = BTreeMap::from([("api", &unit)]);
    let error = executable_target(&units, "run", "api", "run").unwrap_err();

    assert!(matches!(
        error,
        ProjectionInputBuildError::MissingExecutable {
            public_path,
            module_path,
            source_symbol,
        } if public_path == "run" && module_path == "api" && source_symbol == "run"
    ));
}
