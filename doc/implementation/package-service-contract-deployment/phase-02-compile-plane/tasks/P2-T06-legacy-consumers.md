# P2-T06：Legacy Runtime / Test Consumer Adapter

## 目标

让现有runtime、package-test与test-runner只通过明确、单向、可删除的adapter消费canonical PackageArtifact；
保持main可运行，同时禁止旧DTO重新成为compiler事实owner。

## 依赖与 worktree

- 依赖T02–T04合入integration checkpoint。
- 建议branch：`codex/package-service-p2-t06-legacy-consumers`。
- 可与T05并行；不得修改compiler中央pipeline。

## 完成态

1. 唯一明确命名的legacy runtime adapter从PackageArtifact及legacy service deployment seeds生成当前runtime
   所需PackageUnit/ServiceUnit形状；只转换shape，不重算identity/effect/projection/closure。
2. adapter不二次读取source或调用source/type/lowering；File IR、build/local ABI、requirements与callable
   facts均来自PackageArtifact。
3. package-test和test-runner调用production PackageArtifact materializer，再经同一adapter进入当前runtime；
   不维护第二builder或identity helper。
4. 旧service fixture仍可通过现有测试入口执行；canonical artifacts不dual-write，旧外壳只在临时runtime
   artifact位置产生并有Phase03/05删除ledger。
5. 所有legacy import只落在T05 checker预先约定的固定路径；本任务不修改checker或allowlist。若固定路径无法
   容纳实现，回报main调整ownership，不扩大例外。
6. 更新/替换锁定旧PackageUnit canonical owner的tests；删除测试必须给出replacement语义。

## 写入范围

- `compiler/emission/src/legacy_runtime_adapter/**` 中明确legacy adapter及其入口。
- `runtime/package-test/**`、`test-runner/**`及直接fixtures/tests。
- 不修改任何structure checker、checker fixture/self-test或allowlist；这些由T05唯一拥有。
- 不修改artifact identity、source effect、service-call lowering或compiler driver中央cutover。

## 验证

```bash
cargo test -p skiff-runtime-package-test
cargo test -p skiff-test-runner
cargo test -p skiff-compiler --test package_unit_single_path
cargo test -p skiff-test-runner --test test_runner_package_visibility
node scripts/check-artifact-identity-single-source.mjs --self-test
git diff --check
```

按实际package名调整命令并回报。至少证明production/package-test identity一致、legacy service只source compile
一次、adapter外旧DTO canonical import被checker拒绝。

## 回报

提交commit、自验收矩阵、adapter唯一入口与实际legacy import清单、测试 disposition和Phase03/05删除清单。
