# P5-F420G4 Test-runner target current inventory result

状态：`PASS`。Test-runner Cargo target inventory oracle 已收敛到 current 两个独立 integration
target；聚焦测试精确 `3/3 PASS`。本节点只闭合 F420F blocker B4，不建立完整 tooling verdict。

## 1. Exact start 与 implementation checkpoint

- batch exact start / tree：
  `924e8f3a246873b160ba12e2abd697b0b11c9f59` /
  `a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`；
- task checkout / tree：
  `65efc72a08896549c6d5f1c6abb5b6fedb5b2a22` /
  `197f6fc0165d77a968b56578846168942a026bd8`；
- implementation commit / tree：
  `d49a1f5987a0ea38e0eacbb56d276cd936f3a597` /
  `08ccca0ed5ad15ac2b461599bff523695d6ea58e`。

启动时 `git merge-base --is-ancestor` 证明 batch exact start 是 task checkout 的 ancestor，且
exact start tree 与 batch 记录一致。task checkout 相对 exact start 只增加 F420G batch 及其五个
leaf task 文档；implementation 只修改授权的 inventory test。最终 result-only commit 不改变上述
executable tree，其 commit 由交付消息记录。

## 2. Current inventory oracle

`scripts/tests/test-runner-runtime-isolation.test.mjs` 现在按 Cargo manifest 顺序精确接受：

```text
package_service_contract_deployment
canonical_std_seed_bootstrap
```

oracle 仍使用整个 `[[test]]` inventory 的 `deepEqual`，因此缺失、重排或增加第三个 target 都会
失败关闭。原有 `runtime-integration-worker` 与 `test_runner_runtime_isolation` 反向断言保持不变，
继续证明 canonical cutover target 未恢复 feature gate，也没有 recursive wrapper。
`canonical_std_seed_bootstrap` 作为第二个独立 target 被接受；没有修改
`test-runner/Cargo.toml`、任何 test-runner production/test target 或其它 test。

## 3. 聚焦验证

修改前基线精确复现 F420F B4：同一 Node test 为 `2/3`，唯一失败显示 actual 比 expected 多
`canonical_std_seed_bootstrap`。

修改后：

| 命令 | 结果 |
| --- | --- |
| `node --test scripts/tests/test-runner-runtime-isolation.test.mjs` | `3/3 PASS` |
| `git diff --check` | PASS |

## 4. 边界

implementation 只修改任务授权的一个 test，result 是唯一新增文档。没有修改 Cargo manifest、
test-runner、其它 test、lockfile 或验证计划；没有运行完整 tooling、test-runner Rust suite、
`run-skiff-tests`、stable/live、instance 或 watch registry；没有派子 Agent，也没有
merge、rebase 或 push。implementation 与 result 分开提交，最终 worktree clean。
