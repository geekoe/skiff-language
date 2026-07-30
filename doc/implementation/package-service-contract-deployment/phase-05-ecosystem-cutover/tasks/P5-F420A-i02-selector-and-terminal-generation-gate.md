# P5-F420A I02 selector and terminal-generation gate

状态：Ready（F420 scope-expansion 后继）。

## 直接父节点

- `P5-F420-suspension-router-tooling-generation-result.md`

F420 已在原 write set 内完成 Router、tooling、golden、dynamic fixture 与 test-runner 的 terminal
generation 机械迁移，但按工作流在发现唯一范围外 owner 后停止。修改后的 Node 五文件组为
35/36；唯一首错是 I02 real runner 仍按旧扁平字段读取 current v2 HTTP selector。本节点修复该
唯一 production owner，并在 exact checkpoint 上完成 F420 全部未跑门禁。

## 精确起点

- integrated start：
  `56501394220cf0751b599990761323402bbd0582`；
- F420 implementation checkpoint：
  `5b4391eba8f19919b93a80ccdb637eb47a2585dc`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时记录 HEAD/tree，并证明三个 commit 均为 HEAD ancestor。先复现 F420 result 的 Node
五文件组唯一失败，再修改。

## 写入范围

唯一新增 production owner：

```text
scripts/lib/package-service-i02-combined-real.mjs
```

为了完成原节点门禁，若验证暴露 F420 checkpoint 自身的机械遗漏，可继续修正 F420 原授权范围：

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

这不是授权重新设计或重写 F420；除 `package-service-i02-combined-real.mjs` 外，只允许由规定门禁
直接证明的 checkpoint 漏改。禁止修改 artifact model/identity、compiler、deployment、runtime、
test-runner production、std 或 ecosystem 仓库。不得派子 Agent、merge/rebase/push、访问
stable/live、instance 或 watch registry。

## 必须修复

current v2 entrypoint 的 HTTP 地址只存在于：

```text
entrypoint.selector.protocol
entrypoint.selector.host
entrypoint.selector.method
entrypoint.selector.path
```

`requestTypedUnary` 必须：

1. 明确校验 selector 是 HTTP，且 host/method/path 是合法的 current 字段；
2. 只用 `selector.method`、`selector.path`、`selector.host` 构造真实请求；
3. 不读取 `unary.method/path/host`，不接受扁平旧形态，不增加 fallback、adapter 或双写；
4. 保持两次零 artifact-I/O 请求、withdrawal 与 rollback 事务语义不变；
5. 由现有 I02 real-owner test 实际证明 `/probe` 请求，不通过伪造 fixture 绕开 real runner。

## 完整验证

所有 Cargo 命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

先 listing，再 execution。必须运行：

```bash
node --test \
  scripts/tests/artifact-identity-validation.test.mjs \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/runtime-execution-boundary-checker.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs

node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs

pnpm --filter @skiff/router exec vitest list \
  tests/compilerGeneratedManifestCompatibility.test.ts \
  tests/dynamic-build-id-parity.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/protocol.test.ts \
  tests/artifacts.test.ts
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

还要反搜并分类：

- current positive：PackageArtifact v9、Local ABI v7、build v10、PackageUnit v2、
  ServiceContract/protocol v5、RuntimeAssembly v2；
- legacy negative：protocol v4、PackageArtifact v8、canonical build v9、PackageUnit v1、
  legacy build v2、old contract/interface fields；
- current I02 real runner 中旧扁平 `unary.method/path/host` 为 0；
- concrete callable `may_suspend`、service selection、exact operation identity 与
  F415 `collection_name_mapping` 仍保留。

## 交付

实现与
`P5-F420A-i02-selector-and-terminal-generation-gate-result.md` 分开提交。result 记录 exact
commit/tree、修改前首错、selector 修复、每条 listing/execution 的实际计数、dynamic fixture
producer/record、current/legacy inventory，以及 F421 是否解除。保持 clean；不 merge/rebase/push。

若出现第二个超出上述授权范围的 production owner，或语义不再是 current shape 的机械收敛，立即
`TASK_SCOPE_EXPANDED` 停止并上报，不得继续扩范围。
