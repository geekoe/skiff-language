# P5-F420F Tooling path closure audit

状态：Ready（F420E scope-expansion 后继，只读审计）。

## 直接父节点

- `P5-F420E-command-execution-ledger-current-repair-result.md`

F420C、F420D 与 F420E 已让同一 `tooling` 完整路径连续暴露多个彼此独立的旧 fixture/oracle。
按照失败收敛规则，本节点在下一次完整 tooling verdict 前，对尚未执行的剩余 phase 做一次有界
路径闭合审计；不再根据第一个失败逐项创建后继。

## 精确候选与目的

- candidate：
  `f8a7f6a25fc2e0ad6e6cf0e780ffe306acc938a7`；
- tree：
  `7aea1e0a47e56aa0dde2d1d0efa19307e21ed849`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 candidate/tree 与 F415 ancestry。F420E 已在相同 executable code 上证明 tooling
前 8 个 phase 通过，并在第 9 个 `crate-public-api-gate.test.mjs` 得到 4/5；已知首错是测试仍
期待 `compiler-contract`，而 current `MANAGED_CRATE_NAMES.slice(1)` 实际缺
`skiff-deployment`。

本审计必须一次列清第 9–57 个 phase 的全部独立失败、真实 owner、上游遮挡关系和最小批量修复
范围，为一个 successor repair task 提供完整输入。它不是 gate verdict，不能解除 F421。

## 写入与禁止项

唯一允许写入：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F420F-tooling-path-closure-audit-result.md
```

production、test、fixture、manifest、lockfile、生成物与验证计划一律只读；不得修改任何其它文件，
不得 merge/rebase/push、访问 stable/live、instance 或 watch registry。不得派子 Agent。

若某个 phase 会修改受版本控制文件，先记录风险并跳过；临时目录与既有忽略构建缓存不算仓库写入。

## 审计方法

1. 运行并保存：

   ```bash
   node scripts/verify.mjs --only tooling --list
   ```

   确认精确 57 个 phase，前 8 个与 F420E 记录一致。
2. 不再执行会在首错停止的完整 `--only tooling`。从第 9 个
   `scripts/tests/crate-public-api-gate.test.mjs` 开始，按 canonical plan 顺序逐个执行每个
   phase，直到最后的 `vscode-grammar`；一个 phase 失败后继续下一个。
3. 对每个失败记录：
   - 命令、实际 test count 与完整首错；
   - 失败是 current production 缺陷、test/fixture 漂移、环境问题还是上游遮挡；
   - 精确代码 owner和最小必要写入文件；
   - 是否与其它失败属于同一根因，可否由一个批量 repair 合并处理；
   - 修复后应运行的最小聚焦探针。
4. 对通过的 phase 汇总数量和 test count，确认剩余路径没有因零测试被误判通过。
5. 只读检查已知 crate-public-api 首错，证明 current expected name 应从 canonical policy 派生还是
   可以稳定写死；不得修改。
6. 运行：

   ```bash
   git status --porcelain
   git diff --check
   ```

   除 result 外必须零 diff。

## 交付

提交唯一 result，记录：

- exact candidate/tree、57 phase 计划与实际执行覆盖；
- 第 9–57 个 phase 的通过/失败/测试计数；
- 所有独立 blocker 的证据、owner、遮挡关系与建议批量；
- 一个最小 successor repair task 的允许写入清单和聚焦验证矩阵；
- 哪些证据在 repair 后失效、哪些仍可继承；
- 未运行完整 verdict，未访问 stable/live，worktree clean。

若发现需要架构、公共契约或用户决策的问题，明确标记；仍不得实现。

