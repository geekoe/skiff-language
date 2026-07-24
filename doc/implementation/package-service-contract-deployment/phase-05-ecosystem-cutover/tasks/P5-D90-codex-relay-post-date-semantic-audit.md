# P5-D90：Codex Relay Date 后语义审计

状态：Ready（只读）

## 父节点

- `P5-F156-exact-native-route-context-parity-result.md`
- Date实现：`P5-F155-date-from-epoch-native-semantics-result.md`

## 目标

- 用当前integration重跑Codex Relay真实artifact，确认Date与registry parity后availability变化。
- 定位下一首个unknown/effect/alias/identity来源；逐binding核对signature、handler、required context与规范。
- 若触达resource-backed stream/sse，明确现有设计是否足以决定provenance/lifecycle；不足则只返回最小用户决策。
- 不做consumer workaround或prefix semantics。

只读，不修改、不提交、不操作stable。

