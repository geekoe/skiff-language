# P5-F420 Suspension Router, tooling and current-generation oracles

状态：Ready（N4）。

## 直接父节点

- `P5-D93-suspension-current-base-reconciliation-audit-result.md`
- `P5-F419G-suspension-consumer-combined-gate-rerun-result.md`

F419G 已在同一 exact candidate 上证明 schema/identity、compiler、deployment 与 runtime 三层联合
实现全部通过，并明确解除 F420。本节点只迁移 Router、tooling、golden 与 test-runner
current positive oracle；不再修改前三层 production。

## 精确起点与启动门禁

- integrated start：
  `b58dbde08a0de76b9c5cf94398df76f5f5717f11`；
- tree：
  `c7d9fd6d10578f483558358519c8b7734e9c064b`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时必须证明 start 为 HEAD ancestor、tree 匹配，且 F415 仍为 ancestor。先执行任务规定的
focused listing/probe，记录真实 current 首错，再修改；不得仅按字符串批量替换。

## 独占写入范围

只允许修改：

```text
router/**
scripts/check-artifact-identity-single-source.mjs
scripts/lib/runtime-execution-boundary-subjects.mjs
scripts/lib/runtime-execution-boundary-self-test.mjs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-authoring.mjs
scripts/lib/skiff-source-test-suite.mjs
scripts/lib/package-service-i02-combined-oracle.mjs
scripts/tests/artifact-identity-validation.test.mjs
scripts/tests/runtime-execution-boundary-checker.test.mjs
scripts/tests/package-service-authoring.test.mjs
scripts/tests/skiff-source-test-suite.test.mjs
scripts/tests/package-service-i02-combined.test.mjs
scripts/tests/package-service-bootstrap-oracle-handoff.mjs
scripts/tests/package-service-ecosystem-http-fixture.test.mjs
scripts/tests/package-service-ecosystem-smoke-real.test.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/platform-source-transport-combined.test.mjs
scripts/tests/run-skiff-tests-error-evidence.test.mjs
scripts/tests/check-artifact-identity-single-source.test.mjs
scripts/tests/verify.test.mjs
cross-system-fixtures/dynamic-build-id-parity/case.json
test-runner/tests/package_service_contract_deployment.rs
本任务 result
```

特别禁止修改：

```text
artifact-model/**
artifact-identity/**
compiler/**
deployment/**
runtime/**
test-runner/src/package_test_assembly.rs
std/**
任何 ecosystem 仓库
```

`canonical_package_bindings` 的 exact collection-name mapping clone 是 F415 已接受语义，不得通过
改 test-runner production 绕过 fixture。不得派子 Agent、merge/rebase/push、访问 stable/live、
instance 或 watch registry。

## 必须收敛的 current 模型

所有 current positive producer、reader、fixture 与断言必须统一为：

| 对象 | current generation |
| --- | --- |
| PackageArtifact | `skiff-package-artifact-v9` |
| Package Local ABI identity | `skiff-package-local-abi-v7:sha256` |
| Package build identity | `skiff-package-build-v10:sha256` |
| PackageUnit | v2 |
| ServiceContract | `skiff-service-contract-v5` |
| Service protocol identity | `skiff-service-protocol-v5:sha256` |
| RuntimeAssembly | v2 |

具体要求：

1. Router direct-join、compiler-generated manifest compatibility 与 filesystem loader 必须直接读取
   terminal fresh records，不增加 dual-read、prefix fallback 或字段兼容。
2. `check-artifact-identity-single-source` 和 runtime-boundary checker 必须只认可当前 canonical
   owner；旧 suspension requirement fields 不得作为允许项。
3. source-suite 与 package-service authoring production validators 接受 PackageUnit v2 /
   RuntimeAssembly v2；移除当前正例中硬编码的 v1，不得改写真实 legacy rejection 负例。
4. I02 combined oracle/fixture 使用 RuntimeAssembly v2、Package build v10，并适配 current receipt
   shape；不得伪造 receipt 或跳过 identity 校验。
5. `cross-system-fixtures/dynamic-build-id-parity/case.json` 必须由当前 terminal compiler 路径真实
   再生；result 记录 producer commit/tree 与 record path，不能手工猜 hash。
6. `test-runner/tests/package_service_contract_deployment.rs` 删除已不存在的 contract/interface
   suspension fields；concrete callable `may_suspend`、service selection、exact operation identity
   与 F415 mapping 断言保留。
7. current positive 路径不得出现旧 `serviceCall:`、旧 contract/interface fields、v4 protocol、
   PackageArtifact v8、canonical build v9、PackageUnit v1 或 legacy build v2。

## 必须保留的失败关闭

以下是负例，不得为了“反搜为零”删除或改成成功：

- Service protocol v4；
- PackageArtifact v8 与 canonical build v9；
- PackageUnit v1 与 legacy build v2；
- 已删除的 contract/interface old fields；
- path escape、duplicate key、mapping drift/collision；
- 当前已有 legacy rejection 文本。

旧 token 允许存在于明确的 rejection fixture/assertion 中；result 必须区分 current positive
producer 与 legacy negative inventory。

## 验证

所有 Cargo 命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

先 listing，再 execution。至少运行：

```bash
node --test \
  scripts/tests/artifact-identity-validation.test.mjs \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/runtime-execution-boundary-checker.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs

node scripts/check-artifact-identity-single-source.mjs

pnpm --filter @skiff/router exec vitest run \
  tests/compilerGeneratedManifestCompatibility.test.ts \
  tests/dynamic-build-id-parity.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/protocol.test.ts \
  tests/artifacts.test.ts

pnpm --filter @skiff/router exec tsc --noEmit --pretty false

cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --list
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1

node scripts/run-skiff-tests.mjs
node scripts/verify.mjs --only router
node scripts/verify.mjs --only tooling

cargo fmt --all -- --check
git diff --check
```

审计基线只用于发现漏跑，不是强行追平：

```text
Router five-file vitest: 164 tests
Node five-file group: 36 tests（审计时 34 pass / 2 个 N4 已知失败）
test-runner package_service_contract_deployment: 24 listed
```

若 current tree 的合法新增/删除使计数变化，记录 test 名和原因；不得删 selector、过滤失败或拿静态
数量代替实际 listing/execution。

## 交付与范围扩张

写 `P5-F420-suspension-router-tooling-generation-result.md`，至少记录：

- exact start、implementation commit/tree、ancestry；
- 修改前真实首错及对应 owner；
- current schema/prefix/record path 清单；
- dynamic fixture 的真实再生命令、producer commit/tree 与 hash；
- Router、Node、test-runner、source-suite、verify 的实际计数；
- current positive 反搜与 legacy negative inventory；
- collection mapping、concrete `may_suspend` 和 service selection 保留证据；
- 所有失败（如有）及是否阻断 F421。

实现与 result 分开提交，保持 worktree clean；不 merge/rebase/push。

若首轮探查发现所需 production owner 超出上述 write set，或多个不明确语义问题使本节点不能在当前
权威模型内机械收敛，立即停止并返回 `TASK_SCOPE_EXPANDED`，说明已完成证据、精确阻断、最小新增
owner 与建议后继任务；不得自行扩范围。
