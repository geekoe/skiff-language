# P5-F145：Codex Relay 真实 Service 重验结果

结论：`TASK_NOT_EXECUTABLE`

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 共享 blockers

真实 isolated graph 在 Codex Relay publish 失败：

```text
service API schema type CodexRelayProxyClient uses unsupported boundary type non-materializable
```

公开 instance interface 的方法参数 `std.http.HttpRequest` 被 callback-interface schema projector纳入 materialization。
临时只读诊断越过该点后，又命中与 F144 相同的隐式 std requirement unbound。显式把 std 加到 `package.yml` 会被
compiler 正确拒绝，因此不能在 consumer 侧补依赖。

两个共享 blocker 闭合后必须用新叶子任务继续 17/17 operation 与 30 routes 重验。

