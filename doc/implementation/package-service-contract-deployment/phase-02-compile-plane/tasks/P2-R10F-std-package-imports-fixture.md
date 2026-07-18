# P2-R10F：Std Package Imports Fixture

状态：R10 acceptance blocker；依赖 R10G，T07 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Package 编译”
“依赖与 Identity”。

## 目标与 ownership

- 迁移 `compiler/tests/std_package_imports.rs` 到 frozen canonical package fixture 与 R10G file-write API。
- 独占该 target；禁止修改 common、production、Cargo、其它 tests 或恢复 driver test-support。

## 完成态

1. 保留属于 canonical std/package dependency、typed File IR、PackageRequirement/local ABI 的断言。
2. 删除 service assembly/config/provider/deployment aggregate 断言，并给出替代覆盖或 Phase 03–05 owner。
3. `build_temp_service_publication`、`test_support`、legacy unit/runtime holder、service/package assembly 零命中。
4. target test PASS，且 `cargo check --tests -p skiff-compiler` 不再被该 target 阻断。

## 验证

- `std_package_imports` target test、旧 helper/aggregate 反向搜索、targeted rustfmt、`git diff --check`；
  `cargo check --tests` 的最终重跑由独立 R10 acceptance owner执行一次。

提交 clean；回报逐断言 disposition、测试与 blocker。
