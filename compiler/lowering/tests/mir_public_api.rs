use skiff_compiler_lowering::mir::{
    builder::build_mir_units, liveness::compute_liveness, MirBuildError, MirConst,
    MirContractError, MirExpression, MirFunction, MirLiveness, MirUnit,
};

#[test]
fn mir_contract_is_reachable_from_the_crate_root() {
    fn assert_public_type<T>() {}

    assert_public_type::<MirUnit>();
    assert_public_type::<MirFunction>();
    assert_public_type::<MirConst>();
    assert_public_type::<MirExpression>();
    assert_public_type::<MirBuildError>();
    assert_public_type::<MirContractError>();

    let _: fn(&MirFunction) -> Result<MirLiveness, MirContractError> = compute_liveness;
    let _: fn(
        &str,
        &[skiff_artifact_model::FileIrUnit],
        &skiff_compiler_source::SourceCallableEffectFacts,
    ) -> Result<Vec<MirUnit>, MirBuildError> = build_mir_units;
}
