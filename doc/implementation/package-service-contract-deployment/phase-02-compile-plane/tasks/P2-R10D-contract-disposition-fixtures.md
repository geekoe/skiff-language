# P2-R10D：Contract / Disposition Fixtures

状态：R10 consumer batch 3；依赖 R10A，可与 R10B/R10C 并行。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“ServiceContract 编译”
“部署与调用语义”“Fail-closed 条件”。

## Ownership

- 迁移 `compiler/tests/service_conformance.rs` 为显式、code-free ServiceContract fixture。
- 在 `compiler/Cargo.toml` 删除以下 terminal Phase 02 不再拥有的 integration targets，并记录断言 disposition：
  `artifact_output`、`compiler_command`、`http_routes`、`profile_overlay`、`service_config_overlay`。
- 删除退役且未导出的 `compiler/driver/test_support.rs` 与 `compiler/driver/test_support/**`。
- 禁止修改 `compiler/tests/common/**`、其它 consumer targets或 production compile 语义。

## 完成态

1. service conformance 只走 `ServiceContractDefinition -> ServiceContract`，不从 provider/package 推导 contract。
2. 删除 target 的 package/File IR/identity 等价断言映射到 R10B/R10C；route/deployment/config overlay 语义明确
   延后 Phase 03–05，不保留 fake target 或 compatibility test-support。
3. driver test-support 物理删除，`build_service_publication`、`build_temp_service_publication`、旧 CLI binary env、
   `PackageUnit`/`ServiceUnit` 在本 ownership 零命中。

## 验证

- `cargo test -p skiff-compiler --test service_conformance`、Cargo target 审计、反向搜索、targeted rustfmt、
  `git diff --check`；不运行全量 compiler gate。

提交 clean；回报显式 contract coverage 与逐 target disposition。
