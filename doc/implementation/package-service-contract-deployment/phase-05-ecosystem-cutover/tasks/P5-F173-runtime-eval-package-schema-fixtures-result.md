# P5-F173：Runtime Eval Package Schema Fixtures Result

状态：Completed

## 直接父任务

- `P5-F173-runtime-eval-package-schema-fixtures.md`

## 交付

- 修复三个eval通用`PackageArtifact`测试fixture：
  - `assembly_execution/ordinary/tests.rs`
  - `assembly_execution/projection.rs`
  - `spawn_ops/canonical_tests.rs`
- 三个fixture均没有Package public named type，因此显式声明合法空schema：
  - `package_schema_index`使用fixture自身`package_id`；
  - index identity由`package_schema_index_identity(package_id, empty_types)`真实计算；
  - `package_schema_type_records`显式为空。
- 未使用默认值绕过identity校验，未改变fixture public API、contract requirements或任何生产逻辑。

## 验证

通过：

```text
git diff --check
```

已执行：

```text
cargo test --locked -p skiff-runtime-eval --no-run
```

编译输出不再包含三个目标fixture缺少`package_schema_index`或
`package_schema_type_records`的错误，已越过本任务断面。测试二进制随后仍被独立F172 WebSocket
断面阻断：首错位于`assembly_execution/websocket_contract_plan.rs`，并包括
`websocket_ingress.rs`与`websocket_response.rs`仍引用旧`ContractTypeId`、
`ContractSchemaType`、`boundary_schema`和旧enum variants。由于eval library本身尚不能编译，
当前没有可单独运行的eval聚焦测试；按任务边界未修改这些生产文件。
