# P5-F420F Tooling path closure audit result

状态：`AUDIT_COMPLETE / REPAIR_REQUIRED`。本节点不是 gate verdict；F421 仍未解除。
在 exact executable candidate 上展开的 tooling plan 精确为 57 个 phase。本审计没有重跑
F420E 已证明的 phase 1–8，而是按 canonical 顺序逐项执行 phase 9–57，最终得到
`37 phase PASS / 12 phase FAIL`。Node test phase 9–55 合计
`473 tests / 447 passed / 26 failed`；两个非 TAP phase 56–57 均实际执行并通过。

## 1. Exact candidate、tree 与 ancestry

- executable candidate：
  `f8a7f6a25fc2e0ad6e6cf0e780ffe306acc938a7`；
- candidate tree：
  `7aea1e0a47e56aa0dde2d1d0efa19307e21ed849`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- audit checkout：
  `46b2d5e0ef284492e3abb961c53d5a4d509cc099`；
- audit checkout tree：
  `6a77aa966707e4bd97efaaf060c91744bd38c070`。

启动时：

```text
git rev-parse f8a7f6a25fc2e0ad6e6cf0e780ffe306acc938a7^{tree}
7aea1e0a47e56aa0dde2d1d0efa19307e21ed849

git merge-base --is-ancestor \
  7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d \
  f8a7f6a25fc2e0ad6e6cf0e780ffe306acc938a7
exit 0
```

candidate 同样是 audit checkout 的 ancestor；二者 tracked diff 只有
`P5-F420F-tooling-path-closure-audit.md`。因此所有被执行代码与 candidate 相同，audit
checkout 没有额外 executable diff。

## 2. Canonical 57-phase plan 与覆盖

实际运行：

```bash
node scripts/verify.mjs --only tooling --list
```

输出 `selectors: tooling`、`phases: 57`。前 8 个 phase 的名称、顺序和 F420E 记录完全一致：

| # | phase | F420E evidence |
| ---: | --- | ---: |
| 1 | `artifact-identity-validation.test.mjs` | 7/7 PASS |
| 2 | `check-artifact-identity-single-source.test.mjs` | 1/1 PASS |
| 3 | `command-caller-migrations.test.mjs` | 3/3 PASS |
| 4 | `command-execution-policy.test.mjs` | 10/10 PASS |
| 5 | `command-execution.test.mjs` | 13/13 PASS |
| 6 | `compiler-boundaries.test.mjs` | 10/10 PASS |
| 7 | `crate-public-api-characterization.test.mjs` | 82/82 PASS |
| 8 | `crate-public-api-cli.test.mjs` | 4/4 PASS |

本审计没有用完整 `--only tooling` 在首错处停止。从 phase 9 开始，每个 Node phase 都单独执行
canonical `node --test <path>`；phase 56 与 57 使用 plan 中的原命令。

| # | canonical phase | 实际结果 |
| ---: | --- | --- |
| 9 | `crate-public-api-gate.test.mjs` | FAIL，4/5 |
| 10 | `crate-public-api-graph.test.mjs` | PASS，64/64 |
| 11 | `crate-public-api-policy.test.mjs` | FAIL，2/3 |
| 12 | `crate-public-api-rustdoc.test.mjs` | PASS，5/5 |
| 13 | `dev-sync-recovery.test.mjs` | PASS，3/3 |
| 14 | `encrypted-storage-live-harness.test.mjs` | PASS，1/1；仅 hermetic harness test |
| 15 | `isolated-test-runtime-log-evidence.test.mjs` | PASS，3/3 |
| 16 | `isolated-test-runtime.test.mjs` | PASS，32/32 |
| 17 | `live-command-adapters.test.mjs` | PASS，6/6；仅 adapter self-test |
| 18 | `loop-risk-health.test.mjs` | PASS，8/8；未访问 live target |
| 19 | `loop-risk-stress-core.test.mjs` | PASS，11/11 |
| 20 | `loop-risk-stress.test.mjs` | PASS，7/7；见 §3 环境前置 |
| 21 | `managed-binary-lifecycle.test.mjs` | PASS，2/2 |
| 22 | `package-service-authoring.test.mjs` | PASS，9/9 |
| 23 | `package-service-dev-sync.test.mjs` | FAIL，4/5 |
| 24 | `package-service-ecosystem-http-fixture.test.mjs` | PASS，4/4 |
| 25 | `package-service-ecosystem-smoke-diagnostic.test.mjs` | PASS，5/5 |
| 26 | `package-service-ecosystem-smoke-lifecycle.test.mjs` | FAIL，2/6 |
| 27 | `package-service-ecosystem-smoke-real.test.mjs` | FAIL，31/36 |
| 28 | `package-service-generation-lifecycle-fixture-combined.test.mjs` | FAIL，0/1 |
| 29 | `package-service-generation-lifecycle-smoke-lifecycle.test.mjs` | FAIL，0/3 |
| 30 | `package-service-generation-lifecycle-smoke-oracle.test.mjs` | PASS，9/9；旧 WS lane，repair 后不可继承 |
| 31 | `package-service-generation-lifecycle-smoke-real.test.mjs` | FAIL，4/10 |
| 32 | `package-service-host-negative-probe.test.mjs` | PASS，3/3 |
| 33 | `package-service-i02-combined.test.mjs` | PASS，6/6 |
| 34 | `platform-source-probe-diagnostic.test.mjs` | PASS，3/3 |
| 35 | `platform-source-probe-host-evidence.test.mjs` | PASS，27/27 |
| 36 | `platform-source-probe-ownership.test.mjs` | PASS，9/9 |
| 37 | `platform-source-shared-target-probe.test.mjs` | PASS，40/40 |
| 38 | `platform-source-transport-combined.test.mjs` | PASS，1/1 |
| 39 | `run-skiff-tests-error-evidence.test.mjs` | PASS，4/4 |
| 40 | `runtime-execution-boundary-checker.test.mjs` | PASS，4/4 |
| 41 | `runtime-payload-codec.test.mjs` | PASS，8/8 |
| 42 | `runtime-stack-config.test.mjs` | PASS，9/9 |
| 43 | `runtime-stack-deploy.test.mjs` | PASS，9/9 |
| 44 | `rust-clippy-baseline.test.mjs` | PASS，6/6 |
| 45 | `skiff-instance-config.test.mjs` | PASS，4/4 |
| 46 | `skiff-instance-pid-metadata.test.mjs` | PASS，5/5 |
| 47 | `skiff-instance-supervisor-lifecycle.test.mjs` | PASS，9/9 |
| 48 | `skiff-source-test-suite.test.mjs` | PASS，10/10 |
| 49 | `skiff-test-cli.test.mjs` | PASS，8/8 |
| 50 | `test-runner-runtime-isolation.test.mjs` | FAIL，2/3 |
| 51 | `verify-live-plan-platform-source.test.mjs` | PASS，1/1 |
| 52 | `verify-live-registry.test.mjs` | FAIL，19/20；见 §3 环境前置 |
| 53 | `verify-rust-quality.test.mjs` | FAIL，3/4 |
| 54 | `verify-taxonomy.test.mjs` | FAIL，6/7 |
| 55 | `verify.test.mjs` | PASS，35/35 |
| 56 | `node scripts/check-package-store-discovery.mjs` | PASS；输出 `Package store discovery check passed.` |
| 57 | `cd vscode && pnpm run test:grammar` | PASS；见 §3 环境前置 |

35 个通过的 Node phase 合计 370 个 test，最小 phase 也有 1 个 test，没有零测试误判。
12 个失败 Node phase 合计 103 个 test，其中 77 passed / 26 failed。phase 56 是显式失败关闭的
authoring/store/dev-registry checker；phase 57 是直接加载 grammar、WASM 并执行大量
`expectScope` / `expectNoScope` 的 assertion script，二者都不是 Node TAP 的零测试成功。

## 3. 环境前置与重跑

worktree 启动时没有任何 `router/node_modules` 或 `vscode/node_modules`。这造成三个非代码现象：

1. phase 20 首跑 4/7 后有 3 个 `Cannot find module 'ws'`，普通 canonical 命令因失败路径留下
   本地 test socket 而不退出。本审计等待超过 4 分钟、确认 worker 只持有本地临时监听后，终止
   自己启动的 phase 20 进程。用 `--test-force-exit` 做的只读诊断取得完整首错：

   ```text
   explicit skip mode succeeds, preserves URL equals signs, and redacts output
   Error: Cannot find module 'ws'
   Require stack:
   - .../router/package.json
   ```

2. phase 52 首跑 17/20；其中两个 loop-risk test 同样先被
   `loop-risk-stress-live is missing required module(s): ws from router/package.json` 遮挡。
3. phase 57 首跑报告
   `ERR_MODULE_NOT_FOUND: Cannot find package 'vscode-textmate'`，并明确提示本地
   `node_modules` 缺失。

随后仅在 `router/` 与 `vscode/` 执行：

```bash
pnpm install --frozen-lockfile --ignore-scripts
```

两次都完全复用 lockfile/store，只生成被忽略的 `node_modules`，没有 manifest/lockfile/tracked
diff。原 canonical phase 重跑结果：

- phase 20：7/7 PASS；
- phase 52：19/20，只有 default verify 重复 phase 的真实代码失败；
- phase 57：PASS。

因此缺依赖和 phase 20 初始 hang 是已消除的环境前置，不进入 successor 代码 write set。

## 4. Blocker B1：crate-public-api policy 的两个旧 oracle

共同根因是 production canonical policy 从
`fbc3542a feat: define canonical deployment assembly contract` 起已经包含
`skiff-deployment`，但两个 test 仍冻结此前的两 crate 形态。

### phase 9 完整首错

```text
configured package resolution fails closed before probe and explicit absence is a skip
AssertionError: The input did not match
/configured public API crate\(s\) missing.*compiler-contract/
actual:
Error: configured public API crate(s) missing from workspace: skiff-deployment
```

`scripts/tests/crate-public-api-gate.test.mjs` 用
`MANAGED_CRATE_NAMES.slice(1)` 制造缺项。因此缺失值在定义上就是
`MANAGED_CRATE_NAMES[0]`，当前为 `skiff-deployment`。预期应从同一个 canonical policy 数组派生，
并对 regex 转义；硬编码 `compiler-contract` 已经证明会漂移。硬编码当前
`skiff-deployment` 虽能暂时通过，却会再次把 fixture 与其输入构造分裂，不是稳定 owner。

### phase 11 完整首错

```text
managed public API policy declares only the two terminal producer owners
actual:
[
  'skiff-deployment',
  'skiff-compiler-contract',
  'skiff-compiler'
]
expected:
[
  'skiff-compiler-contract',
  'skiff-compiler'
]
```

分类：两个都是 test/oracle drift，production policy 无缺陷。最小 owner/write：

```text
scripts/tests/crate-public-api-gate.test.mjs
scripts/tests/crate-public-api-policy.test.mjs
```

同一批 repair 可处理。最小探针：

```bash
node --test \
  scripts/tests/crate-public-api-gate.test.mjs \
  scripts/tests/crate-public-api-policy.test.mjs
```

## 5. Blocker B2：dev-sync 的 RuntimeAssembly v1 fixture

phase 23 首错：

```text
dev sync has one package phase and consumes generated service receipts before assembly
Error: assembly activation requires an exact RuntimeAssembly reference
```

`scripts/tests/package-service-dev-sync.test.mjs:169` 构造
`skiff-runtime-assembly-v1:sha256:...`；current
`requestAssemblyActivation` 只接受 canonical v2。生产 build/activation 路径与仓库 current
identity owner一致，失败发生在 fake compiler receipt 到达 fetch 之前。

分类：test fixture drift；没有 production 缺陷。唯一最小 write：

```text
scripts/tests/package-service-dev-sync.test.mjs
```

最小探针：

```bash
node --test scripts/tests/package-service-dev-sync.test.mjs
```

## 6. Blocker B3：HTTP-only cutover 后的 ecosystem / generation residue

这是一个可以批量闭合、但包含三个有顺序的子根因的 owner group。

### 6.1 Current receipt 的 package-test identity oracle

phase 28 在真实 Rust fixture authoring 后首错：

```text
Expected values to be strictly equal:
actual:
test.skiff/package/test.skiff/package-service-websocket-smoke/case-0
expected:
test.skiff/package/test.skiff/package-service-websocket-smoke
```

`test-runner/src/package_test_assembly.rs::compile_package_test_contract` 明确拥有
`test.skiff/package/<safe-coordinate>/case-{index}`。Rust fixture 是 current producer；
`scripts/lib/package-service-ecosystem-smoke-oracle.mjs` 与
`scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs` 仍省略 `/case-0`。

分类：shared Node oracle/fixture drift。这个失败先遮挡 phase 28 的 generation oracle；
修正 service id 后，后者会立刻继续读取已经不存在的第三个 WebSocket entrypoint、旧
`name/contract/operation` 字段。

### 6.2 Ecosystem smoke 仍消费第三个 WebSocket entrypoint

phase 26 的首个子失败：

```text
WebSocket never opens
expected: /ecosystem smoke I\/O deadline expired/
actual:   Cannot read properties of undefined (reading 'path')
```

phase 27 首错：

```text
ecosystem smoke waits for exact delayed readiness and creates one WebSocket
TypeError: Cannot read properties of undefined (reading 'path')
at scripts/lib/package-service-ecosystem-smoke-real.mjs:97
```

current v2 receipt 精确只有两个 HTTP entrypoint，并由 phase 24 的 4/4 与 Rust fixture共同证明。
但 `runPackageServiceEcosystemSmoke` 仍读取 `entrypoints[2]`，加载 `ws`，并返回
`skiff-cutover-production-websocket-component`。其 tests 也仍修改第三项
`contract/path`、要求 `exactly 3 entrypoints` 和旧扁平 selector。

分类：

- `scripts/lib/package-service-ecosystem-smoke-real.mjs` 是旧 production helper residue；
- phase 26/27 是相同 residue 的 test/oracle drift；
- 当前 `scripts/run-package-service-ecosystem-smoke.mjs` 已调用 I02 HTTP combined，而不是这个
  WebSocket helper，所以不得为保留死 helper 恢复 v1 receipt 或 Router assembly WS。

### 6.3 R05 generation lane 仍建立在已删除的 Assembly WebSocket 上

phase 29 首错：

```text
generation transcript deadline starts before candidate A authoring and preserves outer cleanup
TypeError: Cannot set properties of undefined (setting 'deployment')
at generationReceipt (...generation-lifecycle-smoke-lifecycle.test.mjs:184)
```

phase 31 首错：

```text
generation lifecycle transcript authors A then B and closes both generation pins
TypeError: Cannot set properties of undefined (setting 'deployment')
at generationReceipt (...generation-lifecycle-smoke-real.test.mjs:519)
```

两个 test 先在构造 fixture 时写 `entrypoints[2].deployment`，因此尚未到 production。
该遮挡移除后，`scripts/lib/package-service-generation-lifecycle-smoke-real.mjs` 仍会在
`entrypoints[2]` 创建 A/B WebSocket，并等待 connection pin/release ACK；
`scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs` 仍读取
`packageTest.name`、`unary.contract`、`unary.operation` 与 `websocket.operation`。这些字段都不在
v2 HTTP receipt 中。

phase 30 虽然 9/9 PASS，却使用 RuntimeAssembly v1、connection pin 及旧 R05 oracle；它没有经过
current receipt parser，不能证明 current generation lane，也不能在 repair 后继承。

分类：旧 Assembly WebSocket production CLI/helper 与其 tests 同时漂移。F420B 已依据 canonical
HTTP-only RuntimeAssembly v2 明确删除旧 Assembly WebSocket 正例；这里不需要新的 public
architecture 决策，也绝不能恢复第三 entrypoint、contract/operation 字段、dual-read 或 Router
WebSocket ingress。最小闭合方向是：

1. ecosystem smoke 改为 current HTTP `probe`，或移除已经没有 CLI owner 的旧 WS run helper；
2. R05 Assembly-WS generation transcript 退役；若保留 A/B author/activation evidence，只能改成
   明确命名的 current HTTP generation test，不能继续声称 connection pin；
3. 被 I02 使用的 bounded HTTP unary client与 RuntimePayload string validator迁到中性 HTTP
   helper，保留现有成功、oversize、redaction和decode tests；
4. 修正 shared package-test `/case-0` oracle，并保持 v2 exact two-entrypoint fail-closed。

这一组建议作为一个 repair batch处理，因为 shared receipt/oracle 改动会同时使 phase
24、28–31、33 的旧证据失效。最小聚焦验证至少覆盖：

```bash
node --test \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs \
  scripts/tests/package-service-ecosystem-smoke-lifecycle.test.mjs \
  scripts/tests/package-service-ecosystem-smoke-real.test.mjs \
  scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-oracle.test.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs
```

若选择删除旧 generation phase，result 必须逐项列出删除的 test 与保留下来的 HTTP utility
coverage，并证明 plan 数量下降与 discovery 精确一致，不能静默缩 selector。

## 7. Blocker B4：test-runner target inventory 旧 oracle

phase 50 首错：

```text
Cargo owns one ungated canonical cutover target and no recursive wrapper
actual:
[
  { name: 'package_service_contract_deployment' },
  { name: 'canonical_std_seed_bootstrap' }
]
expected:
[
  { name: 'package_service_contract_deployment' }
]
```

`canonical_std_seed_bootstrap` 由 `be79cc47 feat(test-runner): seed canonical std package`
加入，是当前独立 integration target；该 target 本身不是 recursive wrapper，且本审计没有发现
manifest 生产缺陷。

分类：test oracle drift。唯一最小 write：

```text
scripts/tests/test-runner-runtime-isolation.test.mjs
```

最小探针：

```bash
node --test scripts/tests/test-runner-runtime-isolation.test.mjs
```

## 8. Blocker B5：default verify 重复执行 package-store checker

依赖物化后，phase 52、53、54 的唯一剩余失败相同：

```text
Error: duplicate verify phase execution:
implementation:tooling:dev-sync-fixture and checks:package-store-discovery
```

真实 owner：

- `scripts/lib/verify-plan.mjs` 的 `scripts-dev-sync` builder 手工声明
  `implementation:tooling:dev-sync-fixture`；
- `scripts/lib/verify-checkers.mjs` 又把完全相同的
  `node scripts/check-package-store-discovery.mjs` 注册为
  `checks:package-store-discovery`；
- default `verify = tests + rust-quality + type-check + checks` 同时展开两者，
  `assertPlanIntegrity` 正确 fail closed。

这是 production verify-plan ownership defect，不是三个 tests 的 oracle drift。最小收敛应让
checker registry 成为单一声明，并把该 invocation 归到 `scripts-dev-sync` /
`implementation:tooling`，或采用等价的单 owner方案；不能放宽
`assertPlanIntegrity` 的 duplicate execution gate。预计最小 write：

```text
scripts/lib/verify-checkers.mjs
scripts/lib/verify-plan.mjs
```

现有 phase 52–55 tests 已足以防回归，不需要为通过而改断言。最小探针：

```bash
node --test \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-rust-quality.test.mjs \
  scripts/tests/verify-taxonomy.test.mjs \
  scripts/tests/verify.test.mjs
node scripts/verify.mjs --list
node scripts/verify.mjs --only tooling --list
node scripts/verify.mjs --only checks --list
```

## 9. 建议的单一 successor repair task

建议 successor 把上述五组作为一个有界 tooling closure batch；允许写入仅限：

```text
scripts/tests/crate-public-api-gate.test.mjs
scripts/tests/crate-public-api-policy.test.mjs
scripts/tests/package-service-dev-sync.test.mjs

scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-ecosystem-smoke-real.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/package-service-ecosystem-smoke-lifecycle.test.mjs
scripts/tests/package-service-ecosystem-smoke-real.test.mjs

scripts/run-package-service-generation-lifecycle-smoke.mjs
scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs
scripts/lib/package-service-generation-lifecycle-smoke-real.mjs
scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs
scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs
scripts/tests/package-service-generation-lifecycle-smoke-oracle.test.mjs
scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs
scripts/lib/package-service-i02-combined-real.mjs

# 可选新建，用于从旧 generation 命名中抽离仍被 I02 使用的 current HTTP utility
scripts/lib/package-service-http-unary.mjs
scripts/tests/package-service-http-unary.test.mjs

scripts/tests/test-runner-runtime-isolation.test.mjs
scripts/lib/verify-checkers.mjs
scripts/lib/verify-plan.mjs

successor task/result 文档
```

`test-runner/**` producer、Router、artifact/compiler/runtime、manifests、lockfiles、fixture source与
验证完整性 gate 都不需要修改。若 successor 发现必须恢复 RuntimeAssembly WebSocket 或修改
这些 production owners，应再次停止；那将违反 F420B 已冻结的方向。

建议验证矩阵：

| batch | focused gate |
| --- | --- |
| crate policy | phase 9 + 11 两文件 direct test |
| dev-sync identity | phase 23 direct test |
| HTTP/generation closure | §6 列出的 current fixture、lifecycle、I02 tests；若删除旧 phase则先后各跑一次 `--only tooling --list` 并记录合法 count delta |
| test-runner inventory | phase 50 direct test |
| verify ownership | phase 52–55 四文件；default/tooling/checks 三个 list |
| terminal tooling | `node scripts/verify.mjs --only tooling`，不得在首错停止后的局部结果冒充 verdict |
| hygiene | `git diff --check`、`git status --porcelain` |

## 10. Evidence inheritance

repair 后失效、必须重跑：

- 所有 12 个 failing phase；
- phase 24（shared receipt oracle/helper 会改）；
- phase 30（虽通过但属于旧 Assembly-WS/v1 oracle）；
- phase 33（I02 的 HTTP helper/import 或 shared oracle会改）；
- phase 51–55 与 default/list plan（verify-plan ownership会改）；
- 57-phase count与所有基于该 count 的 aggregate；
- 任何最终 tooling verdict。本审计从未运行完整 verdict，因此不存在可继承的 PASS。

仍可作为 focused、非 gate 事实继承：

- candidate/tree/F415 ancestry与 audit checkout 只有文档 diff的证明；
- F420E phase 1–8 的同 executable evidence；
- phase 10、12–22、25、32、34–49、56、57 的原始通过证据，只要 successor 未触碰其 owner；
- missing `ws` / `vscode-textmate` 的环境诊断与 frozen install 后通过结果。

即使上述 focused 事实可继承，successor 仍必须在最终 repair tree 上跑一次完整
`--only tooling`，因为 test discovery、plan ownership及 phase count会变化。

## 11. 边界与结论

- 未运行会在首错停止的完整 tooling verdict；
- 未访问 stable/live、instance、watch registry 或固定端口；
- 没有 merge、rebase 或 push；
- 除本 result 外没有 tracked write；依赖物化只写被忽略的 `node_modules`；
- 没有修改 production、test、fixture、manifest、lockfile或验证计划；
- 没有发现需要修改权威架构的缺口：F420B 的 HTTP-only RuntimeAssembly 决策已经足以判定旧
  Assembly WebSocket positive lane应迁移或退役。

因此本审计完整闭合了 phase 9–57 的可见失败与遮挡，但 candidate 仍有 12 个 failing phase，
F421 保持阻断。
