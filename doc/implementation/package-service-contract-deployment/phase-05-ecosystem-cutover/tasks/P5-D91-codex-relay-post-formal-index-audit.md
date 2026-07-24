# P5-D91：Codex Relay Formal-index 后审计

状态：Ready（只读）

## 父节点

- `P5-F157-formal-index-aware-call-transfer-result.md`

## 目标

- 重跑真实Codex Relay artifact，记录17 intended最新Available与reasons。
- 确认withRequestCors alias/identity污染消失。
- 对剩余首个污染逐call site定位；若存在互不依赖exact native semantics可拆并行owner。
- 若触达stream/sse lifecycle且现有规范不足，返回最小用户决策，不自行定义。

只读，不修改、不提交、不操作stable。

