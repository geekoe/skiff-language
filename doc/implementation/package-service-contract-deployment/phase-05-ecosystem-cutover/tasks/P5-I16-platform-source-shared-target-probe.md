# P5-I16：Platform Source Shared-target Combined Probe

## 角色、输入与证据复用

使用未参与F16A/B/C/F17实现、D20审计/修复或R16验收的全新Combined Probe owner会话。输入为D20闭合矩阵已汇总、
其批量repair wave全部合流、无在途写入的exact clean integration commit/tree；不得编辑、提交、修复、操作stable或运行
完整source-suite/Host。权威设计为`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14
及阶段标准2/4/5/6；动态矩阵只验证既有设计，不增加新的trust、CLI或业务语义。

开始前只核对并消费开发任务的exact commit/tree/lock、自验收矩阵、D19/F17 lifecycle ledger及D20闭合矩阵，不重复
仍有效的absolute/symlink/missing/cross-root/reserved/omitted/relative/context-mismatch聚焦测试。唯一merge-only cheap
probe与A/B动态矩阵由F16C提交的`run-platform-source-shared-target-probe.mjs --mode combined`拥有。候选和环境不变时，
R16与G16必须复用本ledger。

## 唯一命令

Combined Probe owner先核对`git status --short`为空及输入ledger有效，然后只执行：

```bash
P5_I16_ROOT=/Users/geek/workspace/skiff-phase-05-integration
P5_I16_COMMIT="$(git -C "$P5_I16_ROOT" rev-parse HEAD)"
P5_I16_TREE="$(git -C "$P5_I16_ROOT" rev-parse HEAD^{tree})"
cd /tmp
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-platform-source-shared-target-probe.mjs \
  --mode combined \
  --integration-root "$P5_I16_ROOT" \
  --candidate "$P5_I16_COMMIT" \
  --expected-tree "$P5_I16_TREE" \
  --expected-lock-blob f3ce5457138c58aec4c84abda431afa96013e3fd \
  --expected-prelude-identity skiff-prelude-v1:sha256:aae18f07de6746b8cc769ca3bd9db6b65b6c292fc75016549b58cd253b3f3f0d \
  --expected-std-package-build-id skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4 \
  --a-worktree /Users/geek/workspace/skiff-p5-i16-a \
  --b-worktree /Users/geek/workspace/skiff-p5-i16-b \
  --ledger /Users/geek/workspace/skiff-phase-05-integration/.p5-i16-combined-ledger.json \
  --json
```

任何primary失败立即停止且不重试。若candidate/lock不匹配、worktree路径或ledger已存在、容量不足或清理前置不成立，
脚本必须在build前返回`PREFLIGHT BLOCKED`。ledger是候选证据，不提交；G16结束后与临时资源一起清理。

## Combined harness冻结行为

脚本及其command-double test属于F16C checkpoint；I16不得修改。`--mode combined`必须：

1. 复验integration exact commit/tree/lock/clean、A/B路径不存在、无同名worktree。容量门槛为任务target预计占用：
   existing shared Cargo target allocated bytes加2 GiB；若无可测shared target则要求至少8 GiB free。创建
   `/Users/geek/workspace/.skiff-p5-i16.XXXXXX`任务目录、其中唯一shared target，并用`try/finally`/signal handlers只
   清理任务目录和A/B detached worktree；保留原子写入的combined ledger，记录清理后ABSENT、端口与残留进程。
2. 在exact合流tree只运行未被开发ledger执行的merge-only
   `node --test scripts/tests/platform-source-transport-combined.test.mjs`，以及一次共同编译
   `cargo check --locked -p skiff-compiler -p skiff-test-runner --bins`。fixture检查compiler authoring、runner、smoke
   bootstrap、source-suite、`skiff test`、runtime-live和encrypted-storage argv共享同一absolute root，并直接执行一个
   omitted-root fail-closed路径。
3. 建A/B detached worktree；共享target依次建立A-origin、B-origin、最终A-origin三轮。targeted clean crate固定为
   `skiff-test-runner`、`skiff-compiler`、`skiff-compiler-input`、`skiff-compiler-source`；每轮显式使用对应absolute
   `--manifest-path`、同一`CARGO_TARGET_DIR`、`--locked`及runner/smoke两个binary selector。
4. A-origin下依次以A、B runtime root运行ignored `platform_source_identity_probe`；B-origin下依次以B、A root运行。
   共4次probe、8个带标签值，全部exact等于两个golden。跨worktree第二次在`-vv` ledger中必须报告相关compiler/
   runner/identity target为`Fresh`，相关binary/rlib/test hash与mtime不变。
5. 对实际production compiler input/source rlib、compiler/runner/smoke binary运行`strings` no-match，禁止编入
   `compiler/input[/\\.]+(std|prelude)`、`compiler/source[/\\.]+(std|prelude)`形式的worktree常量；对应`.d` dep-info的
   `# env-dep:CARGO_MANIFEST_DIR=`必须零命中。导入`canonicalSkiffSourceTestRegistry`并断言exact为
   `[{id:'std', root:'std'}]`。
6. 不调用`run-skiff-tests.mjs`，不启动Router/Runtime，不产生std/Host count。前五项全部PASS后原子写combined ledger；
   cleanup secondary不得改变primary verdict，但必须与primary分别保留。

## 输出

JSON ledger必须包含commit/tree/lock、capacity、A/B/temp路径、三轮origin、targeted clean crate、artifact枚举、hash/mtime/
dep-info、Fresh crate列表、4次probe的8个golden值、structure/registry结果、首错及worktree/temp/PID/port清理证明；明确
`fullProbeRuns: 0`。PASS才解除R16；候选、gate script、platform source、Cargo/lock、D20 repair或F17 lifecycle变化会使
全部I16证据失效。
