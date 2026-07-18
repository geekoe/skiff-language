# P2-R10E：Std / Prelude Schema Fixtures

状态：R10 consumer batch 4；依赖 R10A，从 R10B 动态拆出，可与 R10B/R10C 并行。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Package 编译”
“依赖与 Identity”。

## Ownership

只迁移以下 integration targets，消费 frozen R10 shared fixture：

- `package_std_schema.rs`
- `prelude_std_schema.rs`

禁止修改 `common/**`、R10B/C/D targets、production、Cargo 或 driver test-support；不得复制 fixture helper。

## 完成态

1. std/prelude schema 测试只读取 canonical PackageArtifact/File IR/package dependency closure。
2. 删除 service publication aggregate/legacy unit 断言；仍属于 package std/prelude 的 type/schema 语义保留。
3. 两个 targets 独立 PASS；旧 helper、`PackageUnit`/`ServiceUnit`/`runtime_units` 零命中。

## 验证

- 两个 target tests、反向搜索、targeted rustfmt、`git diff --check`；不重复 check+test，不跑 full gate。

提交 clean；回报逐 target disposition、测试与 blocker。
