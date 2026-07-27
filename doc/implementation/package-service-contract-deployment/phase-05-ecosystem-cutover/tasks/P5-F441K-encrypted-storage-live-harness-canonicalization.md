# P5-F441K Encrypted-storage live harness canonicalization

状态：Ready。只改 encrypted-storage harness，不运行live workload。

## 直接父节点

- `P5-F441J-live-harness-execution-preflight-result.md`
- `P5-F441I-canonical-live-source-root-authoring-result.md`
- `P5-F441H-test-service-profile-target-environment-separation-result.md`

父节点已证明ordinary default test需要同一artifact root内的真实production base assembly，并冻结current
runner参数、production assembly恢复与generation ownership。引用链继续追溯到唯一权威设计；本leaf不得
重新决定test profile、base semantics或external HTTP选择方式。

实现基线为`83543c1cd21bbb454750cbf5ee6e1d51ada987f0`。

## DAG位置与目标

本节点是F441J解除的encrypted harness consumer。完成后，encrypted live harness应只使用current canonical
artifact/assembly/activation/test接口：

1. 一次canonical build-only输入同时包含encrypted dependency package、default service与mapped service；
2. 从真实JSON receipt严格读取并保存production assembly identity；
3. 使用显式artifact root、platform source root、base assembly、activation URL、ingress URL、target
   environment与expected generation调用default `.live.test.skiff`；
4. test activation后恢复保存的production assembly，并由harness严格推进generation；
5. 对业务HTTP直接使用manifest host/path，不再携带legacy service/version selector；
6. 不再生成per-run JSON config，也不保留legacy reload、artifact env或dev-sync参数。

完成本节点只形成non-live实现检查点；真实encrypted workload仍由最终live gate的唯一owner运行。

## 唯一写集

- `scripts/lib/encrypted-storage-live-harness.mjs`
- `scripts/check-db-encrypted-storage-live.mjs`
- `scripts/tests/encrypted-storage-live-harness.test.mjs`
- `scripts/tests/platform-source-transport-combined.test.mjs`
- 本leaf result

禁止修改source roots、test-runner、verify live plan/registry、Compiler、Router/Runtime production、其它
fixture/task/result、instance/stable配置。不得启动Mongo、Router、Runtime、telemetry、watch、stable或
任何live workload。不得派子Agent。

## 必须删除的legacy surface

- runner `--allow-network`、`--config`与临时`test-runner-live.json`；
- `SKIFF_DEV_RELOAD_URL`、`SKIFF_TEST_ARTIFACT_ROOT`、
  `SKIFF_TEST_SYNC_CLEANUP`、`SKIFF_TEST_DB_CLEANUP_SETTLE_MS`；
- dev-sync `--build-root`、`--default-packages-dir`、`--no-reload`；
- `reload-artifacts`/legacy sync调用链；
- 业务请求query中的`service`/`version`及
  `x-skiff-service`/`x-skiff-version`headers；
- 动态`sk-live-test-runner-secret`注入。

不得以alias、双写或“暂时兼容”保留旧路径。

## Canonical命令与生命周期

`encryptedStorageTestRunnerArgs`或等价唯一helper必须生成current runner参数：

```text
cargo run --locked --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-test-runner -- \
  <explicit test file> \
  --artifact-root <canonical store> \
  --platform-source-root <absolute repo root> \
  --base-assembly <production assembly identity> \
  --live \
  --activation-url <control>/__skiff/activate-assembly \
  --ingress-url <ingress origin> \
  --environment dev \
  --expected-generation <owned current generation> \
  --deny-skips \
  --require-tests
```

build-only必须显式`--environment dev`并解析真实receipt；receipt缺失、identity缺失/非canonical或roots不完整
时fail closed，不能猜identity。

harness必须把每次成功activation视为generation推进。test-runner成功后，即使后续storage observation或
cleanup失败，也必须在可安全判定generation的前提下尝试恢复production assembly；恢复失败应保留原失败与
恢复失败的可诊断信息，不得静默继续。使用仓库现有canonical activation request/receipt规则，不另造wire。

禁止修改runner使base deployment常驻test roots；恢复production assembly是本harness owner。

## 测试先行与验证

先改direct test使旧参数/旧selector至少一项失败，再实现。至少覆盖：

- runner args精确且每个singleton参数只出现一次；
- base identity与expected generation来自caller-owned state；
- build-only command只含current参数并覆盖三个required roots；
- malformed/missing receipt fail closed；
- test成功后恢复production assembly且generation顺序正确；
- test或restore失败的错误聚合；
- direct ingress request不再添加legacy service/version selector；
- reverse assertions证明旧参数/env不存在。

只运行non-live测试：

```bash
node --test \
  scripts/tests/encrypted-storage-live-harness.test.mjs \
  scripts/tests/platform-source-transport-combined.test.mjs
node --check scripts/lib/encrypted-storage-live-harness.mjs
node --check scripts/check-db-encrypted-storage-live.mjs
node scripts/check-db-encrypted-storage-live.mjs --help
git diff --check
```

若现有test seam无法在不启动进程/网络的情况下观察build receipt、activation或restore，应在本写集内提取最小
纯helper并测试；不得用live验证代替direct test。

## 停止与交付

若current activation/generation规则需要修改Router、test-runner或source profile，返回
`TASK_SCOPE_EXPANDED`并给出精确路径/调用链；不得越界。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441k-encrypted-live-harness`
- branch：`codex/p5-f441k-encrypted-live-harness`
- result：`P5-F441K-encrypted-storage-live-harness-canonicalization-result.md`

Implementation与result分开提交；不merge/rebase/push。
