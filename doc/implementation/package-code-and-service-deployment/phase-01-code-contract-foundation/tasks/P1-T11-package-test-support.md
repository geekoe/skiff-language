# P1-T11：让 Package Test 消费新 Code Contract

状态：`ready`
类型：Package test runtime / test runner
依赖：P1-T10
执行者：Package Test Agent，一份提交

## 目标

更新 package-test artifact builder、runtime package-test loader 和 `skiff test` runner，使其严格
读取/校验新的 PackageUnit code contract，并能为带 service requirement 的 package传入可信
service artifact root。本阶段只验证编译和 artifact 装配，不执行 service dependency。

## 当前桥接边界

package-test 目前会把 production/test PackageUnit files 装配进 synthetic ServiceUnit/
`service_files` 以复用现有 runtime。Phase 01 可以保留这个桥接；它只能承载新的 typed code facts，
不能成为第二个 boundary/effect/service requirement owner。Phase 02/03 会删除该结构。

## 行为

- PackageUnit 新字段在 package test assembly 中被严格校验，不以 default 补旧 artifact。
- production/test overlay 不改写 production package identities。
- 带 service requirements 的 package test compile 继续要求
  `--profile dev --service-artifact-root <root>`；缺失时错误明确。
- 有效 root 时 compiler 能解析 provider contract并生成 assembly。
- test runtime 若实际触发 service dependency call，Phase 01 应给“尚未支持本地 service
  assembly”的显式错误；不得静默走 router、伪造 provider、生成 stub 或把它当 package call。

## 范围

- `runtime/package-test/src/`
- `test-runner/src/package.rs`、`artifacts.rs`、必要 CLI/root plumbing
- `test-runner/tests/test_runner_package*.rs` 与聚焦 fixture
- 仅为构造新 PackageUnit 必需的 shared test builders

## 非目标

- 不修改普通 runtime loader/linker。
- 不实现 Runtime Assembly、local service dispatch 或 router fallback。
- 不删除 synthetic ServiceUnit/service_files 桥接。
- 不跑 live service/router/runtime instance。

## 必须测试

- 新 PackageUnit 字段缺失/identity不符时 package-test fail closed。
- package requirement 缺 artifact root失败；有效 root编译/装配成功。
- service dependency call 被执行时显式 unsupported，而非隐式远程。
- production identity/path 不因 test-only file或 metadata变化。
- local-only mutable helper仍能在 package test中执行。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-runtime-package-test
cargo test --no-fail-fast -p skiff-test-runner test_runner_package
node scripts/skiff.mjs test <本任务新增的最小 package fixture> \
  --profile dev \
  --service-artifact-root <本任务临时 fixture root>
git diff --check
```

最后一条应使用测试自带临时 fixture，不依赖本机 stable instance。若 CLI test harness 已覆盖同一
行为，可用精确 test filter替代，并在提交记录中说明。

## 验收标准

- package-test/runner 不重算 effect、boundary eligibility 或 identity。
- 新 artifact validation fail closed。
- 带 service requirement 的 package可完成编译和装配。
- 无生产 router/network fallback。

## 停止条件

- 只有实现 service call 执行才能测试 artifact compile；
- package-test 必须复制 compiler resolver/projector；
- synthetic service桥接无法承载新字段且必须改变普通 runtime production path。

以上应上报并重新调整 Phase 01/02 边界。

## 提交

提交信息建议：`feat(package-test): consume package code contracts`
