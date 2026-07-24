# P5-F142：Service Stream Compiler Full-chain Fixture

状态：Ready

## 父节点与进入状态

- 直接父节点：`P5-D82-service-call-stream-capability-audit-result.md`。
- 必要前置结果：
  - `P5-F139-service-stream-boundary-projection-result.md`
  - `P5-F141-contract-stream-call-source-typing-result.md`
- 上述链均可追溯到唯一权威设计。
- 进入 checkpoint：provider projection 与 caller source typing 已分别通过，但尚无同一真实 compiler pipeline fixture
  证明 contract projection、consumer lowering 和 precise `ServiceCallRef` 接通。

## Owner 与入口

- 真实 compiler conformance 入口：
  `compiler/tests/service_conformance.rs`。
- 该入口已经用 provider/consumer package、生成 contract dependency、File IR call site 和
  `validate_file_ir_service_calls` 证明 unary 链；本任务增加等价 server-stream 链，不另造测试框架。
- production lowering owner已存在；本任务默认只补证据，不重写 lowering。

## 写入范围

- `compiler/tests/service_conformance.rs`
- 仅测试需要时可调整该文件现有 fixture helper。
- 若真实 fixture 暴露 production 缺陷，返回精确 blocker，不修改 compiler production crates。

## 完成标准

1. provider package 的公开 `Stream<NominalItem>` callable 投影为 Available `ServerStream`，生成 contract 保留 canonical
   item nominal identity/value plan。
2. consumer package 用精确 service dependency alias 在 `for` 中消费该 operation，并完成真实 compiler pipeline。
3. artifact 与 File IR 都包含同一个精确 `ServiceCallRef`：requirement slot、operation id、protocol identity 一致；
   `validate_file_ir_service_calls` PASS。
4. consumer 不携带 provider implementation binding；HTTP stream owner不参与链路。
5. 增加一个错误 alias/item identity 或非法嵌套 stream 负例，证明 fail closed。

## 验证与证据

- 先运行
  `cargo test -p skiff-compiler --test service_conformance -- --list`
  确认 selector 命中，再运行该 test target 的聚焦测试；零测试无效。
- 目标文件格式与 `git diff --check`；不运行完整 gate。
- 风险：高，共享 compiler full-chain probe。
- 若需要改变公共 schema、stream lifecycle 或跨 production owner，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f142`
- branch `codex/p5-f142-service-stream-compiler-fixture`
- 新的一次性开发会话；提交、不 push、不操作 stable。
