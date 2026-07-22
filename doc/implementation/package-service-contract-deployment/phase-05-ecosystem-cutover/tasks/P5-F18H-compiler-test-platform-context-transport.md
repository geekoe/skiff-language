# P5-F18H：Compiler Test-only Platform Context Transport

权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；F16A与D20/D22。从D20
docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18h-compiler-test-context`、
`codex/p5-f18h-compiler-test-context`。全新Agent、一个test-only commit，不merge/push/stable/I16/Host；五分钟内修改。

exclusive write set仅`compiler/tests/common/package_project.rs`与`package_graph.rs`。不改F18A input/source、F18B
authoring/runner、production compiler、调用它们的18个targets、manifest/lock。

完成态：private project funnel在第一次user manifest读取前构造一次test-only `CompilerPlatformSources`，沿现有测试模式
只在test common使用`env!("CARGO_MANIFEST_DIR")`定位repo；typed传播失败。同一borrow贯穿manifest discovery、graph、
`PackageCompileInput::new`与`read_official_package_sources`；不改76个helper callers、不逐node重建、不加global/OnceLock/
第二resolver。三处旧签名全部消失，18个common targets可编译，production lib/bins仍PASS。

```bash
cargo check --locked -p skiff-compiler --tests
cargo test --locked -p skiff-compiler --tests --no-run
cargo test --locked -p skiff-compiler --test package_std_schema platform_std_schema_types_are_available_without_a_manifest_requirement -- --exact
cargo test --locked -p skiff-compiler --test package_std_schema platform_std_rejects_a_user_dependency_alias -- --exact
cargo check --locked -p skiff-compiler --lib --bins
git diff --check
```

回报18-target compile、正/负std测试、commit/tree/lock、全仓旧签名反搜与extra-review。与F18A/B可并行；若其公开API
必须变化，报`TASK_NOT_EXECUTABLE`，不得越界。
