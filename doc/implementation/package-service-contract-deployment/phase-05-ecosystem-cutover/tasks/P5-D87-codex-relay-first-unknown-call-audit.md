# P5-D87：Codex Relay 首个 Unknown Call 审计

状态：Ready（只读）

## 父节点

- 直接父节点：`P5-F152-imported-http-boundary-types-result.md`
- 上游分段事实：`P5-D86-http-ingress-boundary-availability-audit-result.md`

## 目标

- 用当前Skiff integration重新生成Codex Relay artifact，确认unsupportedBoundaryType已消失。
- 从最小literal HTTP handler开始，按exact local helper、registered native、receiver builtin、真实adminSession/v1Proxy链逐级
  定位第一个把`invokesUnknownTarget`或unknown provenance引入transitive summary的call site。
- 检查`compiler/source` resolved_call_targets与callable_effects analysis/transfer；区分artifact projection刻意省略target与source
  analysis真实未知。
- 动态any-interface、未登记native/provider必须继续Unknown；不得建议泛化放宽eligibility。
- 返回可并行的最小既定owner DAG或精确DESIGN_BLOCKED决策。

只读，不修改、不提交、不运行完整gate、不操作stable。

