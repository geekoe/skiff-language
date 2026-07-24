# P5-D88：Codex Relay 下一 Unknown Call 审计

状态：Ready（只读）

## 父节点

- `P5-F154-http-request-native-transfer-probes-result.md`

## 目标

- 用当前Skiff integration重跑Codex Relay isolated publish并记录17个operation最新availability/reasons。
- 确认headers/cookie污染消失，从真实handler调用图定位下一首个exact或dynamic unknown target。
- 对`std.http.stream`、`std.http.sse`或其他native逐个核对signature、runtime行为与semantics registry；不得按prefix批量推断。
- 返回最小owner实现节点与正负探针；若语义无法从现有规范/实现确定，给出最小用户决策。

只读，不修改、不提交、不操作stable。

