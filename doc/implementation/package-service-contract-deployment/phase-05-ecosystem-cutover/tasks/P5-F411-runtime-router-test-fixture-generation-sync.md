# P5-F411 Runtime, Router and test fixture generation sync

状态：Ready。

## 直接父节点

- `P5-F407-service-calls-shared-schema-model-checkpoint-result.md`
- `P5-F405-linker-public-execution-type-coordinate-validation-result.md`
- `P5-F406-isolated-log-evidence-mock-repair-result.md`

F407要求所有reader/fixture切到PackageArtifact v8与build v9。F405/F406位于clean
`codex/p5-f386-package-test-http-gateway`候选，已分别通过linker 9/9和Node 3/3+4/4；本节点把仍有效的
owned实现移植到新model，不整分支合并。

## DAG位置与候选

- DAG节点：F407后的runtime/router/test consumer同步。
- start commit：`288a105fc87399c5e93228ee9f2ba2e58c4cd2b6`。
- 与F408/F410并行；完成后等待F409及集成gate。
- 风险：高；Runtime admission、Router filesystem loader与test-runner真实路径。

## 独占写入范围

```text
runtime/{loader,linker,eval,host,package-test}/**
router/**
test-runner/**
test-services/**
scripts/run-skiff-tests.mjs
scripts/tests/run-skiff-tests-error-evidence.test.mjs
scripts/tests/isolated-test-runtime-log-evidence.test.mjs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/tests/package-service-ecosystem-http-fixture.test.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/package-service-authoring.test.mjs
本任务result
```

禁止修改artifact model/identity、compiler production、deployment、ecosystem repos或权威设计。

## 精确候选移植

从`codex/p5-f386-package-test-http-gateway`只移植仍缺失且属于上述owner的实现，不得merge整个分支，
不得带入F399 compiler patch或已在integration存在的Router patch-equivalent提交：

- test-runner T1/T2 owned commits：
  `89ffbeca`、`4eccda2b`、`804f2f9c`、`03e81923`；
- diagnostic：
  `a9de53ee6194703e429bc946920d4a2ef3db2d28`；
- evidence mock：
  `8ad4db47b79581e4264148f61eadd084d00feb13`；
- linker coordinate fix：
  `593dcfe1edb741adf5afc66d965183b5a4712769`。

先比较patch与当前tree；只移植缺失hunks。若cherry-pick有跨owner冲突，不得引入候选其它文件，改为在
本owner语义移植并记录映射。

## 必须实现

1. 所有owned PackageArtifact literals删除`service_call_roots`并切v8；Package build literals/prefix切v9。
2. Runtime loader/linker/eval/host不得新增selection reader；`service_call_refs`、call-site dispatch与exact
   deployment binding必须保留。
3. Router filesystem loader与compatibility fixture只接受PackageArtifact v8/build v9，无dual-read。
4. 保留F405语义：publication `ServiceSymbol`与execution `LocalType`先解析为exact linked coordinate，
   再比较完整descriptor；wrong package/file/type、missing/ambiguous/tamper/`boolean`仍拒绝。把其tests
   机械适配v8，不放宽。
5. test-runner package-service provider：
   - `api.yml` function改为scalar `echo: main.handle`；
   - `service.yml.serviceCalls: [echo]`；
   - contract仍精确1 operation。
6. zero-operation test services保持missing/empty `serviceCalls`，HTTP gateway不进入contract。
7. 保留T1/T2的新test-service、strict control、inline setup、package service dependency实现，不恢复
   test-doubles旧写法。
8. `run-skiff-tests`继续输出去敏bounded isolated evidence；startup mock保持当前完整seam。
9. 更新package-service authoring Node fixture到新`api.yml`/`service.yml`选择；不得修改F408 parser。

## 聚焦验证

使用integration共享Cargo target；不得在本worktree生成第二份全量target。至少运行并记录实际计数：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-linker service_error_index
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-runtime-loader
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner --lib runtime_execution -- --test-threads=1
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-test-runner \
    --test package_service_contract_deployment -- --test-threads=1
node --test scripts/tests/run-skiff-tests-error-evidence.test.mjs
node --test scripts/tests/isolated-test-runtime-log-evidence.test.mjs
node --test scripts/tests/package-service-ecosystem-http-fixture.test.mjs
```

运行与generation相关的Router聚焦tests及scoped typecheck，名称以当前package scripts为准并在result记录。
运行`cargo fmt --all -- --check`与`git diff --check`。

本节点不运行完整`node scripts/run-skiff-tests.mjs`；那是F408/F409/F410/F411合流后的新gate owner。
不得操作stable/live，不得派子Agent。

## 交付

写`P5-F411-runtime-router-test-fixture-generation-sync-result.md`，记录exact start/end commit/tree、
候选commit→新model hunk映射、v8/v9反向搜索、测试计数和仍被上游遮挡的范围。提交并保持clean，不
merge/rebase/push。
