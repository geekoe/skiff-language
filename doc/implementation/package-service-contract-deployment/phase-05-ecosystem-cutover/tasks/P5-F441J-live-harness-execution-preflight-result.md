# P5-F441J Live harness execution preflight result

状态：`PASS / READ_ONLY_EXECUTION_OWNERS_PROVEN`。

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

本节点只读检查 F441I 之后的 current test-runner、encrypted-storage harness 和 runtime-live
plan，冻结后继实现所需的 base assembly、generation 与参数 owner。没有修改 production、scripts、
fixture 或 live 状态，没有启动 instance、Mongo、Router、Runtime、stable 或任何网络 workload。

## 1. 输入与引用链

- 检查代码状态：`c926424ff5e6455d153e0a678047a4131d6df9a9`。
- 直接代码事实父节点：
  - `P5-F441I-canonical-live-source-root-authoring-result.md`
  - `P5-F441H-test-service-profile-target-environment-separation-result.md`
  - `P5-F441A-external-control-file-discovery-result.md`
- 上述父节点继续追溯到 F440A/F440H/F440M 和唯一 package/service contract deployment
  权威设计；本节点不新增架构语义。

## 2. `--base-assembly` 的精确边界

### Encrypted default service

`runtime/encrypted-storage-live/default-service/internal/encrypted.live.test.skiff` 属于 ordinary
service，live execution **必须**传 `--base-assembly <identity>`：

- `canonical_package::compile_package_project_for_test` 只为 `kind: test` 读取固定
  `config.skiff-test.yml`；ordinary service不会把 `config.dev.yml`直接投影进test overlay。
- ordinary test deployment的production config/policy owner由 base assembly中 implementation
  精确匹配的deployment继承。
- 该测试调用normal source拥有的`encryptedLive.testRunnerSecret`；没有匹配base deployment时，
  config binding为空，deployment projection fail closed。

`mapped-service` 当前没有 `.live.test.skiff`，不生成 test-runner invocation。

base identity必须来自同一canonical artifact root内、同时包含 encrypted dependency package、
default service与mapped service的真实build receipt：

```text
receipt.runtimeAssemblyReceipt.assembly.assemblyIdentity
```

不得从service/version、pointer、generation或文件名推测identity。

### Runtime live test service

`runtime/live-tests/internal/{db_live,file_live,http_adapter,operation}.live.test.skiff` 属于
`kind: test`：

- 固定读取 `config.skiff-test.yml`；
- per-case state由test assembly自动生成；
- runtime capability从test package closure投影；
- 当前manifest没有service requirement。

因此这些invocation不需要base assembly，后继plan不得新增base input。当前CLI没有禁止为
`kind: test`传冗余base；“强制拒绝”若需要属于新的设计/实现，不在本阶段暗加。

关键证明入口：

- `test-runner/src/lib.rs`
- `test-runner/src/canonical_package.rs`
- `test-runner/src/package_test_assembly.rs`
- `test-runner/src/canonical_store.rs`
- `test-runner/src/runtime_execution.rs`
- `deployment/src/projection/requirements/config.rs`

## 3. Canonical live runner参数

直接调用runner的current参数集合为：

```text
cargo run --locked --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-test-runner -- \
  <explicit .live.test.skiff> \
  --artifact-root <existing canonical store> \
  --platform-source-root <absolute skiff root> \
  [--base-assembly <identity> only for encrypted default] \
  --live \
  --activation-url http://<control>/__skiff/activate-assembly \
  --ingress-url http://<ingress-origin> \
  --environment <target environment> \
  --expected-generation <current generation> \
  --deny-skips \
  --require-tests
```

`--environment`仍是target environment，不选择test-service profile。runtime-live plan当前无base的
参数集合已经基本正确。

encrypted harness必须删除：

- runner参数`--allow-network`、`--config`；
-临时`test-runner-live.json`和per-run service DB/config注入；
- `SKIFF_DEV_RELOAD_URL`、`SKIFF_TEST_ARTIFACT_ROOT`、
  `SKIFF_TEST_SYNC_CLEANUP`、`SKIFF_TEST_DB_CLEANUP_SETTLE_MS`；
- dev-sync参数`--build-root`、`--default-packages-dir`、`--no-reload`；
- legacy reload/sync路径；
- 业务请求上的`service`/`version`query与
  `x-skiff-service`/`x-skiff-version`header。

固定marker来自tracked `config.dev.yml`，不得恢复动态
`sk-live-test-runner-secret`注入。

## 4. Activation与generation owner

base assembly只提供ordinary production binding候选；test-runner激活test assembly后，active roots只含
test deployments，不会继续保留default/mapped production roots。encrypted harness因此必须保存真实
production assembly identity，并在每次test invocation后显式重新激活该assembly；harness同时拥有并严格
推进expected generation，不能依赖legacy reload或猜测当前generation。

runtime-live有四个顺序invocation。每次成功activation都会推进一代，因此plan必须从caller提供的`N`开始，
按确定排序生成`N`、`N+1`、`N+2`、`N+3`，不能给四个phase重复同一
`--expected-generation`。

## 5. 独立后继

本结果解除两个互不重叠的实现节点：

1. encrypted-storage live harness canonical command/base restore；
2. runtime-live plan canonical generation与直接plan tests。

F441I另行保留的两个test-runner execution blocker不属于上述script owner：

- `operation.live.test.skiff`需要非null `__skiffPayload`；
- file over-limit case需要expected-platform-error执行语义。

在独立test-runner execution节点关闭前，不得宣称完整runtime-live workload已可执行；script节点也不得用
skip、删除case或放宽`--deny-skips`/`--require-tests`掩盖它们。
