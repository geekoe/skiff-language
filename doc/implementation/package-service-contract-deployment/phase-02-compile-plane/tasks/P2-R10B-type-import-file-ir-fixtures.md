# P2-R10B：Type / Import / File IR Fixtures

状态：R10 consumer batch 1；依赖 R10A，可与 R10C/R10D 并行。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Package 编译”
“Compiler 与 Projection 流水线”。

## Ownership

只迁移以下 integration targets，消费 R10 shared fixture，不修改 `common/**`、production 或 Cargo target list：

- `connect_mongo_package.rs`
- `package_imports.rs`
- `package_std_schema.rs`
- `prelude_std_schema.rs`
- `root_path_references.rs`
- `runtime_slots.rs`
- `streams_emit.rs`

## 完成态

1. 测试只观察 canonical PackageArtifact/File IR/type/effect/import 语义，不读取 service aggregate。
2. 不恢复旧 CLI/service publication helper；重复 setup 必须回到 R10 owner，而不是批次内复制。
3. 本批 targets 可独立 check/test；旧 `PackageUnit`/`ServiceUnit`/`runtime_units` 零命中。

## 验证

- 以本批 `cargo test --test <target>` 为主；只有无需执行的 target 才用 check。另做反向搜索、targeted
  rustfmt、`git diff --check`；不重复 check+test，不跑全量 gate。

提交 clean；回报每个 target 的迁移/删除断言映射。
