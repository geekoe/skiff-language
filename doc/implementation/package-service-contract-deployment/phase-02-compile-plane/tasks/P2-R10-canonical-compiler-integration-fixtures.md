# P2-R10：Canonical Compiler Shared Fixtures

状态：shared test checkpoint；旧 R10 脏 worktree 只作只读证据，禁止整体提交、移植或继续开发。

依赖：T05C10A–G、R03、R04、R06、R11、R13 已合入 terminal integration checkpoint。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Compiler 与 Projection
流水线”“ServiceContract 编译”“Fail-closed 条件”。

## 目标与 ownership

- 建立 integration tests 唯一共享的 canonical package project/compile fixture 与 code-free
  `ServiceContractDefinition -> ServiceContract` fixture。
- 独占 `compiler/tests/common/**`；建议按 `package_project.rs`、`contracts.rs`、artifact 读取职责拆分。
- 删除已无 compiler binary owner 的 `common/cli_command.rs`，不伪造 CLI。
- 禁止修改 production、integration test targets、`compiler/Cargo.toml` 或 driver test-support；消费者由
  R10B/R10C/R10D 迁移。

## 完成态

1. package fixture 以 `package.yml` 为源码根，调用公开 `compile_package`，返回单责 typed result：
   `PublishedPackageArtifact`、精确 File IR/resource 与 canonical dependency artifacts。
2. contract fixture 只接受显式 `ServiceContractDefinition`，不读取 provider source/service config，不构造空 contract。
3. 不返回 `PackageUnit`/`ServiceUnit`/`runtime_units`，不恢复 `build_service_publication` 或万能聚合 builder。
4. canonical compile pipeline、dependency resolution、identity 和 artifact reading 各只有一个 helper owner。

## 验证

- targeted rustfmt、helper 旧名/legacy type 反向搜索、`git diff --check`。
- 可运行能独立编译的 common fixture test/check；consumer targets 在 R10B/C/D 前暂时失败须精确记录，不跑
  compiler 全量 gate。

提交 clean checkpoint；回报 public fixture API、旧 helper disposition 与 R10A probe handoff。R10A 通过前不直接
扇出 R10B/C/D。
