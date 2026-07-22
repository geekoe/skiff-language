# P5-I16：Platform Source Shared-target Combined Probe

## 角色、输入与唯一性

Gate owner只读运行F16A/F16B/F16C合流后的exact clean integration commit/tree；D19必须已`AUDIT CLOSED`，或D19
冻结的F17 exact repair已合流且无在途写入。否则不得启动I16。不得编辑、提交、修复或操作stable。
本任务是platform shared-target动态证据与F04原样Host gate的唯一owner。候选和环境不变时，R16与F04 narrow receive
必须复用该证据，不重复完整gate。

开始前记录free space、Cargo.lock blob、端口/进程与source provenance。容量不足时在启动构建前报告BLOCKED；不得
删除用户或其它任务cache。A/B固定为`/Users/geek/workspace/skiff-p5-i16-a`与`.../skiff-p5-i16-b`的detached
worktree。用`mktemp -d /Users/geek/workspace/.skiff-p5-i16.XXXXXX`创建`I16_TMP_ROOT`，共享target固定为
`$I16_TMP_ROOT/target`。结束后清理两个worktree、端口、进程和整个任务自有临时目录。

## 冻结顺序与命令

先在合流integration运行一次便宜combined检查；任一失败不得创建A/B或运行Host：

```bash
cargo check --locked -p skiff-compiler --bin skiff-compiler -p skiff-test-runner --bins
cargo test --locked -p skiff-compiler-source platform_source_context_preserves_legacy_prelude_identity
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment platform_source_context_contract
node --test scripts/tests/package-service-authoring.test.mjs scripts/tests/skiff-source-test-suite.test.mjs scripts/tests/skiff-test-cli.test.mjs scripts/tests/test-runner-runtime-isolation.test.mjs scripts/tests/encrypted-storage-live-harness.test.mjs
```

F16A/B/C各自的absolute/symlink/missing/cross-root/reserved/omitted/relative/context-mismatch单元矩阵直接消费开发ledger，
I16不重复；combined过滤测试必须引用这些fixture并证明仍在exact合流tree。随后执行：

1. 同一exact commit建立路径不同的A/B worktree，共用一个任务自有`CARGO_TARGET_DIR`。从A build
   与smoke fixture：

   ```bash
   CARGO_TARGET_DIR="$I16_TMP_ROOT/target" cargo build --locked --manifest-path /Users/geek/workspace/skiff-p5-i16-a/test-runner/Cargo.toml --bin skiff-test-runner --bin skiff-package-service-smoke-fixture
   ```

   记录runner、fixture及`skiff_compiler_input/source` rlib的hash/mtime/dep-info。再依次运行：

   ```bash
   SKIFF_TEST_PLATFORM_SOURCE_ROOT=/Users/geek/workspace/skiff-p5-i16-a CARGO_TARGET_DIR="$I16_TMP_ROOT/target" cargo test --locked --manifest-path /Users/geek/workspace/skiff-p5-i16-a/test-runner/Cargo.toml --test package_service_contract_deployment platform_source_identity_probe -- --nocapture
   SKIFF_TEST_PLATFORM_SOURCE_ROOT=/Users/geek/workspace/skiff-p5-i16-b CARGO_TARGET_DIR="$I16_TMP_ROOT/target" cargo test --locked -vv --manifest-path /Users/geek/workspace/skiff-p5-i16-b/test-runner/Cargo.toml --test package_service_contract_deployment platform_source_identity_probe -- --nocapture
   ```

   B必须报告相关crate/test为`Fresh`、hash/mtime不变，且两行prelude identity/std PackageBuildId exact相同。
2. 在任务自有target执行精确clean：

   ```bash
   cargo clean --manifest-path /Users/geek/workspace/skiff-p5-i16-b/Cargo.toml --target-dir "$I16_TMP_ROOT/target" -p skiff-test-runner -p skiff-compiler -p skiff-compiler-input -p skiff-compiler-source
   CARGO_TARGET_DIR="$I16_TMP_ROOT/target" cargo build --locked --manifest-path /Users/geek/workspace/skiff-p5-i16-b/test-runner/Cargo.toml --bin skiff-test-runner --bin skiff-package-service-smoke-fixture
   ```

   接着运行：

   ```bash
   SKIFF_TEST_PLATFORM_SOURCE_ROOT=/Users/geek/workspace/skiff-p5-i16-b CARGO_TARGET_DIR="$I16_TMP_ROOT/target" cargo test --locked --manifest-path /Users/geek/workspace/skiff-p5-i16-b/test-runner/Cargo.toml --test package_service_contract_deployment platform_source_identity_probe -- --nocapture
   SKIFF_TEST_PLATFORM_SOURCE_ROOT=/Users/geek/workspace/skiff-p5-i16-a CARGO_TARGET_DIR="$I16_TMP_ROOT/target" cargo test --locked -vv --manifest-path /Users/geek/workspace/skiff-p5-i16-a/test-runner/Cargo.toml --test package_service_contract_deployment platform_source_identity_probe -- --nocapture
   ```

   A必须`Fresh`，四次identity输出exact相同。该clean只建立B-origin镜像证据，不是任何PASS前置或修复。
3. 检查production rlib/binary字符串与dep-info没有`compiler/input..std`、`..prelude`或platform用途
   `CARGO_MANIFEST_DIR`；source registry仍唯一std。然后再次用步骤2的精确clean命令清理四个任务crate，从A执行
   步骤1的build，再从B以相同manifest target执行`cargo build --locked -vv`，必须`Fresh`且hash/mtime不变。
   全部cheap/identity/structure证据PASS后，才从任意非repo cwd运行：

   ```bash
   CARGO_TARGET_DIR="$I16_TMP_ROOT/target" node /Users/geek/workspace/skiff-p5-i16-b/scripts/run-skiff-tests.mjs
   ```

   Cargo必须复用A-origin产物，std 11/11与Host 1/1返回exact `provider-observed-helper-mutated`。

任何primary失败立即停止完整gate，不重试掩盖；完整记录首错、阶段、cleanup结果和exact command。FileHandle cleanup
只记录并关联已关闭的D19/F17 ledger，不改变platform verdict。输出PASS/FAIL、commit/tree/lock、A/B路径、target、
hash/mtime、Fresh证据、三次identity输出、std/Host计数与资源清理证明。
