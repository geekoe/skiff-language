# P5-F441C External live root migration result

状态：`DECISION_REQUIRED / PRE_IMPLEMENTATION_STOP`。

本 leaf 按任务的显式停止条件在任何 implementation 写入前停止：三个 root 的 role / `kind`
可以从当前调用链唯一证明，但 `runtime/live-tests` 实际应拥有哪个 tracked
`config.<profile>.yml` 无法从当前调用链唯一确定。本文只记录只读证据和唯一决策问题；没有迁移
service root、harness 或 verify test，没有运行 live、instance、stable 或任何 workload。

## 1. 输入与隔离

- 任务声明的 implementation checkpoint：`67d61b8db9cb1750fe624dc40b9968642fb6d7f3`
  （tree `6ffd7924e0e7359e3ffd2f05635bd724a2d961ff`）。
- dispatch HEAD：`a33b4810aefab9b1ad60f5aaddce3b07cb53487e`
  （tree `7de8adf011739af8b912803335e9232de518716d`）。
- 已完整读取直接父结果 F440A、F440H、F440M；没有读取并行 sibling F441A 的未完成结果或复制其改动。
- 只做 tracked source / task / result 的静态读取；没有读取 `.skiff-instance`、watch registry、
  runtime target、固定端口或 live target。

## 2. `kind` 与 encrypted profile 已唯一证明

### 2.1 encrypted default / mapped

当前调用链为：

1. `scripts/check-db-encrypted-storage-live.mjs::run` 创建
   `EncryptedStorageLiveHarness`；
2. `EncryptedStorageLiveHarness.initialize` 分别调用
   `seedServiceArtifacts("default-service")` 与
   `seedServiceArtifacts("mapped-service")`；
3. `seedServiceArtifacts` 调用 `scripts/skiff-dev-sync.mjs --root ...`；
4. `runDevSyncOnce` 对每个 root 固定调用 compiler `kind: "package"` /
   `action: "publish"`；
5. `compiler/driver/authoring.rs::build_package_after_platform_context_guard`
   对该 ordinary publish/watch 路径显式拒绝 `service.yml kind: test`。

因此两个 root 必须是 ordinary service；省略 `kind` 精确反序列化为
`ServiceAuthoringKind::Service`。任务又已精确冻结两者的 tracked profile 为
`config.dev.yml`、`timeout: 120000`。harness 当前没有把 `--environment dev` 写进命令，而是走
dev-sync registry environment；后继实现应按已冻结目标把这一选择显式化，不能依赖本机 registry。

### 2.2 runtime/live-tests

当前 `runtime-live` selector 的调用链为：

1. `scripts/lib/verify-live-plan.mjs::runtimeFixturePhases` 为每个
   `*.live.test.skiff` 生成 test-runner 命令；
2. 命令没有 `--base-assembly`，但传入 `--environment <runtimeLiveEnvironment>`；
3. `test-runner/src/lib.rs::run_skiff_tests_with_options` 把该 environment 传给
   `compile_package_project_for_test`；
4. `test-runner/src/canonical_package.rs::read_test_service_profile` 只有在
   `service.yml kind: test` 时才按 environment 读取 tracked profile；
5. ordinary service 在当前无 base assembly 的路径只能得到 empty/default package-test bindings，
   而当前 runtime-live tests 明确读取 config、database state 与 runtime capability，不能形成等价
   可执行输入。

因此 `runtime/live-tests/service.yml` 必须显式为 `kind: test`。这不是从目录名或
`.live.test.skiff` 后缀猜测，而是当前 test-runner role/profile 调用链的唯一可执行分支。

## 3. runtime-live profile 无法唯一确定

当前代码保留三个不等价候选，且没有 owner 选择其中一个：

| 候选 | 当前证据 | 缺失的唯一性 |
| --- | --- | --- |
| `runtime-live` | 三个 direct verify test helper 均以 `runtimeLiveEnvironment: "runtime-live"` 构造 positive plan；CLI tests也使用同名值 | 这些只是调用方示例/fixture，production plan没有把它冻结为 canonical constant |
| `dev` | 较早 canonical live-root audit给出三个 root 的 `config.dev.yml`目标；encrypted lane也确实使用dev authoring | 当前 external runtime-live selector不默认、也不强制 `dev` |
| caller-selected token | `verify-live-registry.mjs`允许 CLI `--runtime-live-environment` 或环境变量 `SKIFF_RUNTIME_LIVE_ENVIRONMENT`；`verify-live-plan.mjs`只校验任意 canonical ASCII token并原样转发 | repo无法把一个 `timeout: 120000` 写入所有可能的 `config.<token>.yml`；当前 plan也不验证所选 tracked profile存在 |

精确调用点：

- `scripts/lib/verify-live-registry.mjs` 的 `LIVE_INPUTS.runtimeEnvironment` 把 profile owner留给
  CLI / process environment；
- `scripts/lib/verify-live-plan.mjs::resolveRequiredInputs` 选择该任意值；
- `inspectRuntimeFixtureState` 只应用
  `/^[A-Za-z0-9._-]{1,200}$/`，不约束为 `dev` 或 `runtime-live`，也不检查
  `config.<environment>.yml`；
- `runtimeFixturePhases` 将它原样放入 test-runner `--environment`；
- `test-runner/src/canonical_package.rs::read_test_service_profile` 用它直接索引
  `service.config_profiles`，缺失时 terminal 报
  `MissingTestServiceProfile`。

F440A 直接父结果也明确保留了同一分叉：通常使用 `config.runtime-live.yml`；若继续允许任意
environment，workflow 必须先证明对应 tracked profile存在。当前 task / production call chain没有
关闭这项选择。

## 4. 需要用户决定的唯一问题

**`runtime-live` 是否应固定使用 canonical profile `runtime-live`（新增
`runtime/live-tests/config.runtime-live.yml`，并让 canonical plan 拒绝其它 environment），还是继续
允许 caller-selected environment（则请同时冻结允许的 tracked profile 集及 plan 的存在性规则）？**

若选择固定 `runtime-live`，`dev` 不再是该 selector 的候选；若选择 caller-selected，必须明确是否
至少同时 tracked `dev` / `runtime-live`，不能由本 leaf自行创造 profile policy。

## 5. 未执行项

由于停止发生在 test-first 与 migration 之前：

- 没有 implementation commit；不会用空提交伪造 implementation；
- 三个 root 仍保持 legacy layout，canonical 40-ingress receipt尚不存在；
- 未运行任务列出的 direct tests、checker plan、syntax或receipt验证，因为它们只能验证尚未获决策的
  implementation；
- 未运行 Mongo、Router、Runtime、telemetry、watch、instance、stable或任何 live selector；
- 未 merge、rebase或push。

只读取证后运行 `git diff --check`；本文由独立 result-only commit交付。
