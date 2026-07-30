# P5-R21C：Gate Router Dependency Acceptance

使用未参与 D27、P27S/P27R、F21A/B/C、R21 或其它验收的全新只读 Agent。唯一权威语义引用
`doc/architecture/test-runner-runtime-isolation.md` 的 **Ownership Boundary**、**Runner Contract**、
**Lifecycle And Recovery**，以及 `doc/architecture/package-service-contract-deployment.md` §11、§14；实现合同为
P5-F21C。输入为包含本合同的 exact clean integration candidate、F21C dev/integration commit、root combined 证据与
P27R evidence。

只给 R21C verdict，不给 G16、F04 或阶段 verdict；不修改/提交，不运行 dependency install、Cargo、I16、H18、full、
Host、真实 isolated runtime 或 stable。

验收矩阵：

- full only：dependency preparation 位于 A→B artifact PASS 之后、Host attempt/source suite 之前；combined 为 0；
- B ownership：cwd/root 与 executable 都来自 owned B；不链接/复制 integration、A、home 或 foreign `node_modules`；
- reproducibility：Router install 使用 checked-in lock、`--frozen-lockfile`、`--offline`，不更新 lock、不网络 fallback；
- executable proof：直接执行 B-local `router/node_modules/.bin/tsx --version`，仅 PATH/file existence 不算通过；
- fail closed：install spawn/nonzero/signal 与 tsx failure 都在 Host 前停止，`fullProbeRuns=0`，保留 bounded outcome，
  primary failure 不被 cleanup 覆盖；
- lifecycle：所有分支仍进入原 owned worktree/task/process/port cleanup，不能删除 foreign path；
- single owner：dependency argv/outcome/validation 只在聚焦 child owner，Gate/evaluator/validator/test helper 不复制规则；
- F21C 未触碰 Router/Runtime、公共 package/service contract、artifact comparator、manifest/lock 或业务语义。

聚焦命令：

```bash
node --test --test-name-pattern 'dependencies|combined and full modes|primary failure' \
  scripts/tests/platform-source-shared-target-probe.test.mjs
node --check scripts/lib/platform-source-shared-target-probe.mjs
node --check scripts/lib/platform-source-probe-node-dependencies.mjs
git diff --check
```

要求 matched > 0，回报 exact commit/tree/lock、逐条 PASS/FAIL、命令/计数、P27R 动态证据是否与代码 owner 对应、
blocking findings 与 extra-review。任何实现/测试/合同不一致为 FAIL；公共设计缺口单独阻塞，不得自行改语义。
