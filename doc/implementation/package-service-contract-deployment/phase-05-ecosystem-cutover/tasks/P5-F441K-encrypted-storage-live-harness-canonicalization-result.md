# P5-F441K Encrypted-storage live harness canonicalization result

状态：`PASS / NON_LIVE_CANONICAL_HARNESS_CHECKPOINT`。

## 1. 输入、提交与写集

- 任务声明 implementation baseline：
  `83543c1cd21bbb454750cbf5ee6e1d51ada987f0`
  （tree `25d7b42418694b34e510c29592026aa867a92cdd`）。
- leaf dispatch HEAD：
  `0543aa9a6c29fc599565134eb2e78e0c53de614b`
  （tree `f23ff5b150eb4b6c9dbee3082aad61c410f65541`）。
- implementation：
  `db4a32b55b3ffbf5ca1070c05781fb8cd5be87b4`
  （tree `e356879264fd573727e82d67d7b0b4b56a3a045f`）。

Implementation只修改任务允许的四个script/test文件：

- `scripts/lib/encrypted-storage-live-harness.mjs`；
- `scripts/check-db-encrypted-storage-live.mjs`；
- `scripts/tests/encrypted-storage-live-harness.test.mjs`；
- `scripts/tests/platform-source-transport-combined.test.mjs`。

没有修改source roots、test-runner、verify live plan/registry、Compiler、Router/Runtime production、
其它fixture/task/result或instance/stable配置。本文由独立result-only commit交付；其commit/tree由最终
交付消息记录。

## 2. Test-first RED

先只把direct runner test改为current canonical参数合同，再执行：

```bash
node --test scripts/tests/encrypted-storage-live-harness.test.mjs
```

旧实现按预期得到`0 passed / 1 failed`。diff精确证明旧helper仍输出：

```text
--allow-network --config undefined
```

并缺少`--locked`、`--quiet`、显式bin、artifact root、base assembly、activation/ingress URL、
target environment、expected generation、`--deny-skips`与`--require-tests`。随后才扩充direct seam并
修改production harness。

## 3. 终态实现

### 3.1 Canonical authoring与receipt

- 唯一build helper一次向`skiff-dev-sync.mjs`传入encrypted dependency package、default service与
  mapped service三个显式root；
- build固定为`--environment dev --build-only --json`，不再传build root、default package dir、
  reload或sync参数；
- stdout必须解析为真实JSON receipt；
- receipt必须精确包含三个PackageArtifact坐标、两个ServiceDeployment service id、`dev`
  RuntimeAssembly receipt及唯一canonical v2 assembly identity；任一缺失、重复、非canonical或额外
  root都会fail closed；
- fresh managed Mongo在Router/Runtime启动前，复用仓库既有activation-state seed seam，把该真实
  production assembly安装为caller-owned generation `0`；没有猜测identity或依赖pointer/file name。

### 3.2 Runner、restore与generation

runner命令现在精确为：

```text
cargo run --locked --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-test-runner -- \
  <explicit test file> \
  --artifact-root <canonical store> \
  --platform-source-root <absolute repo root> \
  --base-assembly <saved production identity> \
  --live \
  --activation-url <control>/__skiff/activate-assembly \
  --ingress-url <ingress origin> \
  --environment dev \
  --expected-generation <caller-owned current generation> \
  --deny-skips \
  --require-tests
```

`runEncryptedStorageTestLifecycle`在同一个caller-owned state中推进generation：

1. runner使用当前`N`并在成功activation后推进为`N+1`；
2. transient storage observation/cleanup完成或失败后，仍尝试以`N+1`恢复保存的production assembly；
3. canonical activation request/receipt成功后推进为`N+2`；
4. runner失败时只在control health严格证明generation已经是`N+1`且没有pending activation后恢复；
5. test、observation、cleanup与restore错误用`AggregateError`保留原始cause和全部诊断。

direct test冻结了`8 -> 9 -> 10`顺序，并覆盖runner失败后generation可证明且restore同时失败的双错误聚合。

### 3.3 Ingress与retired surface

- business POST只构造manifest path相对显式ingress origin的URL；
- headers只保留`content-type`及可选rotation token；
- query不再携带service/version，headers不再携带service/version selector；
- outer live test读取tracked `config.dev.yml`中的固定
  `encrypted-live-test-runner-secret`，不再生成per-run JSON config或动态secret；
- `--help`在创建harness、租端口或启动任何component前返回usage。

## 4. 证据矩阵

| 任务条款 | 代码证据 | direct / reverse证据 |
| --- | --- | --- |
| 三root canonical build-only | `encryptedStorageBuildArgs`与`buildProductionAssembly` | 精确argv test；三个`--root`；旧dev-sync参数为0 |
| receipt/identity/roots fail closed | `encryptedStorageProductionAssembly` | missing receipt、missing/noncanonical identity、missing package/service root均拒绝 |
| caller-owned base与generation | `activationState`及`runEncryptedStorageTestLifecycle` | runner收到saved base与generation `8` |
| test后恢复production | `restoreProductionAssembly`复用canonical request及receipt validator | 事件顺序`test@8 -> restore@9`，终态generation `10` |
| 后置失败仍恢复、错误聚合 | lifecycle `Promise.allSettled`与`AggregateError` | observation失败仍restore；runner+restore双失败均保留 |
| direct manifest ingress | `encryptedStorageIngressRequest`及两个business调用方 | URL无query；无legacy selector headers |
| retired surface完全删除 | harness/check source reverse assertion | 规定legacy flag/env/reload/selector/secret字符串0命中 |
| platform source owner不分叉 | `encryptedStorageTestRunnerArgs` | combined transport test证明唯一absolute repo root |

## 5. Non-live验证

| 命令 | 结果 |
| --- | --- |
| 两个规定Node test文件 | PASS，9 passed / 0 failed |
| `node --check scripts/lib/encrypted-storage-live-harness.mjs` | PASS |
| `node --check scripts/check-db-encrypted-storage-live.mjs` | PASS |
| `node scripts/check-db-encrypted-storage-live.mjs --help` | PASS，仅输出usage |
| `git diff --check` | PASS |

规定reverse search：

```bash
rg -n \
  'allow-network|test-runner-live|SKIFF_DEV_RELOAD_URL|SKIFF_TEST_ARTIFACT_ROOT|SKIFF_TEST_SYNC_CLEANUP|SKIFF_TEST_DB_CLEANUP_SETTLE_MS|build-root|default-packages-dir|no-reload|reload-artifacts|x-skiff-service|x-skiff-version|sk-live-test-runner-secret' \
  scripts/lib/encrypted-storage-live-harness.mjs \
  scripts/check-db-encrypted-storage-live.mjs
```

为0命中（`rg` status 1）。

## 6. 隔离与收尾

- 未运行`db-encrypted-storage-live`或任何其它live selector/workload；
- 未启动或访问Mongo、Router、Runtime、telemetry、watch、instance、stable或固定端口；
- 未派sub-agent，未merge、rebase或push；
- current activation/generation规则可完全由harness既有写集消费，没有触发`TASK_SCOPE_EXPANDED`。
