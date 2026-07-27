# P5-F441H Test service profile / target environment separation result

状态：`PASS / FIXED_PROFILE_AND_TARGET_ENVIRONMENT_SEPARATED`。

## 1. 输入、提交与写集

- 任务声明 implementation baseline：
  `c45aecb3c5290f470285f26fa8ab2af9a776739c`
  （tree `ad90d17f31c0af8b8915decbf05a72e00281d181`）。
- leaf dispatch HEAD：
  `34acdf0b7bd4ec5e8f9b62537c6cf310cd6d150a`
  （tree `01a81e633bfd85b98f18061f301ae8a6434d5a46`）。
- implementation：
  `e3227f545f4cc54503451dc3f82ed921465c10ee`
  （tree `ffb5f8936dc9b0989f9d5af49952fe76cf379783`）。

Implementation 只修改任务允许的八个 test-runner 文件：

- `test-runner/src/canonical_package.rs` 及 `canonical_package/tests.rs`；
- `test-runner/src/lib.rs`、`main.rs`、`runtime_execution.rs`；
- `test-runner/src/runtime_execution/tests/{orchestration,readiness}.rs`；
- `test-runner/tests/package_service_contract_deployment.rs`。

没有修改 scripts、live roots、Compiler、Router/Runtime production、public reference、其它 task/result
或 stable/live 状态。

## 2. Test-first RED

先只增加/修改 profile separation 测试，再执行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment test_service
```

旧实现按预期得到 `3 tests / 1 passed / 2 failed`：

1. 同时存在 `config.skiff-test.yml` 与冲突的 `config.dev.yml` 时，旧实现把
   `profile_name` 错选为 `dev`，而断言要求 `skiff-test`；
2. 只有 `config.dev.yml` 时，旧实现返回成功 project，而断言要求因缺少
   `config.skiff-test.yml` fail closed。

随后才修改 production。

## 3. 终态实现

- `canonical_package` 内的唯一 profile owner 是
  `TEST_SERVICE_CONFIG_PROFILE = "skiff-test"`。
- `compile_package_project_for_test` 已删除 environment/profile 参数；内部 workflow 只区分 ordinary
  与 test compile，不允许 caller 传入另一个 profile token。
- `kind: test` 的 test workflow 只读取固定 profile；缺失时精确报告
  `requires config.skiff-test.yml`。错误文本不再包含 selected test environment 或
  `<test-environment>`。
- ordinary package/service workflow 仍走原 ordinary 分支；额外执行完整非-live contract integration
  文件验证其行为未回归。
- `SkiffTestOptions.environment` 已更名为 `target_environment`。CLI 仍使用 `--environment`，
  non-live harness 仍读取 `SKIFF_TEST_ENVIRONMENT`，activation/receipt/readiness wire 仍使用
  `environment`。
- activation request 与 readiness target 的新增单元测试都以 `dev` 为精确 target，证明 runtime
  target 没有被固定成 `skiff-test`。

规定的 `runtime_execution` selector 首次运行还暴露同一允许写集内两个既有 stale golden：测试名已是
current F385，内容却仍构造 parser 已拒绝的 GatewayEntry v1 identity。测试 golden 已对齐同一仓库
integration receipt 使用的 current v2 identity；没有修改 production reader、wire 或 dispatch。

## 4. 验证

所有 Cargo 命令均使用共享
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-test-runner --lib canonical_package` | PASS，2 passed / 2 ignored / 39 filtered |
| `cargo test -p skiff-test-runner --lib runtime_execution` | PASS，29 passed / 14 filtered |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment test_service` | PASS，3 passed / 25 filtered |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment runtime_target_environment` | PASS，1 passed / 27 filtered |
| 完整 `cargo test -p skiff-test-runner --test package_service_contract_deployment` | PASS，27 passed / 1 ignored |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

任务给出的两个 integration selector 不需要改名；实际 listing/execution 分别为 3 个 `test_service`
测试和 1 个 `runtime_target_environment` 测试。

## 5. 反向搜索

规定搜索：

```bash
rg -n 'selected test environment|<test-environment>|compile_package_project_for_test\([^;]*environment' \
  test-runner
```

为 0 命中（`rg` status 1）。

第二个规定搜索：

```bash
rg -n 'config\.\{?environment|config\.dev\.yml' test-runner/src
```

只有一个既有命中：

```text
test-runner/src/package_service_host_fixture.rs:171:
target.join(format!("config.{environment}.yml"))
```

该路径为 ordinary provider/consumer host fixture 的 service authoring helper，未被本 leaf 修改，也不由
`compile_package_project_for_test` 或 `kind: test` profile selection 消费；保留它正是任务要求不改变的
ordinary service compile 行为。test service canonical compile path 已无动态 profile 拼接。

## 6. 隔离与收尾

- 未运行 live selector、instance、watch、stable、固定端口或外部 workload；
- CLI contract 测试在 missing input 阶段终止，只验证参数/harness ownership，没有发起网络请求；
- 未派子 agent，未 merge、rebase 或 push；
- 未触发 `TASK_SCOPE_EXPANDED` 停止条件。
