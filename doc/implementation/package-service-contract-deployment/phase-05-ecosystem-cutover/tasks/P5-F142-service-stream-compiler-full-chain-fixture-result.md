# P5-F142：Service Stream Compiler Full-chain Fixture 结果

结论：`TASK_NOT_EXECUTABLE`

## 父节点链

- 直接父节点：`P5-D82-service-call-stream-capability-audit-result.md`
- 前置结果：P5-F139、P5-F141。
- 整条链可追溯到唯一权威设计。

## 已确认 blocker

真实 provider package（公开 `Event` / `Request` 与 `events() -> Stream<Event>`）能完成 package compile，但
`project_service_api` 返回：

```text
MissingPublicTypeSource { public_path: "Event" }
```

原因：

- canonical implementation-link type key 由
  `compiler/projection/src/package_artifact/export_links/mod.rs` 以公开 `package_symbol` 写入；真实 key 是 `Event`、
  `Request`。
- `compiler/contract/src/projection.rs::project_boundary_schema` 却要求 key 能剥离
  `"<package-id>/"` 前缀后等于 public path。
- compiler/contract 现有 unit fixtures 使用 prefixed key，掩盖了真实 pipeline key shape。

因此 F142 的测试文件写入范围不能修复 production owner。临时 probe 已移除，未提交生产或测试改动。修复应先落在
compiler/contract canonical lookup 和相邻 fixtures，再重新执行 F142。

