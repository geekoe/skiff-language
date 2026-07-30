# P5-G16E：V6 Real Host Gate

## 角色、设计追溯与预算

使用未参与旧 G16/G16D、D27/D30/D31、P27S/P27R、F21/F22、R21/R21C/R22、I16 或相关实现/验收的
全新 Gate Agent。唯一权威设计为 `doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、
§9–§14；阶段标准为 phase plan §2 的2/4/5/6；隔离边界为
`doc/architecture/test-runner-runtime-isolation.md` 的 **Ownership Boundary**、**Runner Contract**、
**Lifecycle And Recovery**。只读、不修复、不提交、不操作 stable；PASS只解锁同一候选的全新 F04 receive reviewer，
不给 F04 或阶段 verdict。

G16D 是 D27/F21/F22 收敛周期第1次、历史 full #4、历史真实 Host attempt #3，且永久保持FAIL。G16E 是该周期
第2次、历史 full #5，也是本周期预算上限；若到达真实 child，则为历史 Host attempt #4；若PASS，则为首个完整
positive Host。本合同只授权下面production full命令一次，任一失败均不得重试。一旦执行该Node命令，即使脚本内部
preflight blocked也已消费full #5；只有命令前的外部前置不满足可作为`PREFLIGHT BLOCKED`且不消费调用。

G16E失败后周期预算耗尽；任何下一次full前必须重新审计全部剩余路径、说明第三次周期调用必要性，并建立新合同和
全新Gate owner。

## 硬前置与冻结候选

执行前逐项满足，否则在命令前停止：

1. final candidate包含F22A implementation `d7ac987d54469238c413f3ed84c962a0bc2984b2`、F22B implementation、P5-R22B、更新后的active
   P5-I16与本合同；`Cargo.lock` blob为`f3ce5457138c58aec4c84abda431afa96013e3fd`。G16D合同/result blobs保持
   不变，不得事后改判。
2. 原R22 reviewer按P5-R22B只复验其global-uniqueness blocker并在该exact candidate上PASS、blocking findings为0；同候选replacement I16的
   `p5-i16-command-group-v3` 20项与唯一dynamic combined均PASS。
3. combined ledger为v6/combined PASS且digest复算一致；candidate/tree/lock/goldens相同，
   `fullProbeRuns:0`、`hostAttempt:null`、`sourceSuite:null`，无own `nodeDependencies`。记录文件SHA与内部digest，
   full前后必须bit-identical。
4. integration tracked clean，唯一允许的untracked是当前`.p5-i16-combined-ledger.json`；A/B路径、Git登记、task root、
   owner、进程、监听端口和lease均不存在，无在途写入。
5. 容量满足`available >= existing shared target allocated + 2 GiB`；无法测量existing target时至少8 GiB。
6. P27R/R21C dependency/startup证据继续有效：F22A未改变dependency helper、full orchestration、source runner、Router
   manifest/lock、fixture/discovery/formatter、isolated startup/lifecycle。不得为G16E重跑这些窄探针。
7. `stableOperations:0`；不访问stable或固定`4000`–`4003`。

## 唯一命令

仅从非repo cwd执行一次；不得直接调用source suite、重跑combined/narrow/dependency install或自行启动Host：

```bash
P5_G16E_ROOT=/Users/geek/workspace/skiff-phase-05-integration
P5_G16E_COMMIT="$(git -C "$P5_G16E_ROOT" rev-parse HEAD)"
P5_G16E_TREE="$(git -C "$P5_G16E_ROOT" rev-parse HEAD^{tree})"
cd /tmp
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-platform-source-shared-target-probe.mjs \
  --mode full \
  --integration-root "$P5_G16E_ROOT" \
  --candidate "$P5_G16E_COMMIT" \
  --expected-tree "$P5_G16E_TREE" \
  --expected-lock-blob f3ce5457138c58aec4c84abda431afa96013e3fd \
  --expected-prelude-identity skiff-prelude-v1:sha256:aae18f07de6746b8cc769ca3bd9db6b65b6c292fc75016549b58cd253b3f3f0d \
  --expected-std-package-build-id skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4 \
  --combined-ledger /Users/geek/workspace/skiff-phase-05-integration/.p5-i16-combined-ledger.json \
  --a-worktree /Users/geek/workspace/skiff-p5-g16e-a \
  --b-worktree /Users/geek/workspace/skiff-p5-g16e-b \
  --json
```

## 冻结顺序与 PASS 证据

production Gate保持单向顺序：A build → B build → artifact PASS → owned-B locked/offline dependency preparation → B-local
tsx proof → fixture guard → 唯一source-suite/Host child → cleanup。不得并行、跳过或借用其它checkout结果。

PASS必须同时记录：

- 顶层v6/full，candidate/tree/lock/goldens一致；`status`和`primary.status`为PASS，`primary.error`与`firstError`为null，
  `fullProbeRuns:1`，ledger digest复算一致；combined和integration前后不变。
- artifact中4个targeted crate全Fresh；exact两个top-level `.d`只发生A→B root materialization，
  `changed=2/allowed=2/disallowed=0`；其它binary/rlib/hashed dep-info不变。
- `nodeDependencies.status:PASS`且root/cwd为owned B；install恰好一次
  `pnpm --dir router install --frozen-lockfile --offline`，随后B-local
  `router/node_modules/.bin/tsx --version`恰好一次；均code0、signal/spawnError null并有字节数和SHA。
- Host command恰为`node <B>/scripts/run-skiff-tests.mjs`且只启动一次；code0、signal/error null，process/port evidence为true；
  result lines精确且有序为`11 passed; 0 failed`后`1 passed; 0 failed`。
- `expectedPassLine`只能是协议描述
  `PASS <runtime-module-path>::provider observes helper mutation`，不得硬编码module。actual目标行必须符合
  `^PASS [A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*::[\x20-\x7e]+$`、不超过512 UTF-8 bytes、test name精确，且只在
  stdout两条result line之间出现一次。当前candidate静态推导的观察值是
  `PASS main.__test::provider observes helper mutation`；它是本次事实，不得重新成为JS oracle。
- `observedPassLine`保存actual完整行，`exactPassLineCount:1`。正常12条PASS evidence中，唯一目标保留actual；其余11条
  只能是`PASS <unexpected sha256:<64-lowerhex>>`，不得保存原始无界文本。
- `sourceSuite.std=11/11`、`host=1/1`、`finalValue=provider-observed-helper-mutated`；`finalValueEvidence`同时含actual
  pass line、B fixture path与exact assertion
  `assert root.main.run() == "provider-observed-helper-mutated"`。
- phase/subject为`host-runner`/`package-service-host`；stdout/stderr/output byte/hash完整。diagnostics最多3条、每条
  ≤512 bytes、总excerpt≤1536，并含kind/stream/sanitizedExcerpt/originalLineSha256/truncated及omitted count。
- cleanup：A/B path、Git admin/registry、task/shared target、inner workspace、B dependencies、PID/PGID、端口与lease全部
  absent；nonce/marker/dev+ino验证，foreign preserved，errors为空，stable operations为0。

## FAIL、清理与失效边界

- artifact或更早失败：`fullProbeRuns:0`、Host/sourceSuite null；dependency失败再保留bounded install/tsx outcome。
- fixture失败不得启动Host；Host开始后失败为`fullProbeRuns:1`、`hostAttempt.status:FAIL`、`sourceSuite:null`，保留v6
  diagnostics/hashes/issue。wrong/missing/duplicate/malformed/oversized PASS、非法module、std段或stderr目标均fail closed。
- cleanup secondary不得覆盖primary；所有可安全清理资源仍须absent。任一FAIL后Agent结束，不修复、不重跑、不改G16D。

candidate/tree/lock或本合同/active I16、fixture/test name/assertion、Rust discovery/formatter、Host evidence/re-export、
orchestration/schema/diagnostic、source suite、artifact comparator、dependency/Router lock、ownership/cleanup、golden identity或
combined ledger任一变化，都会使R22B/I16/G16E共同失效。

提交本合同时只做静态lint：diff-check；唯一命令块内`--mode full`和`--combined-ledger`各1，combined/独立ledger/direct
Host/stable/fixed ports为0，A/B路径不同；旧G16D合同/result无diff；production/test路径旧错误literal零命中，Host identity
parser只有新child owner。不得在合同冻结阶段运行任何probe。
