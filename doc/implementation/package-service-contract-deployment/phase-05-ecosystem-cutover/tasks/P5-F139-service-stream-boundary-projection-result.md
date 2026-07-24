# P5-F139：Service Stream Boundary Projection 结果

结论：PASS

## 父节点链

- 直接父节点：`P5-D82-service-call-stream-capability-audit-result.md`
- 该 result 向上追溯到 P5-D82 审计合同和唯一权威设计。

## 交付

- commit：`7d97f79`，已合入 Phase 5 integration。
- `compiler/projection` 现在把公开 callable 最外层 `Stream<T>` 直接投影为既有
  `BoundaryStreamContract::ServerStream`，保留 canonical item type 和 provider item value plan。
- Stream 参数、嵌套 Stream、collection 内 Stream、错误 arity 与不可 materialize item 继续结构化 fail closed。
- Package public nominal item 保留 local type identity，供 contract projection canonicalize。
- HTTP stream owner/schema 未被复用或修改。

## 证据

- compiler projection boundary：13/13 PASS。
- 正例、关键负例和 deterministic unavailable receipt 均在
  `compiler/projection/src/package_artifact/tests/boundary.rs`。
- `cargo fmt --check`、`git diff --check` PASS。

该结果解除 caller source typing 与真实 lowering fixture；projection、artifact schema 或 public type closure 变化会使
本证据失效。

