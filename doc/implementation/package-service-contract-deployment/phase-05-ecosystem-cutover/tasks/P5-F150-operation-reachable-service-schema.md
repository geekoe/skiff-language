# P5-F150：Operation-reachable Service Schema

状态：Ready

## 父节点

- `P5-D84-instance-interface-service-schema-audit-result.md`

## 写入范围与 owner

- `compiler/contract/src/projection.rs` 及同文件 tests。
- 不修改 source instance projection、consumer authoring、artifact schema 或 Runtime。

## 完成标准

1. 先索引 public type source/id，再从 operation parameter/return/error/stream/callback ContractTypeId seeds lazy project。
2. 每个 reachable shape递归扩 closure；reachable interface仍严格投影 callback descriptor并 fail closed。
3. 未被 operation/callback引用的 public interface，即使含 non-materializable method type，也不进入或阻断 ServiceContract。
4. instance method operations保持 Available；不能通过删除 public Package API绕过。
5. 覆盖 unreachable invalid interface正例、同 interface作为 callback seed的负例，以及 closure缺失/重复 fail closed。

## 验证

- 先列出 compiler-contract projection selector；运行聚焦测试、格式与 `git diff --check`。
- 不运行完整 gate；若需公共 schema语义改变则停止。

## Worktree

- `/Users/geek/workspace/skiff-p5-f150`
- branch `codex/p5-f150-operation-reachable-schema`
- 新的一次性会话；提交、不 push、不操作 stable。

