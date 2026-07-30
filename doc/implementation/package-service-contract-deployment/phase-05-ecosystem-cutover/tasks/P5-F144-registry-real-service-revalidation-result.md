# P5-F144：Registry 真实 Service 重验结果

结论：`TASK_NOT_EXECUTABLE`

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 共享 blocker

隔离 store 已存在 canonical `skiff.run/std@1.0.0` record/pointer，Registry artifact 也声明精确 std ABI，但真实
`npm run test:registry` 在 generated deployment 阶段失败：

```text
exact package requirement skiff.run/std@1.0.0 is unbound
```

该问题属于共享 Skiff package/deployment closure owner。

## Registry 后续 owner 工作

失败前真实 artifact 显示 20 个 intended projections 中 12 Available、8 Unavailable：

- 四个 `*Put`：`unknownCallTarget`、`returnsCallerAlias`
- 四个 `*PointerCas`：`unknownEffect`、`returnsCallerAlias`、`requiresSameHeapIdentity`

共享 blocker 闭合后必须用新叶子任务继续 Registry owner 修复和真实 immutable storage 测试。

