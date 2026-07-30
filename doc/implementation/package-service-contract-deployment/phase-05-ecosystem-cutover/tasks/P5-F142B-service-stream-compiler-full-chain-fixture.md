# P5-F142B：Service Stream Compiler Full-chain Fixture 重验

状态：Ready

## 父节点与进入状态

- 直接父节点：`P5-F143-contract-public-type-source-key-result.md`。
- 父节点向上追溯到 F142 blocker result、D82 result、审计合同和唯一权威设计。
- F142 的 `MissingPublicTypeSource` blocker 已由 F143 闭合；本任务重新建立真实 compiler full-chain probe。

## Owner 与入口

- 唯一写入入口：`compiler/tests/service_conformance.rs`。
- 复用该文件既有 provider/consumer package、生成 contract dependency、File IR call site 与
  `validate_file_ir_service_calls` 模式，不另造框架。
- Production projection/source/contract/lowering 均为已合流前置；本任务只加真实路径证据。

## 完整执行需求

1. provider package 的公开 nominal `Event`/`Request` 与 `events() -> Stream<Event>` 经过真实 compiler pipeline，
   投影为 Available `ServerStream`，contract 保留 canonical item nominal identity/value plan。
2. consumer 通过精确 service dependency alias，在 `for event in alias/events(input)` 中消费，并完成 compile/lowering。
3. artifact 与 File IR 的 `ServiceCallRef` 在 requirement slot、operation id、protocol identity 上精确一致；
   `validate_file_ir_service_calls` PASS。
4. consumer 不携带 provider implementation binding；HTTP stream owner不参与。
5. 至少一个错误 item identity、alias 或非法 nested stream 负例经同入口 fail closed。

## 写入与禁止范围

- 只允许 `compiler/tests/service_conformance.rs` 及该文件现有 helper。
- 禁止 production 修改、workspace-wide formatting 和无关 fixture 改动。
- 若再次暴露 production blocker，清除临时 probe 后返回精确 result，不跨 owner 修复。

## 验证

- `cargo test -p skiff-compiler --test service_conformance -- --list` 必须实际列出新增测试。
- 运行该 test target 聚焦测试、目标文件格式与 `git diff --check`；不运行完整 gate。

## Worktree

- `/Users/geek/workspace/skiff-p5-f142b`
- branch `codex/p5-f142b-service-stream-compiler-fixture`
- 新的一次性开发会话；提交、不 push、不操作 stable。

