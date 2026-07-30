# P5-F445H-I7-T0 Internals isolated gate tooling result

## Outcome

```text
PASS
T0_COMPLETE = YES
BLOCKING_ISSUES = 0
TASK_SCOPE_EXPANDED = NO
C_UNBLOCKED_BY_T0 = YES
A_UNBLOCKED_BY_T0 = YES
J_T0_PREREQUISITE_SATISFIED = YES
```

T0 恢复了 Internals current isolated gate 的 target-specific authoring 与真实 test dispatch；
没有改变任何 business service/client/Host 语义或公共契约。C/A 现在可以针对同一冻结 Skiff source
并行消费该 shared tooling。J 仍必须等待 S1/P0/C/A/U 等其它父节点；`T0_COMPLETE` 不等于
I7 或 J 完成。

## 1. Parent chain and exact inputs

直接合同 checkpoint：

```text
Skiff commit 65ea6175552566afd6b4401932773d403859428f
Skiff tree   f56713c34b57fd7f25a8e352116fe64511a1ea69
P5-F445H-I7-T0-internals-isolated-gate-tooling.md
```

直接父节点是 I7R result 与 I6K-R4 acceptance result；I6 为
`PASS / I6_ACCEPTED = YES / I7_UNBLOCKED = YES`。

| 项 | 值 |
| --- | --- |
| frozen Skiff source | `54fb087f122c53aed5c017260c7bca43e2b54404` / `008d3a05927cdf845004db980d1b46de263612be` |
| Internals baseline | `19d41001f048efc0b70e13c21d105a855ddd86e2` / `15c48e07cc3d51794269719c606c87169bd0ee72` |
| Internals integration branch | `codex/package-service-phase-05` |

frozen Skiff source 相对 I6 frozen candidate 的相关 runtime、CLI 与 test-runner source无 diff。

## 2. True preflight RED and classification

baseline 已使用 `--artifact-root` 并拒绝旧 `--packages-dir` /
`--service-artifact-root`，但 canonical workflow refactor 丢失两项原 gate 语义：

1. `check-isolated-service-graph.mjs` 与 `test-isolated-service.mjs` 仅校验 target，随后无条件执行
   完整 `{codex-relay,aihub,agine,account}` publish + assembly；target 不参与 selection。
   C 会被未迁移 A 或独立 Account 遮挡，C/A 无法隔离并行。
2. `test-isolated-service.mjs` 从不 spawn `skiff test`，所以 `.test.skiff` 没有执行，零选择也无法
   由 `--require-tests` fail closed。

分类是 shared test-tooling implementation gap，不是 service 业务缺陷、Skiff CLI 设计缺口或
public contract 变化。

## 3. Implementation and integration identity

| 项 | 值 |
| --- | --- |
| leaf branch | `codex/p5-f445h-i7-t0-tooling` |
| leaf worktree | `/Users/geek/workspace/internals-p5-f445h-i7-t0-tooling` |
| implementation commit | `320100b10c955b90469534a99f0d2a6fd4cbf82b` |
| implementation tree | `1a906f87c663439c022b9ee4f1ad19ed3471f6f1` |
| Internals integration owner | `/root/phase05_internals_integration_steward` |
| integration merge | `1b28fea6925209d668034707a6a57cb72e3c4707` |
| integration tree | `1a906f87c663439c022b9ee4f1ad19ed3471f6f1` |

implementation parent 是精确 Internals baseline。integration 使用 `git merge --no-ff`，parents 为
baseline与 implementation；merged tree 与 leaf tree bit-identical。Internals integration worktree
clean，leaf worktree与branch已删除，未 push。

## 4. Exact write ownership

唯一写集为七个授权 tooling/test 文件：

```text
scripts/check-isolated-service-graph.mjs
scripts/isolated-service-graph.mjs
scripts/isolated-service-graph.test.mjs
scripts/prepare-canonical-assembly.mjs
scripts/prepare-canonical-assembly.test.mjs
scripts/test-isolated-service.mjs
scripts/test-isolated-service.test.mjs
```

没有修改任何 service/client/Host production、service fixture/package manifest、
`canonical-source-provenance.mjs`、Skiff 或 `skiff-packages`。
`prepare-canonical-assembly` 扩展只复用现有唯一 std bootstrap/authoring owner，没有复制
bootstrap implementation。

## 5. Delivered behavior

- target package/service closure：
  - Relay check `{Relay}`；
  - AIHub `{Relay, AIHub}`；
  - Agine `{Relay, AIHub, Agine}`；
  - Account 独立 `{Account}`。
- Test 只发布 target 的 service providers，不发布 target 本身：
  - Relay `{}`；
  - AIHub `{Relay}`；
  - Agine `{Relay, AIHub}`；
  - Account `{}`。
- package closure同样 target-specific：
  - Relay/AIHub只需要 std + llm-api + llm-providers；
  - Agine需要 std + llm-api + llm-providers + agent + http-session + track；
  - Account只需要 std + http-session。
- 三仓 absolute provenance validation 仍先 fail closed。
- check path包含 target、生成 exact deployment receipts并 resolve一个 assembly。
- test path在同一个 owned temp artifact root中准备 package/providers，随后精确 spawn：

  ```bash
  node <frozen-skiff>/scripts/skiff.mjs test <target-root> \
    --artifact-root <owned-temp>/ecosystem-store \
    --deny-skips \
    --require-tests
  ```

- 所有 child 共用 owned `<temp>/cargo-target`，通过 `CARGO_TARGET_DIR` 隔离并行 build cache；
  artifact/build temp在 signal/error 后沿既有 single cleanup owner 清理。
- inherited stable/shared env keys，包括 Mongo/service DB/telemetry、dev home/reload、旧 test
  activation/artifact/ingress，在 child env 中删除。
- wrapper 不读取 `27017`、`.skiff-instance` 或 reload URL。non-live runtime/Mongo 继续由 frozen
  Skiff CLI 的 managed isolated runtime owner创建，T0 未复制该 owner。
- unknown/missing target、outside artifact root、legacy flags、stable path/shared Mongo mutation
  与零 test selection 均 fail closed；`--require-tests` 是零选择动态 owner。

## 6. Evidence ledger

| 层级 | 命令 | owner | commit/tree | 结果 | 覆盖 |
| --- | --- | --- | --- | --- | --- |
| focused fake/unit | `node --test scripts/isolated-service-graph.test.mjs scripts/test-isolated-service.test.mjs scripts/prepare-canonical-assembly.test.mjs` | T0 leaf | `320100b1` / `1a906f87` | PASS `22/22`，fail/skip=0 | target package/service closures；Agine exact 3-service check + one assembly；AIHub providers-before-test；single artifact/Cargo root；exact CLI；legacy/stable/Mongo negatives；cleanup owner |
| syntax | `node --check` on all seven touched `.mjs` | T0 leaf | same | PASS `7/7` | production + test module syntax |
| hygiene | `git diff --check` | T0 leaf | same | PASS | leaf whitespace |
| integration hygiene | `git diff --check` and clean status | Internals integration steward | `1b28fea6` / `1a906f87` | PASS | merged bit-identical tree |

第一次 focused run 有一个 test-only expected-label assertion 错误：actual final invocation携带 service
metadata，但期望写成 `test`。只修改 test mapping 后同一 suite `22/22` PASS，没有 production 或
行为回归。最终证据只锚定上述 GREEN tree。

按合同没有运行真实 service matrix、Skiff CLI/Cargo、stable/watch/reload、network、Mongo、
OAuth、browser或 live。真实 C/A service tests与 J final matrix各有后续唯一 owner，T0 不提前
冒充其 receipt。

## 7. Acceptance matrix

| 条款 | 代码/结构证据 | 反向/负向证据 | 测试 | 判定 |
| --- | --- | --- | --- | --- |
| target-isolated check | canonicalTargetPlan + target-aware run workflow | Account不进入Agine；unknown target fails | exact three-service fake assembly | PASS |
| actual service test | isolatedServiceTestInvocation / runCanonicalIsolatedServiceTest | test path无assembly/target prepublish；legacy flags rejected | providers then one test fake | PASS |
| current frozen CLI | exact args with artifact-root/deny-skips/require-tests | no config/packages-dir/service-artifact-root/live | exact invocation + mutations | PASS |
| artifact/build isolation | withCanonicalAssembly owned ecosystem-store/cargo-target | outside root和inherited shared env fail/strip | single-root assertions | PASS |
| no business/public change | seven-file exact write set | service/client/Host/fixture/provenance diff=0 | Git identity/write-set audit | PASS |
| cleanup/integration | existing signal/error owner retained | no stable state | existing cleanup units + clean merged worktree | PASS |

## 8. Blocking, residual risk and invalidation

```text
BLOCKING_ISSUES = 0
```

Residual risk严格留给后续 owner：T0 未运行真实 Skiff/Cargo/service matrix；C/A 可能暴露 service
source/fixture缺陷；J 必须在最终精确 tree上运行 real hermetic matrix。

Skiff test CLI/isolation contract、Internals graph/provenance/helper、七个文件、repo identity或
temp/env ownership变化会使 T0 证据失效。

## 9. DAG release

```text
T0_COMPLETE = YES
C: T0 dependency released (仍等待其其它父节点)
A: T0 dependency released (仍等待 S0/P0 及其它父节点)
U: 仍 blocked by A + C
J: T0 prerequisite satisfied；仍 blocked by S1/P0/C/A/U exact integrated identities
```

当前需要用户决策：无。

