# P5-F53G：Runtime Config v2 Fixtures

只改`runtime/host/src/loader/runtime_config.rs`测试fixture/helper，使两个失败正例使用完整canonical
`skiff-service-protocol-v2:sha256:<64hex>`并实际产生register frame；不得放宽production validation。
运行`cargo test --locked -p skiff-runtime-host runtime_config`，rustfmt/diff/反搜，提交单一commit。
