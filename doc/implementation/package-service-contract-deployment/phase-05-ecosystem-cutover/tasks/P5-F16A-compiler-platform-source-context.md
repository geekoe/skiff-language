# P5-F16A：Compiler Platform Source Context

## 输入与DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14；阶段标准1、2、6。
- 输入：D18完成；production基线`40ed693` / tree `01f6b8d1`，从包含本任务合同的integration docs
  checkpoint建立`/Users/geek/workspace/skiff-p5-f16a-platform-context`、分支
  `codex/p5-f16a-platform-context`。
- 高风险shared trust checkpoint；完成后解除F16B/F16C。一个clean commit，不merge/push，不改stable。
- 五分钟内开始实际修改；若context无法在既有依赖方向内成为唯一owner，立即回报`TASK_NOT_EXECUTABLE`，不得
  另造parallel helper或fallback。

## 写入owner与非目标

owner限`compiler/input`的platform context、manifest/source trust，`compiler/source/prelude_registry/**`及把显式
context接入compiler library pipeline所需的最小Rust API和直接tests。可调整`compiler/driver/pipeline/**`与
`compiler/driver/authoring.rs`的library signature，使F16B/F16C能从同一checkpoint独立接线；不得实现binary CLI
parse、JS transport或test-runner consumer。`authoring.rs`在本任务后仍归F16A checkpoint，F16B不得回改。

不改artifact/schema/identity语义、Router、Runtime、deployment、source-suite、Host fixture、manifest或Cargo.lock。
禁止`Default`、ambient cwd/env/executable位置、`CARGO_MANIFEST_DIR` production fallback、任意
`ManifestOwner + root`组合、reserved-id放宽或双路径。`package_sources.rs`已超过500行；直接修改时必须把tests或
新platform职责移出，不能继续膨胀。

## 完成态

- 新`CompilerPlatformSources`要求绝对root，运行时canonicalize；严格验证唯一`std/registry.yml`、registry成员
  canonical containment、`std`与`prelude/error.skiff`，symlink到同一canonical root可接受。
- builtin manifest discovery与official source读取只能由context授予并复验provenance；trust root外复制的
  `skiff.run/std`仍按用户package拒绝，missing/cross-root/duplicate/unknown registry输入fail closed。
- compiler package pipeline显式消费context；所有production platform source读取均来自该owner。
- `PreludeRegistry`只从context初始化，同canonical root幂等、不同root返回typed failure；`prelude_identity`只读
  已初始化registry。production代码和production dep-info不再含platform用途的`CARGO_MANIFEST_DIR`。
- test-only旧content算法或冻结golden证明新context的prelude schema/native/combined identity与`40ed693` bit-identical；
  固定测试名包含`platform_source_context_preserves_legacy_prelude_identity`，production不得保留legacy reader。
- 没有第二platform-root helper；直接触碰的>200行文件完成extra-review，production文件不新增>500行混合职责。

这是implementation checkpoint：compiler library必须buildable，但compiler binary与test-runner在F16B/F16C合流前允许
因缺required transport暂时断链；该状态不得作pre-acceptance candidate。证据只对F16A exact commit及未变化的platform
context、input/source/prelude、compiler library API、platform source内容与Cargo.lock有效。

## 唯一聚焦验证

```bash
cargo test --locked -p skiff-compiler-input platform_sources
cargo test --locked -p skiff-compiler-source prelude_registry
cargo test --locked -p skiff-compiler --lib
cargo check --locked -p skiff-compiler --lib
cargo fmt --all -- --check
git diff --check
```

tests覆盖absolute/canonical/symlink正例、missing/cross-root/fake reserved manifest负例、同root幂等与不同root
fail closed。不得运行source-suite、Host或完整verify。回报commit/tree/lock blob、API handoff、反向搜索、文件行数
与extra-review自验收矩阵。
