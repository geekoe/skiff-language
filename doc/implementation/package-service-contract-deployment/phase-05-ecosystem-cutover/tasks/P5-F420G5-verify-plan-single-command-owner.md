# P5-F420G5 Verify plan single command owner

状态：Ready。

## 直接父节点

- `P5-F420G-tooling-closure-batch.md`

F420F 的 B5 已证明 default verify 同时从 `scripts-dev-sync` builder 与 checker registry 展开同一个
`node scripts/check-package-store-discovery.mjs`，而 `assertPlanIntegrity` 正确拒绝重复执行。

## 唯一写入

```text
scripts/lib/verify-checkers.mjs
scripts/lib/verify-plan.mjs
本任务 result
```

从 batch exact start/tree 启动。让 package-store discovery command 只有一个 canonical 声明，
并归属于 `scripts-dev-sync` / `implementation:tooling`。优先让 checker registry 成为事实源：
registry invocation 指向 `scripts-dev-sync`，对应 phase builder 消费
`checkerPhases(root, 'scripts-dev-sync')`；它不再由 `checks-default` 直接展开，但仍通过 default
`tests -> implementation-tests -> tooling -> scripts-dev-sync` 精确执行一次。不得复制 command、
放宽 duplicate gate、改 tests 来掩盖问题或改变 live/default 边界。不得派子 Agent、
merge/rebase/push/stable/live。

```bash
node --test \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-rust-quality.test.mjs \
  scripts/tests/verify-taxonomy.test.mjs \
  scripts/tests/verify.test.mjs
node scripts/verify.mjs --list
node scripts/verify.mjs --only tooling --list
node scripts/verify.mjs --only checks --list
git diff --check
```

四文件预期全部通过；三个 list 都必须通过 plan integrity。package-store command 在 default
verify 与 tooling 中各精确出现一次，在 checks-only 中为零。实现/result 分开提交并保持 clean；
若现有 registry API 无法表达单 owner，停止并返回精确范围扩张，不自行改 scanner/test。
