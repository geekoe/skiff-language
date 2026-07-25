# P5-F294R Applied nominal task contract review

状态：Ready。

只读审查：

- `P5-F294-applied-nominal-shared-model.md`
- 直接父 `P5-F293-generic-nominal-type-ref-owner-audit-result.md`

判断该叶子合同是否：

1. 完整且唯一地冻结审计推荐的 applied nominal wire；
2. 写入范围覆盖 shared DTO、strict admission与所有 identity generation owner；
3. 与运行中的 F286 compiler范围零重叠；
4. version/identity变化与保持项精确；
5. 测试能够证明 non-empty、single representation、owner/arity、nested traversal、branch去重和mutation；
6. 没有暗中要求 compiler/runtime/public generic schema实现；
7. 可由一个开发Agent在授权范围内完成。

只返回 `PASS` 或 `BLOCKED`。若 blocked，列出精确缺项、错误owner与最小任务文件修正；不修改任何文件，
不运行build/test，不自行实现。
