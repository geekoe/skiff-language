# P5-F18B：Platform Context Guard Before Source IO

权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；D18/F16与D20 result。从D20
docs checkpoint建立`/Users/geek/workspace/skiff-p5-f18b-platform-pre-read`、`codex/p5-f18b-platform-pre-read`。
全新Agent、一个clean commit，不merge/push/stable/I16/Host；五分钟内修改。

exclusive write set：`compiler/driver/authoring.rs`及child tests、`test-runner/src/canonical_package.rs`及child tests。
不改compiler input/source/pipeline、binary/JS、runner其他模块、manifest/lock。

完成态：package authoring在第一次manifest/source读取前调用既有`initialize_prelude_registry(platform_sources)`；runner
在root manifest/store/dependency/source读取前调用同一guard。same root顺序为guard→read spy恰1次；different root返回
typed `DifferentPlatformRoot`且package manifest/source spy为0。pipeline guard保留为defense-in-depth；contract/deployment/
assembly不被无故要求prelude。不得新增root comparator/resolver或字符串化typed error。

本节点在`canonical_package` child tests中新增merge-only
`canonical_package::tests::combined::p5_f18_compiler_repair_combined`，覆盖F18A真实symlink escape、same-root golden与
authoring/runner different-root zero-read；本分支只确保它可编译，不运行。它只消费基线公开入口，不得制造F18A开发依赖。

```bash
cargo test --locked -p skiff-compiler --lib p5_f18b_authoring_mismatch_zero_source_reads -- --test-threads=1
cargo test --locked -p skiff-test-runner --lib p5_f18b_runner_mismatch_zero_source_reads -- --test-threads=1
cargo check --locked -p skiff-compiler -p skiff-test-runner --lib
git diff --check
```

测试放child module，不继续膨胀authoring大文件。回报typed downcast/pattern、same=1/mismatch=0、pipeline guard反搜、
commit/tree/lock/extra-review。若需消费F18A新API或越写集，停止；两节点设计上并行。
