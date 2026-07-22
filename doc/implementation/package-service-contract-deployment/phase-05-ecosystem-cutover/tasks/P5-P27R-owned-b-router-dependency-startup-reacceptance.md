# P5-P27R：Owned B Router Dependency Startup Reacceptance

## 输入与边界

唯一权威语义来自 `doc/architecture/test-runner-runtime-isolation.md` 的 **Ownership Boundary**、
**Runner Contract**、**Lifecycle And Recovery**，以及
`doc/architecture/package-service-contract-deployment.md` §11、§14。执行合同来自 P5-F21C；诊断输入为
P5-P27S result 与持久 evidence。F21C 已合流到 checkpoint
`f14374ee5f26f0394eef56fc7881a896f2879cc2` / tree
`9f14b5136656dff707e244d10cb997e370e4cc5f` / `Cargo.lock` blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

使用未参与 D27、P27S、F21A/B/C、R21 或其它探针的全新只读 Agent，从包含本合同的 exact candidate 执行。
不得修改/提交 production 或文档，不给 G16、F04 或阶段 verdict，不运行 Cargo A/B build、I16、H18、full、Host
source suite、stable instance，也不访问固定 `4000`–`4003` 端口。

P27S 对 A/B shared target 的 `4 Fresh / 2 allowed / 0 disallowed` 事实继续有效，因为 F21C 没有修改 Cargo、
Rust source、manifest/lock 或 artifact comparator。本任务只验证被 F21C 改变的 dependency preparation → Router startup
边界，不重复已关闭的共享 Cargo 构建。

## 唯一动态探针

先做只读 preflight：candidate/tree/lock/clean、可用空间、`node`/`pnpm`/`cargo`、owned 路径未占用和
`46000`–`46999` lease 可用。READY 后实际动态调用最多一次：

1. 创建 nonce/marker/dev+ino 保护的 owned task root 与一个 exact-candidate detached B worktree；记录 Git admin owner。
2. 从 production `scripts/lib/platform-source-probe-node-dependencies.mjs` 调用同一 dependency preparation owner；不得在
   probe helper 复制 argv 或改为手写 install。断言 cwd/root 是 B，恰好一次 locked+offline install、一次 B-local
   `router/node_modules/.bin/tsx --version`，两者 code 0/signal null；lockfile 与 tracked tree不变。
3. 从 owned task cwd import B candidate 的 production `runInIsolatedTestRuntime`，使用 owned temp/ports 与 empty callback；
   断言 generation-0 bootstrap、supervisor、Router/Runtime readiness 成功，callback 恰好 1，未执行 source runner/Host。
4. 正常或失败都沿 production owner teardown。删除前复核 marker/nonce/dev+ino；结束后 B path、Git admin、inner
   workspace、task root、dependency materialization、PID/PGID、leases/listeners 全部 absent，foreign state preserved。

失败时在 inner workspace cleanup 前有界保留 Router/Runtime component logs：每文件 bytes + SHA-256；不超过 20KB 可保留
脱敏全文，否则只保留脱敏 head/tail。不得保存 env、secret、HTTP body 或无界输出。若 dependency preparation 失败，分类为
F21C environment/evidence blocker；若准备通过但 component 在 callback 前失败，按日志给出新的 production owner；若 ready 与
callback 成功，则只关闭 F21C/P27S startup blocker。

## 交付与失效

持久 JSON evidence 写到 `/Users/geek/workspace/skiff-phase-05-evidence/`，记录 candidate/tree/lock、生产 helper identity、
依赖 outcome、startup 跳点、callback count、bounded logs、cleanup 与 canonical digest；返回文件 SHA-256 和独立 digest
复算。不得把 evidence 写入 Git worktree。

candidate、F21C helper、Router package manifest/lock、isolated runtime startup/lifecycle 或 cleanup owner 任一变化都会使本
动态证据失效。PASS 后只解锁 fresh R21C 与 replacement v6 combined；不直接解锁 full。
