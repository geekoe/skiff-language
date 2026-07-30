# P5-F420G1 Crate public API current oracles

状态：Ready。

## 直接父节点

- `P5-F420G-tooling-closure-batch.md`

F420F 的 B1 已证明 production canonical policy 正确包含
`skiff-deployment`、`skiff-compiler-contract`、`skiff-compiler`；两个 test 仍冻结旧的双 crate
形态。

## 唯一写入

```text
scripts/tests/crate-public-api-gate.test.mjs
scripts/tests/crate-public-api-policy.test.mjs
本任务 result
```

从 batch 记录的 exact start/tree 启动并证明 ancestry。不得修改 production policy、checker、
其它 test、manifest 或 lockfile；不得派子 Agent、merge/rebase/push/stable/live。

## 实现与验证

1. gate test 制造 `MANAGED_CRATE_NAMES.slice(1)` 缺项时，预期缺失名必须从同一 canonical 数组
   派生并安全转义为 regex；不得硬编码 `compiler-contract` 或当前首项。
2. policy test 的精确 owner 集合更新为 current 三项，仍以 canonical policy 为事实源。
3. 保留 missing/skip fail-closed 与其它断言语义。

```bash
node --test \
  scripts/tests/crate-public-api-gate.test.mjs \
  scripts/tests/crate-public-api-policy.test.mjs
git diff --check
```

预期精确 8/8。实现/result 分开提交；result 记录 commit/tree、派生方式、8/8、反搜旧硬编码和
clean 状态。范围扩张立即返回 `TASK_SCOPE_EXPANDED`。

