# Rust 大型测试模块重构：line gate 叶子

日期：2026-08-01

状态：completed；结果见
[`rust-large-test-module-refactor-line-gate-result.md`](./rust-large-test-module-refactor-line-gate-result.md)

直接父节点：[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md)，该父节点继续引用唯一权威设计
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)。本文件只定义 DAG 节点 D 的执行合同，不改变父节点或
权威设计。

## 节点与基线

- DAG 节点：D（line gate）；唯一前置为已通过联合验证的 C checkpoint。
- C baseline：commit `eb53d7c67cfaeecf5eaf74351d0d98690d0d56d2`，tree
  `05c7b5e2e413d31add48879fc4278fd4412963c4`。
- worktree / branch：`/Users/geek/workspace/skiff-rust-test-line-gate` /
  `codex/rust-test-line-gate`。
- 唯一集成 owner：`/root/rust_test_integrator`；本节点不 merge、push 或清理 worktree。
- 当前成熟度：line-gate 开发叶子；完成后仍需由集成 owner 形成 E stable candidate。

## Owner 与写入范围

本节点只可修改：

- `scripts/check-rust-file-lines.mjs` 中的 `MAX_FILE_LINES` 数值及同行 `current maximum` 注释；
- 本文件；
- `rust-large-test-module-refactor-line-gate-result.md`。

不得修改 checker 算法、消息或新增 allowlist/exception；不得修改任何 Rust 文件、A/B 证据文档、生产/测试源码、
manifest、lockfile、schema、配置或集成分支。

## 执行与验收合同

1. 在 C 精确 tree 上列出全部 tracked `.rs` 文件，以 checker 相同的 `wc -l` 口径降序计数，真实最大值决定
   `MAX_FILE_LINES`，不得使用预估值或人为整数阈值。
2. 更新前后保留完整降序计数证据；记录最大路径、最大值、并列情况及 tracked Rust 文件总数。
3. clean 状态下核对 tracked Rust path 集合与 checker 使用的 `rg --files --glob '*.rs'` 集合完全一致；若不一致，
   精确记录差集，不改算法。
4. 更新后运行 checker，要求 PASS 且输出 limit 精确等于真实最大值；以临时、不提交的 max-1 检查证明最大文件会被识别。
5. 运行 `git diff --check`、精确写集审计和 clean 提交审计；无需 Cargo 或 full verify。
6. result 记录 baseline、命令、完整 top 排行/最大值来源、集合一致性、证据、停止条件和最终 commit/tree。

## 停止条件

若完成任务需要修改 checker 算法/消息、引入例外、修改 Rust/生产/测试文件或超出上述写集，则以
`TASK_SCOPE_EXPANDED` 停止。若 C commit/tree 不可达或身份不匹配，则以 `TASK_NOT_EXECUTABLE` 停止。tracked 与
`rg` 集合不一致或最大值并列本身不阻塞，但必须在 result 中精确记录。
