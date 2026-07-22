# P5-I16：Platform Source Shared-target Combined Probe

## 角色、输入与证据复用

使用未参与F16A/B/C/F17实现、D19审计或R16验收的全新Gate owner会话。输入为F16A/F16B/F16C/F17全部合流、
D19 `DESIGN GO`已关闭、无在途写入的exact clean integration commit/tree；不得编辑、提交、修复或操作stable。
权威设计为`doc/architecture/package-service-contract-deployment.md` §3、§9、§10、§14及阶段标准6；动态矩阵只验证
D18/F16合同，不增加新的trust或CLI语义。

开始前只核对并消费四个开发任务的exact commit/tree/lock、自验收矩阵和D19 ledger，不重复其absolute/symlink/
missing/cross-root/reserved/omitted/relative/context-mismatch、transport或lifecycle聚焦测试。唯一merge-only cheap probe与
A/B动态矩阵由F16C提交的`run-platform-source-shared-target-probe.mjs`拥有；完整source-suite/Host只在该脚本末尾运行
一次。候选和环境不变时，R16与F04 narrow receive必须复用本ledger。

## 唯一命令

Gate owner先核对`git status --short`为空及四项开发ledger有效，然后只执行：

```bash
P5_I16_ROOT=/Users/geek/workspace/skiff-phase-05-integration
P5_I16_COMMIT="$(git -C "$P5_I16_ROOT" rev-parse HEAD)"
P5_I16_TREE="$(git -C "$P5_I16_ROOT" rev-parse HEAD^{tree})"
cd /tmp
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-platform-source-shared-target-probe.mjs \
  --integration-root "$P5_I16_ROOT" \
  --candidate "$P5_I16_COMMIT" \
  --expected-tree "$P5_I16_TREE" \
  --expected-lock-blob f3ce5457138c58aec4c84abda431afa96013e3fd \
  --expected-prelude-identity skiff-prelude-v1:sha256:aae18f07de6746b8cc769ca3bd9db6b65b6c292fc75016549b58cd253b3f3f0d \
  --expected-std-package-build-id skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4 \
  --a-worktree /Users/geek/workspace/skiff-p5-i16-a \
  --b-worktree /Users/geek/workspace/skiff-p5-i16-b \
  --json
```

任何primary失败立即停止且不重试。若candidate/lock不匹配、worktree路径已存在、容量不足或清理前置不成立，脚本
必须在build前返回`PREFLIGHT BLOCKED`。

## Gate harness冻结行为

脚本及其command-double test属于F16C checkpoint；I16不得修改。脚本必须：

1. 复验integration exact commit/tree/lock/clean、A/B路径不存在、无同名worktree。容量门槛为任务target预计占用：
   existing shared Cargo target allocated bytes加2 GiB；若无可测shared target则要求至少8 GiB free。创建
   `/Users/geek/workspace/.skiff-p5-i16.XXXXXX`任务目录、其中唯一shared target，并用`try/finally`/signal handlers只
   清理任务目录和A/B detached worktree；记录清理后ABSENT、端口与残留进程。
2. 在exact合流tree只运行未被开发ledger执行的merge-only
   `node --test scripts/tests/platform-source-transport-combined.test.mjs`，以及一次共同编译
   `cargo check --locked -p skiff-compiler -p skiff-test-runner --bins`。fixture检查compiler
   authoring、runner、smoke bootstrap、source-suite、`skiff test`、runtime-live和encrypted-storage argv共享同一absolute
   root，并直接执行一个omitted-root fail-closed路径。
3. 建A/B detached worktree；共享target依次建立A-origin、B-origin、最终A-origin三轮。targeted clean crate固定为
   `skiff-test-runner`、`skiff-compiler`、`skiff-compiler-input`、`skiff-compiler-source`；每轮命令显式使用对应absolute
   `--manifest-path`、同一`CARGO_TARGET_DIR`、`--locked`及runner/smoke两个binary selector。
4. A-origin下依次以A、B runtime root运行ignored `platform_source_identity_probe`；B-origin下依次以B、A root运行。
   共4次probe，每次输出带标签的2个值（8项），全部exact等于上述两个golden。跨worktree第二次必须在`-vv` ledger中
   报告`skiff-compiler-input`、`skiff-compiler-source`、`skiff-compiler`、`skiff-test-runner`及identity test为`Fresh`，
   相关binary/rlib/test hash与mtime不变。
5. 对实际production compiler input/source rlib、compiler/runner/smoke binary运行`strings` no-match，禁止
   `compiler/input[/\\.]+std`、`compiler/input[/\\.]+prelude`、`compiler/source[/\\.]+std|prelude`形式的worktree
   常量；对对应`.d` dep-info运行`rg '# env-dep:CARGO_MANIFEST_DIR='`必须零命中。导入
   `canonicalSkiffSourceTestRegistry`并断言exact为`[{id:'std', root:'std'}]`。
6. 前述cheap/identity/structure全部PASS后，最终从任务临时目录用A-origin shared target和B的absolute
   `scripts/run-skiff-tests.mjs`运行唯一完整gate；要求std 11/11、Host 1/1及exact
   `provider-observed-helper-mutated`。不因cleanup secondary改变primary verdict，但必须保留两者。

## 输出

JSON ledger必须包含commit/tree/lock、capacity、A/B/temp路径、三轮origin、targeted clean crate、artifact枚举、hash/mtime/
dep-info、Fresh crate列表、4次probe的8个golden值、structure/registry结果、std/Host计数、首错以及worktree/temp/PID/port
清理证明。PASS才解除R16；候选、gate script、platform source、Cargo/lock或F17 lifecycle变化会使全部I16证据失效。
