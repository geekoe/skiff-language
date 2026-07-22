# P5-F21C：Gate Owned Worktree Router Dependencies

## 输入、设计引用与 DAG

唯一权威语义来自 `doc/architecture/package-service-contract-deployment.md` §11、§14，以及
`doc/architecture/test-runner-runtime-isolation.md` 的 **Ownership Boundary**、**Runner Contract** 与
**Lifecycle And Recovery**。本任务只补齐 test-only Gate 对 detached checkout 的资源准备，不新增或改写
package/service、Router、Runtime、artifact identity 或测试发现语义。

输入为 D27A/B closure audit、P27S 持久证据
`/Users/geek/workspace/skiff-phase-05-evidence/p5-p27s-7bb6c2a-shared-target-startup.json`，以及已合流
F21A/B 的 checkpoint `dbfb98ac0a10d3959d803a8a92de1c04bba66fce` / tree
`68a824aa233ade4cd455c7be999f5fa1219b46cc` / `Cargo.lock` blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。P27S 已证明 A/B Cargo reuse 为四 crate `Fresh`、artifact
diff 为 `2 allowed / 0 disallowed`，随后 B checkout 的 Router 在 readiness 前因
`router/node_modules` 缺失而以 `sh: tsx: command not found` 退出；Runtime 未启动，callback 为 0。

执行 DAG：

```text
D27A/B + P27S + F21A/B/R21 checkpoint
  -> F21C owned-B Router dependency preparation
  -> fresh non-full shared-target+B startup reacceptance
  -> replacement v6 combined evidence
```

F21C 只解锁后续非 full 窄复验，不给 G16、F04 或阶段 verdict，也不允许在当前旧周期运行第四次完整探针。

## 写入 owner 与非目标

使用因明确合同前置而暂停的同一 F21C 开发 Agent，从包含本合同的精确 integration checkpoint 创建
`/Users/geek/workspace` 下的独立 worktree 和 branch；交付一个 clean commit 后结束，不 merge/push/stable。

exclusive write set：

- `scripts/lib/platform-source-shared-target-probe.mjs`；
- 可新增一个聚焦的 `scripts/lib/platform-source-probe-node-dependencies.mjs` child owner；
- `scripts/tests/platform-source-shared-target-probe.test.mjs`。

不得修改 `router/**`、`runtime/**`、compiler/test-runner production、isolated runtime、source suite、Gate
diagnostic/evidence/contract、workspace/package manifests、任一 lockfile 或 stable instance。不得从 integration、A、
用户 home 或其它 checkout 链接/复制 `node_modules`，不得把缺失依赖降级为 skip/fallback，也不得放宽 A→B
Cargo `Fresh` 与 artifact comparator。

若实现必须越过上述写集、改变 package manager/公共架构职责或联网获取未锁定输入，停止并报告设计决策。

## 完成态

1. 仅在 full 模式、owned B worktree 已建立且 A→B Cargo/artifact evidence 通过之后、真实 Host attempt 之前，
   为 B 的 `router/` 物化其 checked-in `pnpm-lock.yaml` 精确锁定的依赖。combined 模式不得运行该步骤。
2. 准备命令固定为 B-root-relative 的 Router package operation，必须同时使用 `--frozen-lockfile` 与 `--offline`；
   不更新 lockfile、不使用网络、不读取另一个 checkout 的 `node_modules`。本机 pnpm content-addressed store 只作为
   锁定输入的缓存；缓存不完整时在 Host 前 fail closed。
3. 准备后显式验证 B checkout 自己的 Router 可解析/执行 `tsx`；只检查文件或外部 PATH 不足以通过。该验证与
   dependency command 的 code/signal 必须进入 bounded Gate outcome，失败时保留 F21A 的 startup causal diagnostic，
   且 `fullProbeRuns` / Host attempt 仍为 0。
4. dependency preparation 恰好一次、只针对 B，并先于 source suite/Host child；成功后仍沿用 B source、owned
   isolated workspace 和现有 lifecycle cleanup。失败、signal 或中断不能绕过 worktree/task/process/port cleanup。
5. command-double 必须证明 combined 不准备依赖；full 的准备/验证/Host 顺序；locked+offline argv；B-only cwd/root；
   dependency 或 `tsx` 验证失败阻断 Host；无 integration/A `node_modules` 借用；primary failure 仍先于 cleanup error。
6. 不复制 dependency/preflight 规则到 evaluator、validator 或测试 helper。若现有主编排文件因新增职责继续膨胀，
   使用上述单一 child owner，并执行 extra-review 检查重复与边界。

## 聚焦验证与交付

禁止运行真实 Cargo build、dependency install、I16、H18、full、Host 或 stable 操作。开发自验收只使用 command-double：

```bash
node --test --test-name-pattern 'dependencies|combined and full modes|primary failure' \
  scripts/tests/platform-source-shared-target-probe.test.mjs
node --check scripts/lib/platform-source-shared-target-probe.mjs
node --check scripts/lib/platform-source-probe-node-dependencies.mjs
git diff --check
```

报告 matched test count、commit/tree/lock、精确 argv/顺序、failure/cleanup 矩阵、反搜外部 `node_modules`、
extra-review 与 clean status。后续 fresh reacceptance 才允许运行一次 P27S 同形的非 full B-startup probe。

## 证据失效边界

F21C 会使当前 shared-target Gate source、旧 v5 combined ledger、R21 对 F21A/B-only 候选的整体 Gate 结论失效；
F21A/B 的聚焦 parser/marker 单测与 P27S 的根因诊断事实仍可保留。F21C 合流后必须在同一精确候选上重新执行
fresh 窄验收与 v6 combined。此后若 Gate resource preparation/Host sequencing、Router package manifest/lock、isolated
runtime launch command、Cargo/lock 或候选 tree 任一变化，F21C 动态证据及其下游 combined/full 证据全部失效。
