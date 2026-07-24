# P5-F143：Contract Public Type Source Canonical Key

状态：Ready

## 父节点与进入状态

- 直接父节点：`P5-F142-service-stream-compiler-full-chain-fixture-result.md`。
- 父节点向上追溯到 D82 result、审计合同和唯一权威设计。
- 当前 blocker 是已确认的 production implementation-link key owner 不一致，不涉及新架构语义。

## Owner 与真实 key

- canonical producer：
  `compiler/projection/src/package_artifact/export_links/mod.rs`，以公开 `package_symbol` 作为
  `implementation_links.types` key。
- consumer：
  `compiler/contract/src/projection.rs::project_boundary_schema`。
- 真实 compiler pipeline key 是 `Event` / `Request` 等 public path；现有 prefixed unit fixtures 不是 canonical 生产形状。

## 写入范围

- `compiler/contract/src/projection.rs` 及其同文件 tests。
- 禁止修改 canonical producer、artifact schema、compiler source/lowering、Runtime。

## 完成标准

1. `project_boundary_schema` 按精确 public path 查找 type implementation link，并继续校验 descriptor/source identity；
   不做 suffix search、fallback 或 dual-read。
2. 将直接触碰的 prefixed unit fixtures 改为 canonical public-path keys。
3. 真实 `Event`/`Request` key 正例通过；缺失 key、错误 descriptor、仅存在 prefixed legacy key 均 fail closed。
4. 搜索同一 consumer 内 package-id prefix inference 残留；不得为旧 fixture 保留兼容。

## 验证与证据

- 先 `--list` 确认 compiler-contract projection selector，随后运行该聚焦测试；零测试无效。
- 目标文件格式与 `git diff --check`；不运行完整 gate。
- 风险：高，contract public type identity/source closure。
- 若 canonical producer 与其他 production consumer 对 key 仍有矛盾，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f143`
- branch `codex/p5-f143-contract-public-type-key`
- 新的一次性开发会话；提交、不 push、不操作 stable。

