# P5-I16：Platform Source Shared-target Combined Probe

## 角色、输入与证据复用

使用未参与F16A/B/C/F17/F18A–J/F19A/B/F20A/B/F21A/B/C实现、D20/D25/D26/D27/D28审计、P27S/P27R探针、
R21/R21C验收或后续验收的全新Combined Probe owner会话。输入为D27/D28闭合矩阵已汇总、F18A–J至F21A/B/C
全部合流、P27R为PASS且无在途写入的exact clean integration commit/tree；R21C与本任务是P27R间接解锁的两个独立
后继，可以并行，不互为前置。不得编辑、提交、修复、操作stable或运行完整source-suite/Host。权威设计为
`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14及阶段标准2/4/5/6；动态矩阵只
验证既有设计，不增加新的trust、CLI或业务语义。

`ecc53ec`上的首次combined ledger因R18A发现pre-store IO、随后F18J修改authoring/test surface而失效；它只作历史证据，
不得被R16/G16消费。root在冻结新candidate前须校验其hash/candidate并移到repo外归档，使production ledger路径不存在。
全新owner在F18J合流后的候选上重新执行本合同一次；这不计完整Host预算，也不得拆成多轮重试。

`10746a2`上的replacement combined虽PASS，但G16随后暴露test-only full comparator/evidence与inner workspace ownership
blocker；F19A/B修改gate/isolated surface后该v3 ledger及全部窄验收再次失效。下一owner只消费D25与F19A/B证据，在
最终合流candidate上建立v4 combined；不得消费或改写任一历史ledger。

`f82282c`上的v4 combined虽PASS，但第二次G16真实Host code1时仅保留output hash；F20A修改诊断schema且F20B修改公开
`skiff test` caller后，v4 ledger及全部窄验收失效。`7bb6c2a`上的v5 combined随后PASS，但第三次G16仍在readiness前
失败；D27定位Gate因果证据与B-root Router依赖准备缺口，F21A把ledger schema升为
`skiff-platform-source-shared-target-probe-v6`，为full Host失败保留最多3条`diagnostics`、
`diagnosticOmittedCount`及`phase/subject`；F21B增加exact pre-readiness marker
`[skiff-tests] phase startup: isolated-runtime`。F21C增加full-only owned-B Router依赖准备：A→B artifact PASS后、Host前，
从B cwd运行`pnpm --dir router install --frozen-lockfile --offline`，再运行B-local
`router/node_modules/.bin/tsx --version`。因此v5只作历史失效证据，不得被R16、新G16或本任务消费。

integration当前未跟踪的`.p5-i16-combined-ledger.json`正是上述v5历史ledger：candidate为`7bb6c2a`，文件SHA-256为
`244c921ab4efea2bbd3bf20e4f480f7d12af5d535a3b31ab87d722d727a37519`，内部digest为
`937ff2ecba2e1292e5476f7c9d9c1a8c673d94ecb5f1d90b71df5deabbdaae38`。root须在冻结实际运行candidate前核对这些
身份并将其移到repo外历史归档，确认production ledger路径不存在；归档动作不改变其失效状态。P27R的PASS只证明
F21C dependency preparation到B-root readiness/callback的窄边界，并间接解锁fresh R21C与本replacement v6 combined，
不直接解锁full。

开始前只核对并消费开发任务的exact commit/tree/lock、自验收矩阵、D19/F17 lifecycle ledger、D20/D27/D28闭合矩阵、
F21A/B batch combined及P27R PASS持久证据，不重复仍有效的absolute/symlink/missing/cross-root/reserved/omitted/relative/
context-mismatch聚焦测试。唯一merge-only cheap probe与A/B动态矩阵由F16C后续经F19A/F21A/C收敛的
`run-platform-source-shared-target-probe.mjs --mode combined`拥有。候选和环境不变时，R16与后续新G16必须复用本ledger。

## 唯一命令组

Combined Probe owner先核对`git status --short`为空、旧v5已归档且production ledger路径不存在、相关repair ledger、
P26S/P27S/P27R与仍有效的窄结果。在同一候选上先执行一次19项merge-only接线组；不得重复开发者行为矩阵：

```bash
cargo test --locked -p skiff-test-runner --lib \
  canonical_package::tests::combined::p5_f18_compiler_repair_combined -- --exact --ignored --test-threads=1
cargo test --locked -p skiff-compiler --tests --no-run
pnpm --filter @skiff/router type-check
node --check scripts/lib/isolated-test-runtime-instance.mjs
node --check scripts/lib/isolated-test-runtime.mjs
node --check scripts/lib/isolated-test-runtime-workspace.mjs
node --check scripts/lib/supervised-entry-lifecycle.mjs
node --check scripts/lib/managed-pid-metadata.mjs
node --check scripts/skiff-instance.mjs
node --check scripts/lib/platform-source-probe-ownership.mjs
node --check scripts/lib/platform-source-probe-evidence.mjs
node --check scripts/lib/platform-source-probe-diagnostic.mjs
node --check scripts/lib/platform-source-probe-node-dependencies.mjs
node --check scripts/lib/platform-source-probe-support.mjs
node --check scripts/lib/platform-source-probe-contract.mjs
node --check scripts/lib/platform-source-shared-target-probe.mjs
node --check scripts/lib/skiff-source-test-suite.mjs
node --check scripts/lib/package-service-host-negative-probe.mjs
node --check scripts/run-package-service-host-negative-probe.mjs
```

全部PASS后才从非repo cwd执行唯一动态combined命令：

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

命令组任一primary失败立即停止且不重试，不启动reviewer。若candidate/lock不匹配、worktree路径或ledger已存在、容量不足或清理前置不成立，
脚本必须在build前返回`PREFLIGHT BLOCKED`。ledger是候选证据，不提交；G16结束后与临时资源一起清理。

## Combined harness冻结行为

脚本及其command-double test属于F16C/F19A/F21A/C checkpoint；source-suite marker属于F21B checkpoint；isolated owner属于
F19B checkpoint；I16不得修改。`--mode combined`必须：

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
6. 不调用dependency preparation helper、`pnpm ... install`或B-local `router/node_modules/.bin/tsx`，三者production调用
   count必须分别为0；combined ledger不得出现own `nodeDependencies`字段。不调用`run-skiff-tests.mjs`，不启动
   Router/Runtime，不产生std/Host count；ledger必须为`hostAttempt: null`、`sourceSuite: null`、`fullProbeRuns: 0`。
   Host-only `diagnostics`/`diagnosticOmittedCount`不属于combined必需字段，不得为了本任务扩schema。前五项全部PASS后
   原子写combined ledger；cleanup secondary不得改变primary verdict，但必须与primary分别保留。

## 输出

I16证据bundle由两项共同组成，任何一项缺失都不是PASS：

1. gate脚本原子写入的v6 dynamic JSON ledger，包含commit/tree/lock、capacity、A/B/temp路径、三轮origin、targeted
   clean crate、artifact枚举、hash/mtime/dep-info、Fresh crate列表、4次probe的8个golden值、structure/registry、首错及
   worktree/temp/PID/port/registry/ownership清理证明，明确`fullProbeRuns: 0`、`hostAttempt: null`、`sourceSuite: null`且own
   `nodeDependencies`字段不存在；不得要求combined ledger含Host diagnostics，不得事后修改或扩schema。
2. Combined Probe owner在最终回报中给出immutable `p5-i16-command-group-v2` report：同一candidate/tree/lock、clean
   before/after、上述19条前置命令的exact argv/exit/result，以及combined production dependency helper/install/tsx调用
   count分别为0。merge-only Rust test必须是`1 passed / 0 failed / 0 ignored`，不能以exit 0、0-run或ignored冒充；Node
   checks/type-check/no-run也必须逐项列出。

PASS只建立该candidate的replacement v6 combined证据；它与fresh R21C是P27R后的并列后继，任一缺失都不能解除后续
新G16。候选、F18A–J/F19A/B/F20A/B/F21A/B/C任一相关production/test surface、Gate contract/script/schema、dependency
helper、source-suite marker、Router package manifest/lock或依赖物化方式、platform source、Cargo/lock、A/B/shared target隔离
环境变化会使全部I16证据失效。

本任务保持`fullProbeRuns: 0`，不消耗新的完整探针周期。交给后续新G16合同时，历史累计仍为3次full-mode调用、2次真实
Host attempt、0次完整positive Host；D27/F21/P27R/R21C/replacement v6 combined闭合后建立的新周期从0计数，下一次
full是该新周期第1次且默认最多2次。具体full命令、候选冻结与预算只能由新的G16合同定义，本合同不得直接运行或
解锁full。
