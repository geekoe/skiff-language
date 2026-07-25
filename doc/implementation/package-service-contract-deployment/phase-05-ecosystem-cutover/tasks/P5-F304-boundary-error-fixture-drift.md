# P5-F304 Removed boundary error fixture migration

状态：Completed。结果见
`P5-F304-boundary-error-fixture-drift-result.md`。

## 直接父节点

- failure classification：
  `P5-F303-compiler-probe-failure-classification-result.md`
- failed probe：`P5-F302-applied-nominal-compiler-combined-probe-result.md`

## DAG位置与边界

- 节点：F302-B1 mechanical fixture drift。
- 与等待用户决策的std WebSocket production branch、F299 runtime carrier并行。
- 完成后只关闭F302中`file_ir_execution_type_representation`的编译遮挡；不单独解除A2/F269。
- 这是低风险机械节点，不是稳定候选。

## 唯一写入范围

- `compiler/tests/file_ir_execution_type_representation.rs`
- `compiler/tests/service_conformance.rs`
- `compiler/tests/shared_fixture_lane_probes.rs`
- `compiler/tests/websocket_ingress.rs`
- `compiler/driver/ecosystem_store/tests/fixtures.rs`

禁止修改production、其它tests、artifact/runtime/std/文档。

## 完成标准

- 删除五处旧`BoundaryErrorContract` import；
- 删除五处`BoundaryOperationContract { errors: ... }`字段；
- 其它fixture值、operation contract字段与测试断言逐字保持，不用新placeholder替代；
- compiler生产/测试反搜不再有`BoundaryErrorContract`或`.errors`旧构造；
- 不修改open error channel、public generic或service contract语义。

## 验证owner

```bash
cargo test -p skiff-compiler --test file_ir_execution_type_representation -- --list
cargo test -p skiff-compiler --test service_conformance -- --list
cargo test -p skiff-compiler --test shared_fixture_lane_probes -- --list
cargo test -p skiff-compiler --test websocket_ingress -- --list
cargo test -p skiff-compiler --lib -- --list
git diff --check
```

selector必须非零。若运行语义被F302-B2遮挡，只记录精确首错，不越界修复。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f304-boundary-fixtures`
- branch：`codex/p5-f304-boundary-fixtures`
- 新的一次性开发Agent；5分钟内完成首次修改；
- 提交并返回commit、五处迁移、反搜与验证；不push、不操作stable、不承接其它节点。
