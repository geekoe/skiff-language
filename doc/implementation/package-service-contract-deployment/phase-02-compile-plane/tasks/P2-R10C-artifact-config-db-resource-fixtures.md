# P2-R10C：Artifact / Config / DB / Resource Fixtures

状态：R10 consumer batch 2；依赖 R10A，可与 R10B/R10D 并行。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Package 编译”
“依赖与 Identity”。

## Ownership

只迁移以下 integration targets，消费 R10 shared fixture，不修改 `common/**`、production 或 Cargo target list：

- `artifact_model_conformance.rs`
- `config_shape.rs`
- `db_process_metadata.rs`
- `package_unit_single_path.rs`（终态改为 PackageArtifact 单路径）
- `provider_connect_packages.rs`
- `publication_resources.rs`（只保留 package static resource/build identity）
- `test_artifact_identity.rs`

## 完成态

1. 保留 canonical artifact/config/logical DB/resource/identity 断言；删除 service namespace、collection mapping、
   provider binding、deployment shell 与 PackageUnit 聚合断言。
2. 每个删除断言给出现有/新替代覆盖或明确延后 owner，不建立兼容 holder。
3. 本批 targets 可独立 check/test；旧 service publication helper 与 legacy unit 零命中。

## 验证

- 以本批 `cargo test --test <target>` 为主；只有无需执行的 target 才用 check。另做反向搜索、targeted
  rustfmt、`git diff --check`；不重复 check+test，不跑全量 gate。

提交 clean；回报断言 disposition 表。
