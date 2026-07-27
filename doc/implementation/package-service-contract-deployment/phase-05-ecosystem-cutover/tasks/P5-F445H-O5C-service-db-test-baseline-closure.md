# P5-F445H-O5C Service-DB hermetic test baseline closure

状态：Ready。O5R2 暴露的两个 required-full-gate基线问题的独立测试owner。本节点与O6并行，
只修测试fixture与live分类，不修改任何production行为。

## 直接父节点

- `P5-F445H-O5R2-service-db-prepared-runtime-operation-result.md`

production prerequisite 为 Skiff integration `69ba325a`。父结果已经精确定位：

1. `provider_input`把publication-style `service_id`同时当作Mongo `state_namespace`，导致本应合法
   的opaque provider fixture被Mongo database name校验拒绝；
2. `service_db_runtime_create_and_find_runtime_roundtrips_local_interface`是未标记的真实Mongo
   测试，与任务要求的hermetic full gate冲突。

本任务文件完整描述本节点需求；不得借机修改provider namespace规则、Mongo校验、recoverable
行为或O5R2生产实现。

## 目标

### 1. 修正provider fixture的两个身份

`provider_input`必须分别提供：

- `service_id`：继续使用现有publication-style service identity，证明provider build接收真实
  service id；
- `state_namespace`：使用确定性、合法的Mongo database namespace，不含`/`、`\`、`.`、空格、
  `$`或NUL等禁用字符，也不依赖当前进程、网络或Mongo实例。

不得把production校验放宽，也不得让测试通过删除`state_namespace`验证。invalid opaque config
矩阵继续使用同一fixture，并保持当前错误优先级。

### 2. 把真实Mongo roundtrip明确归类为live

`service_db_runtime_create_and_find_runtime_roundtrips_local_interface`必须：

- 保留测试逻辑，不删除覆盖；
- 使用Rust标准的显式ignored/live分类，使普通
  `cargo test -p skiff-runtime-service-db --locked --no-fail-fast`不连接
  `127.0.0.1:27017`；
- ignore reason明确写出需要本地Mongo replica set/真实网络资源；
- 可用精确的`--ignored` + 完整测试名显式运行，但本任务不得实际运行；
- 不引入新的env fallback、默认连接、secret文件、test harness或Cargo feature。

这是低层adapter live测试的分类，不新增Skiff language test service或修改用户级test模型。

### 3. Hermetic full gate

完成后普通service-db full suite必须：

- 全部非ignored测试通过；
- 真实Mongo测试显示为ignored且未poll连接；
- O5R2 `prepared_runtime` 11项继续通过；
- provider valid/invalid config测试继续实际执行，不能被过滤或ignore；
- 无其它测试因本改动改变选择、计数或语义。

## Test-first 与验收

先用现状复现fixture失败，但不得运行真实Mongo测试。可以使用精确selector或
`--skip service_db_runtime_create_and_find_runtime_roundtrips_local_interface`得到原始单一失败。

修正后运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo test -p skiff-runtime-service-db \
    mongo_provider_builds_db_capability_source_from_valid_opaque_config -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo test -p skiff-runtime-service-db prepared_runtime -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo test -p skiff-runtime-service-db --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo check -p skiff-runtime-service-db --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline/build/cargo-target \
  cargo fmt --check
git diff --check
```

结果记录：

- red与green的实际测试数；
- full suite passed/failed/ignored数；
- ignored测试的精确显式运行命令，但明确本节点没有执行；
- 证明普通full gate没有建立Mongo连接或访问网络；
- production diff为零。

零测试selector不算证据。不得启动Mongo、stable、live、instance，连接网络，读取本机secret，
或修改任何稳定instance配置。

## 写集与停止规则

只允许：

- `runtime/service-db/src/tests.rs`
- 本 result

只允许对`tests.rs`做以下两类最小修改：

1. `provider_input`的`state_namespace` fixture值；
2. 真实Mongo roundtrip测试的显式ignore/live标记。

不得顺手拆文件、改其它fixture、production、Cargo manifest、lockfile或文档。若hermetic full
suite出现第三类失败，或正确分类必须修改test runner/feature/production，立即停止并提交
`TASK_SCOPE_EXPANDED`，记录准确失败，不自行扩大范围。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o5c-test-baseline
branch   codex/p5-f445h-o5c-test-baseline
```

先提交test修改，再单独提交
`P5-F445H-O5C-service-db-test-baseline-closure-result.md`。最终worktree clean；不得
merge/rebase/push。

本任务很小且完整，不派子 Agent。探查后若范围超出预期，立即结束并如实上报。
