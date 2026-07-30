# P5-G16D：V6 Real Host Gate

## 角色、设计追溯与边界

使用未参与旧G16、D27、P27S、P27R、F21A/B/C、R21、R21C、replacement I16或相关实现/审计/验收的
全新Gate Agent。唯一权威设计为
`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14；阶段完成标准为
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/phase-plan.md` §2的2/4/5/6；
隔离执行边界为`doc/architecture/test-runner-runtime-isolation.md`的 **Ownership Boundary**、
**Runner Contract**、**Lifecycle And Recovery**。

本任务是冻结候选上的高风险完整Gate，只读且唯一owner。Gate Agent不得编辑、提交、修复、安装候选之外的共享依赖、
操作常驻开发环境或改变公共契约；`stableOperations`必须为0。PASS只解锁一个全新的F04 receive reviewer，不给F04或阶段
最终verdict。任一失败后本Agent结束，不修复、不重试。

## 周期与预算

历史事实固定为3次full-mode调用、2次真实Host attempt、0次完整positive Host。D27闭合审计、F21A/B/C修复、P27R、
R21C与replacement I16 v6共同建立新收敛周期；G16D是新周期第1次、历史第4次full-mode调用。新周期原则上最多2次，
但本合同只授权下面的唯一调用一次；preflight、dependency、artifact或Host任一阶段失败都不得在本合同内重试。

若本次到达真实Host child，它是历史第3次Host attempt；若最终PASS，它是历史第1次完整positive Host。任何第二次调用都
必须由root先回到剩余路径审计、归类失败与证据失效面，再建立新的Gate合同和全新owner；本文件不能授权该调用。

## 硬前置与冻结候选

执行前必须逐项满足，任一不满足均为`PREFLIGHT BLOCKED`并停止：

1. P27R持久证据
   `/Users/geek/workspace/skiff-phase-05-evidence/p5-p27r-35f93c9-owned-b-router-startup.json`为PASS，文件SHA-256精确为
   `5259efa942af11840307efc87865cb0895329d7af9e931eb770ade9e56720f4a`，内部evidence digest精确为
   `959f5527fbe213bd9652ebb279ef2d4a5096845e4f9f1a7b4da8312fb3d89522`；从该证据锚点到当前候选不得有命中
   P27R失效边界的变更。对应合同为
   `doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-P27R-owned-b-router-dependency-startup-reacceptance.md`。
2. 全新R21C reviewer已按
   `doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-R21C-gate-router-dependency-acceptance.md`
   给出PASS且blocking findings为0；其验收表面到当前候选没有失效变更。
3. 全新replacement I16 owner已按
   `doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-I16-platform-source-shared-target-probe.md`
   在当前exact candidate上完成v6 dynamic combined与`p5-i16-command-group-v2`，两项均PASS。full输入、I16 ledger和
   integration HEAD的commit/tree/`Cargo.lock` blob/prelude identity/std package build identity必须逐字一致。
4. integration无在途写入、HEAD已冻结；tracked status为空，唯一允许的untracked项是
   `/Users/geek/workspace/skiff-phase-05-integration/.p5-i16-combined-ledger.json`。A/B路径不存在且未登记为Git worktree，
   没有同名task root、残留owner、进程、监听端口或租约。
5. v6 combined ledger的`schemaVersion`为`skiff-platform-source-shared-target-probe-v6`、`mode`为`combined`、
   `status`与`primary.status`均为`PASS`、`firstError`为null、canonical digest复算一致；`fullProbeRuns`为0、
   `hostAttempt`与`sourceSuite`均为null，且不存在own `nodeDependencies`字段。其candidate/tree/lock/goldens、三轮origin、
   4次identity probe、4 Fresh矩阵、artifact/structure/registry与cleanup/ownership证据完整。Gate owner在执行前后分别记录
   ledger文件SHA-256与内部digest，二者必须不变。
6. 容量预检满足`available bytes >= existing Cargo target allocated bytes + 2 GiB`；无法测得existing target时，
   `available bytes >= 8 GiB`。容量、路径、candidate与ledger检查必须在任何build前完成。

D27、P27S、F21与R21历史事实只用于解释闭合链，见
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-D27-shared-target-startup-closure-audit-result.md`、
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-P27S-shared-target-b-root-startup-probe-result.md`、
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F21A-gate-causal-evidence-result.md`、
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F21B-source-startup-marker-result.md`、
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F21C-gate-owned-worktree-router-dependencies.md`和
`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-R21-gate-causal-evidence-acceptance-result.md`；
不得把它们替代当前exact I16或本次full证据。

## 唯一命令

Gate Agent只从非repo cwd执行以下命令一次。candidate与tree动态读取冻结HEAD；不得把本合同创建时的commit硬编码为
执行candidate。不得直接调用source-suite child，不得重跑combined动态矩阵、merge-only接线组、窄验收或dependency
install；这些步骤只能由下面的production Gate owner按冻结顺序触发。

```bash
P5_G16D_ROOT=/Users/geek/workspace/skiff-phase-05-integration
P5_G16D_COMMIT="$(git -C "$P5_G16D_ROOT" rev-parse HEAD)"
P5_G16D_TREE="$(git -C "$P5_G16D_ROOT" rev-parse HEAD^{tree})"
cd /tmp
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-platform-source-shared-target-probe.mjs \
  --mode full \
  --integration-root "$P5_G16D_ROOT" \
  --candidate "$P5_G16D_COMMIT" \
  --expected-tree "$P5_G16D_TREE" \
  --expected-lock-blob f3ce5457138c58aec4c84abda431afa96013e3fd \
  --expected-prelude-identity skiff-prelude-v1:sha256:aae18f07de6746b8cc769ca3bd9db6b65b6c292fc75016549b58cd253b3f3f0d \
  --expected-std-package-build-id skiff-package-build-v4:sha256:3bbab8df662b54826dfbd3112c960446dd8b429f3018e7b0a5f27ffc314b7fa4 \
  --combined-ledger /Users/geek/workspace/skiff-phase-05-integration/.p5-i16-combined-ledger.json \
  --a-worktree /Users/geek/workspace/skiff-p5-g16d-a \
  --b-worktree /Users/geek/workspace/skiff-p5-g16d-b \
  --json
```

## 冻结执行顺序与失败边界

production Gate必须保持以下单向顺序，不得并行、跳过或从其它checkout借用结果：

1. 在owned shared Cargo target以A-root build runner与package-service smoke fixture；随后从B-root消费同一target。
2. B build必须报告`skiff-test-runner`、`skiff-compiler`、`skiff-compiler-input`、`skiff-compiler-source`四crate全
   `Fresh`。artifact comparator只允许两个exact top-level dep-info文件中的A→B绝对root materialization；binary、rlib、
   hashed dep-info和其它artifact不变，`disallowedCount`必须为0。
3. artifact PASS后才调用F21C唯一dependency owner，从owned B cwd恰好一次执行
   `pnpm --dir router install --frozen-lockfile --offline`，再从同一B cwd恰好一次执行B-local
   `router/node_modules/.bin/tsx --version`。不得读取、链接或复制integration、A、home或foreign `node_modules`。
4. dependency两步PASS后才读取并验证B fixture；B source的canonical source-suite child恰好启动一次，其中std与Host
   entry各执行一次。命令证据必须为executable `node`与唯一参数`<B>/scripts/run-skiff-tests.mjs`。Gate Agent不得在
   production owner外直接执行该child。

dependency preparation在install spawn/nonzero/signal或tsx验证失败时立即fail closed：`fullProbeRuns: 0`、
`hostAttempt: null`、`sourceSuite: null`。仅在Host child开始前把`fullProbeRuns`置为1并创建`hostAttempt`；Host开始后的
任一失败仍不得重试。

## PASS证据

PASS必须同时满足并由Gate owner在最终回报逐项列出：

- 顶层ledger为v6/full，candidate/tree/lock/goldens与冻结HEAD和被消费combined ledger完全一致；`status: PASS`、
  `primary.status: PASS`、`primary.error: null`、`firstError: null`，`fullProbeRuns: 1`；`ledgerDigest`从返回JSON精确复算一致。
- `combinedLedger`为preflight消费的原始v6 PASS对象；combined文件的SHA-256、内部digest和内容前后bit-identical，
  integration candidate/tree/tracked status也前后不变。
- `artifactEvidence`为`full-root-materialization-v1`且PASS：上述4 crate全Fresh、exact两个top-level `.d` root
  materialization、`disallowedCount: 0`，artifact枚举、hash、mtime与diff证据完整。
- `nodeDependencies.status: PASS`且root为B；install与tsx的cwd均为B，命令/argv符合冻结顺序，二者均
  `code: 0`、`signal: null`、`spawnError: null`，并分别记录stdout/stderr byte count与SHA-256。install count和tsx count
  均恰好为1。
- `hostAttempt.status: PASS`、`code: 0`、`signal: null`、`error: null`，stdout/stderr byte count与SHA-256及
  `outputSha256`完整；命令为exact `node <B>/scripts/run-skiff-tests.mjs`。
  结果行恰好两条且顺序为`test result: ok. 11 passed; 0 failed`、`test result: ok. 1 passed; 0 failed`；
  `PASS main.test.skiff::provider observes helper mutation`精确出现一次。
- `sourceSuite.std`为`11/11`，`sourceSuite.host`为`1/1`，`sourceSuite.finalValue`精确为
  `provider-observed-helper-mutated`。`finalValueEvidence`必须同时保留B fixture assertion path、exact assertion
  `assert root.main.run() == "provider-observed-helper-mutated"`与上述唯一PASS行，证明provider可观察结果而非中间hook。
- cleanup与ownership全部闭合：A/B path和Git admin/registry、owned task root/shared target、inner workspace、owned
  PID/PGID、监听端口与租约均ABSENT；nonce、marker与dev+ino claim在删除前验证，B-local dependencies随B一起删除，
  foreign state preserved，cleanup errors为空。`stableOperations: 0`。

## FAIL证据、清理与交接

Host child失败时，v6 evidence必须保留`phase`、`subject`、stdout/stderr byte count与SHA-256，以及最多3条
`diagnostics`；每条包含`kind`、`stream`、脱敏且UTF-8有界的`sanitizedExcerpt`、`originalLineSha256`与`truncated`，
另存`diagnosticOmittedCount`。不得包含旧`firstDiagnostic`字段，也不得把stderr优先等确定性排序描述为两个pipe的真实跨流时序。
dependency失败只保留其有界install/tsx outcome、byte count、hash与spawn error，不伪造Host diagnostics。

任何primary failure都先于cleanup secondary；两类错误必须分别保留，cleanup不得覆盖原始失败。无论失败发生在build、
artifact、dependency或Host，均按nonce/marker/dev+ino owner执行同一安全清理并核对foreign preserved；所有仍可清理的临时
资源必须ABSENT。Gate Agent随后返回exact candidate/tree/lock、历史与新周期计数、primary、cleanup、combined ledger
前后身份和首个未闭合跳点，结束会话且不修改候选。

失败统一退回root做剩余路径审计和DAG更新。本合同不允许同Agent修复或重跑；如root判定新周期第2次full确有必要，必须
在相关修复、cheap combined和失效窄验收全部闭合后，另写合同并交给另一全新Gate Agent。
