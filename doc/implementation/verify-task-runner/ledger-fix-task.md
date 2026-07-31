# ledger-fix-task：为 check-rust-file-lines.mjs 登记 execFileSync 生命周期 owner

## 目标

修复预存 command-execution policy 失败：`scripts/check-rust-file-lines.mjs` 顶层使用
`execFileSync`（`node:child_process`）但未登记 ledger，导致
`scripts/tests/command-execution-policy.test.mjs` 失败。

## 方案（用户已授权，方案 1：保留 execFileSync）

- `scripts/check-rust-file-lines.mjs`：两处 `execFileSync` 调用移入命名函数
  `runFileLineGate()`，每个调用点前加 `// child-process-owner: rust-file-line-gate`，
  保持同步执行与既有行为/输出完全一致，函数在文件末尾调用。
- `scripts/lib/command-execution-policy.mjs`：`ALLOWED_IMPORTED_SYMBOLS` 增加
  `'execFileSync'`。
- `scripts/lib/command-execution-ledger.mjs`：新增 owner 条目（callCount: 2，
  ownerClass: domain-adapter，reason 说明行数门禁同步执行 rg/wc 并保留输出/退出语义）。
- `scripts/tests/command-execution-policy.test.mjs`：计数期望更新为 13 owner
  （10 spawn / 2 execFile / 1 execFileSync）。

## 预检结论

- 唯一未登记 production 文件即 `scripts/check-rust-file-lines.mjs`；无第二个未登记文件。
- scanner 语义不变；不需要超出授权面的机制改动。
- 已知过时文档 `doc/implementation/tail-call-execution/t1-tooling-cardinality.md`
  仍写 11 owners（baseline 已是 12），属 doc/ 禁止修改范围，不在此任务处理。

## 聚焦验证

```bash
node --test scripts/tests/command-execution-policy.test.mjs
node scripts/check-command-execution-policy.mjs
node scripts/check-rust-file-lines.mjs
node --test scripts/tests/command-execution.test.mjs
```
