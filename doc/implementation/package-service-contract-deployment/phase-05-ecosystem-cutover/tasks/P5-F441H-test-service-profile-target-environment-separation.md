# P5-F441H Test service profile / target environment separation

状态：Ready。F441C 的确定性前置修复；完成后恢复 live-root 迁移。

## 直接父节点

- `P5-F441C-external-live-root-migration-result.md`
- `P5-F441B-external-canonical-fixtures-result.md`
- `doc/reference/testing.md`
- `doc/reference/service-yml.md`

实现基线为 `c45aecb3c5290f470285f26fa8ab2af9a776739c`
（tree `ad90d17f31c0af8b8915decbf05a72e00281d181`）。

F441C 已证明 `runtime/live-tests` 必须是 `kind: test`，但当时的 test-runner 把 runtime target
environment 同时当作 config profile。testing reference 现已关闭该分叉：

- test service config profile 固定为 `skiff-test`；
- 只读取 `config.skiff-test.yml` 及可选、ignored 的 `config.skiff-test.secret.yml`；
- live target environment 仍是 Router/Runtime activation generation 的显式标识，可以是 `dev`
  或其它 canonical token；
- target environment 不得选择 `config.<environment>.yml`。

## 目标

在 test-runner 中彻底拆开两个 owner：

1. `kind: test` 的 package compile path 无条件选择固定 profile `skiff-test`；
2. compile API 不再接收可变 environment/profile 参数，避免其它 caller 重新引入动态 profile；
3. `SkiffTestOptions` 中 runtime 侧字段命名为 `target_environment`，只供 activation body、receipt 与
   readiness 校验使用；
4. 外部 CLI 继续使用 `--environment`，non-live harness 继续使用 `SKIFF_TEST_ENVIRONMENT`，wire JSON
   继续使用 `environment`；这些都是 target environment，不改名、不删除；
5. ordinary package/service compile 行为保持不变。

允许在 `canonical_package` 内建立唯一常量，例如
`TEST_SERVICE_CONFIG_PROFILE: &str = "skiff-test"`；不得在 CLI、runtime execution 和测试中复制第二个
profile owner。

## 唯一写集

- `test-runner/src/canonical_package.rs`
- `test-runner/src/canonical_package/tests.rs`
- `test-runner/src/lib.rs`
- `test-runner/src/main.rs`
- `test-runner/src/runtime_execution.rs`
- `test-runner/src/runtime_execution/tests/**`
- `test-runner/tests/package_service_contract_deployment.rs`
- 本 leaf result

只允许为函数签名或字段重命名机械更新上述范围内的直接 call site。禁止修改 scripts、live roots、
compiler、Router/Runtime production、public reference、其它 task/result、stable/live 状态。
不得派子 agent。

## 必须关闭的错误形态

当前错误调用：

```rust
compile_package_project_for_test(
    &options.platform_sources,
    &package_root,
    artifact_root,
    &options.environment,
)
```

会在 `--environment dev` 时要求 `config.dev.yml`。终态 compile call 不得再看到 target environment；
`compile_package_project_for_test` 也不得保留允许调用方传 `"other"` 的参数。

错误消息不得再使用 “selected test environment” 或 `<test-environment>` 表述 profile 缺失。缺少固定
profile 时应明确报告 `config.skiff-test.yml`。

## 测试先行与验收

先添加或修改测试，使旧实现至少因以下一个断言失败，再实现：

- test service 同时存在 `config.skiff-test.yml` 与 `config.dev.yml` 时，即使 target environment 是
  `dev`，投影值仍来自 `config.skiff-test.yml`；
- test service 只有 `config.dev.yml` 时，即使 target environment 是 `dev`，仍因缺少
  `config.skiff-test.yml` fail closed；
- activation request 和 readiness target 仍精确携带 `dev`，证明 target environment 没有被固定成
  `skiff-test`；
- CLI 的 live `--environment` 要求与 non-live harness target 注入规则保持原样。

必跑：

```bash
cargo test -p skiff-test-runner --lib canonical_package
cargo test -p skiff-test-runner --lib runtime_execution
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  test_service
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  runtime_target_environment
cargo fmt --all -- --check
git diff --check
```

若 selector 名称需要按实际测试名调整，result 必须记录实际 listing 与 execution count。Cargo 命令统一使用：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

反向搜索：

```bash
rg -n 'selected test environment|<test-environment>|compile_package_project_for_test\([^;]*environment' \
  test-runner
rg -n 'config\.\{?environment|config\.dev\.yml' test-runner/src
```

测试 fixture 中用于证明 fail-closed 的 `config.dev.yml` 可以保留；production compile path 不得动态拼接
target environment。

## 停止与交付

若固定 profile 需要改变 CLI target 参数、activation wire、scripts 或 live plan，返回
`TASK_SCOPE_EXPANDED`，不得越界修改。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f441h-test-profile-environment`
- branch：`codex/p5-f441h-test-profile-environment`
- result：`P5-F441H-test-service-profile-target-environment-separation-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push，不访问 stable/live。
