# Leaf Task: dispatch phase F1 Gate 机械同步修复

## 引用链

- 批次父节点：`doc/implementation/dispatch-e-batch.md`（集成 Agent
  `/root/dispatch_e_integration` 创建，位于集成分支 `dispatch-e-integration`
  commit `96431bd7`）。
- F1 报告来源：Gate 失败清单（全部为机械同步，禁止生产语义改动）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`96431bd78627ff49c5ff83e3f46ef1cbd45500e3`（`git rev-parse`
  验证；共享主 worktree `main` 只读，未改动）。
- worktree：`/Users/geek/workspace/skiff-fix-gate`，branch `gate-fix`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支。

## 任务合同摘要

把 F1 Gate 报告的本批次引入失败全部按既有机制做机械同步：

1. `-p skiff-compiler --lib`：`EXPECTED_STD_BUILD_ID` 刷新为 E1
   （std/task.skiff 新增后 std build identity 变化）观测 actual。
2. `-p skiff-compiler --test builtin_canonical_spelling`：`CURRENT_STD_BUILD`
   / `CURRENT_STD_LOCAL_ABI` 刷新为观测 actual。
3. `-p skiff-compiler --test std_package_imports`：std public symbol 计数
   91 → 93（std.task.status/cancel 新增 2 个）。
4. health 契约：test-runner 健康解码 `requestPending.derivedTask` →
   `requestPending.taskAttempt`（router 已发 taskAttempt）。
5. execution-boundaries：subject 从 `spawn_request_on_active_assembly_route`
   同步为 runtime 实际名 `task_request_on_active_assembly_route`。
6. `checks:runtime-crate-dag`：允许表补齐 capability-context/model →
   request-contract 两条真实 normal 依赖；同步移除已不存在的
   request-contract → capability-context 旧条目。
7. scripts-tests：loop-risk health/stress 夹具 `spawnedTasksActive` →
   `taskRequestsActive`；package-service-i02 regex `spawn` → `dispatch`。
8. `checks:artifact-identity`：检查器 relPath 从已目录化的
   `runtime/linked-type-plan/src/type_plan.rs` 修正为
   `runtime/linked-type-plan/src/type_plan/recoverable.rs`（一行路径修复，
   `sort_json_value` 现位于 recoverable.rs）。

## 预检结论（只读，锚定 baseline 96431bd7）

- identity 刷新机制：仓库既有 golden-closure 机制
  （`doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/
  tasks/P5-F445C-R4-official-std-authoring-golden-closure.md`）为“运行聚焦测试、
  把常量更新为观测 actual，再继续暴露下一断言”。本 leaf 按该机制迭代刷新。
- E1（`2fab6d66`）改动了 `compiler/core/src/prelude_registry.rs`，因此
  `EXPECTED_PRELUDE_ID` 也在同一条 golden 链上变化（F1 报告因测试停在 build
  断言未列出；属于同批次引入的同一类 test-only golden 同步）。
- D2（`3a0c138c`）从 router `ActorCounters` 移除了 `actor.task` lane（现为
  catalog/ownership/activation/invocation/control/lease 六个 lane），但
  test-runner 解码与夹具仍要求 `actor.task`。该漂移与 F1 item 4 同属 health
  契约同步；`cargo test -p skiff-test-runner` 的
  `http_entry_test_service` 集成测试（真实 isolated router）暴露了它。
- DAG 根因：`bc0a0d08` 给 capability-context/model 加了 normal 依赖
  request-contract，同时移除了 request-contract → capability-context
  normal 依赖，但 `scripts/check-runtime-crate-dag.mjs` 允许表未同步；若不
  移除旧条目，`validateEncodedDag` 会因编码环直接抛错。移除旧条目只收紧
  已不存在的允许边，不放松任何检查。
- `scripts/tests/loop-risk-stress.test.mjs` 需要 `ws` 依赖；worktree
  `scripts/node_modules` 不存在（git-ignored），用
  `pnpm install --frozen-lockfile --ignore-scripts` 安装后测试全绿，无 tracked
  文件变化。
- `cargo fmt --all -- --check` 的既有漂移全部位于本 leaf 未改动的
  compiler/core、compiler/lowering 等文件（baseline 已存在），未触碰。

## 写集

### 本次提交修改（13 个文件 + 本叶子文档）

| 文件 | 改动 |
| --- | --- |
| `compiler/driver/authoring/package_publication/tests.rs` | `EXPECTED_STD_BUILD_ID`、`EXPECTED_PRELUDE_ID` 刷新为观测 actual |
| `compiler/tests/builtin_canonical_spelling.rs` | `CURRENT_STD_BUILD`、`CURRENT_STD_LOCAL_ABI` 刷新为观测 actual |
| `compiler/tests/std_package_imports.rs` | std public symbol 计数 91 → 93 |
| `scripts/check-runtime-crate-dag.mjs` | 允许表加 capability-context/model → request-contract；request-contract 允许表清空 |
| `scripts/check-artifact-identity-single-source.mjs` | `type_plan.rs` → `type_plan/recoverable.rs` |
| `scripts/lib/runtime-execution-boundary-subjects.mjs` | subject/anchor 改为 `task_request_on_active_assembly_route` |
| `scripts/lib/runtime-execution-boundary-self-test.mjs` | self-test 夹具同步新函数名 |
| `scripts/tests/runtime-execution-boundary-checker.test.mjs` | registry 断言同步新函数名 |
| `scripts/tests/loop-risk-health.test.mjs` | 夹具 `spawnedTasksActive` → `taskRequestsActive` |
| `scripts/tests/loop-risk-stress.test.mjs` | 夹具 `spawnedTasksActive` → `taskRequestsActive` |
| `scripts/tests/package-service-i02-combined.test.mjs` | regex `spawn` → `dispatch`（fixture 已是 dispatch） |
| `test-runner/src/runtime_execution/wire.rs` | `derivedTask` → `taskAttempt`；移除 `actor.task` lane 要求 |
| `test-runner/src/runtime_execution/tests/support.rs` | 夹具 `derivedTask` → `taskAttempt`；移除 `actor.task` lane |

### 明确未写

- 生产语义代码：runtime/router/compiler production 零改动。
- `doc/reference/**`、`doc/architecture/**`、`doc/implementation/**` 既有文件
  （本叶子文档除外）零改动。
- 共享主 worktree `main` 零改动；无 push。
- 既有 fmt 漂移文件（compiler/core、compiler/lowering 等）未改。

## 证据矩阵

| Gate 项 | 验证命令 | 结果 |
| --- | --- | --- |
| 1/2/3 compiler | `cargo test -p skiff-compiler --lib` | 41 passed |
| 2 compiler | `cargo test -p skiff-compiler --test builtin_canonical_spelling` | 9 passed |
| 3 compiler | `cargo test -p skiff-compiler --test std_package_imports` | 7 passed |
| 4 test-runner health | `cargo test -p skiff-test-runner`（含 lib runtime_execution 61 项与 `http_entry_test_service` 真实 isolated router） | 全绿 |
| 4 test-runner 域 | `node scripts/verify.mjs --only test-runner` | passed |
| 5 execution-boundaries | `node scripts/check-runtime-execution-boundaries.mjs --self-test`（30 mutation cases）与 production 检查 | passed |
| 6 runtime-crate-dag | `node scripts/check-runtime-crate-dag.mjs --self-test`（12 cases）与 production 检查 | passed |
| 7 scripts-tests | `node --test scripts/tests/loop-risk-health.test.mjs scripts/tests/loop-risk-stress.test.mjs scripts/tests/package-service-i02-combined.test.mjs scripts/tests/runtime-execution-boundary-checker.test.mjs` | 26 passed |
| 7 scripts 全域 | `node scripts/verify.mjs --only scripts` | 638 tests passed + dev-sync passed |
| 8 artifact-identity | `node scripts/check-artifact-identity-single-source.mjs --self-test` 与 production 检查 | passed |
| checks 域 | `node scripts/verify.mjs --only checks` | 18/18 passed |
| 格式 | `cargo fmt --all -- --check`（本次写集内 Rust 文件无 diff）、`git diff --check` | 通过 |

## identity actual（观测值）

- `skiff-package-build-v10:sha256:293e1908d0a1b4bb749c7b9781a2da81968d779c1c2ab4ecc72c8924e1aef66b`
- `skiff-package-local-abi-v7:sha256:a5fc494093c3fe766717d6e2de0822288beba2aa691a09d1f447c97a9540df62`
- `skiff-prelude-v1:sha256:9e7d3f17f413582137306544eef42997faa25f57d3e66580da400031a4cddfa0`

## 提交

- branch：`gate-fix`，base：`96431bd7`。
- 提交信息：`fix(gate): sync F1 mechanical gate expectations (dispatch phase F)`。
