# Phase 0 result: baseline ledger and trustworthy Live foundation

Status: complete（见下方 Known residual risks 的两项 deferred 证据）

## Candidate commits/trees

隔离 Live 候选（candidate-pass 时点）：

| 仓库 | commit | tree |
| --- | --- | --- |
| skiff（bcvm/p0-scripts） | 7e018b60 | 8c95fcbe |
| internals（bcvm/p0-host） | d1423242 | 2756b13e |
| skiff-packages | db4ddd9e | 35f31026 |

合流后各仓 main：

| 仓库 | main commit | 说明 |
| --- | --- | --- |
| skiff | 3d468e38 | merge bcvm/p0-doc（c88b34bc）+ bcvm/p0-router（0bace4a8）+ bcvm/p0-scripts |
| internals | 1d89a8e | merge bcvm/p0-host |
| skiff-packages | db4ddd9e | 本阶段无改动 |

## Requirement IDs closed/deferred

- 关闭：R-001..R-243 ledger 全部入账（requirements/ledger.md），Phase 0 负责的 registry/selector/strict-host-tools/
  actor-disposition 条目 closed；语义实现类条目按 ledger 分派到 Phase 1..9（TBD 状态）。
- 本阶段无 retirement-only 条目。

## Focused gate table

| selector / 命令 | 结果 | 说明 |
| --- | --- | --- |
| `node scripts/verify.mjs --only checks` | 17/17 PASS | 含 P0E 修复的 5 个既有失败 |
| `git diff --check` | PASS | 三仓 merge 范围干净 |
| `pnpm --dir scripts type-check` | PASS | 合流后 scripts 类型检查 |
| `node scripts/verify.mjs --only tooling` | PASS | 合流后 tooling selector |

## Phase-specific proof

- requirement ledger 243 条 + benchmark-baseline.md（W16 host_tools_profiling 固定输入/统计口径）。
- strict host-tools 断言单元测试：`client/e2e/host-tools-strict.test.mjs` 29 tests 全绿
  （terminal/empty-answer/zero-tool-calls/shell-run/pid/sample 全部失败路径）。
- CLI 级故障注入非零退出（stable 上逐项验证）：`wrong-pid`（runtime-pid-not-alive）、
  `error-terminal`、`stopped-terminal`（terminal-not-completed）三项目非零退出。
  empty-answer / zero-tool-calls / missing-sample 三项目由同一 strict 断言路径 + 单元测试覆盖，
  CLI 级重跑 deferred（见 Known residual risks）。
- actor idle/lease ordering 与 exact-build fence 两个已知问题转为故意失败测试
  （`router/src/actor/ownership.rs`），ledger disposition 见 requirements/actor-disposition.md（Phase 3A/7 修复）。
- harness 隔离性：临时 Mongo（45167..45354 动态端口）、动态端口 45434/45435/45802..45982、
  显式子进程、只读 host-tools workspace，未触碰 stable 4000..4007 与稳定 host home。

## Isolated Live manifest

- selector：`node scripts/verify.mjs --only router-live:agine`（同一隔离栈依次运行
  chat-smoke → host-tools --check → strict full host-tools）。
- 最终 PASS 运行（run6）manifest 生成于
  `/var/folders/.../skiff-router-agine-live-9MgXwA/router-agine-live-manifest.json`
  （PASS 后临时目录被 harness 清理，manifest 原文不保留；evidence 摘要如下）。
- evidence（run6，engine: legacy-tree）：
  - compiler SHA 33950502b2e1…，router SHA b28b7b60…，runtime SHA 52b1a4f1…
    （`skiff-p0-scripts/build/cargo-target/debug/`）
  - assembly `skiff-runtime-assembly-v3:sha256:bbbbc7ac…`，config snapshot
    `skiff-runtime-config-snapshot-v1:483490c6…`
  - chat-smoke PASS（reply 12 chars）；host-tools check PASS；strict full host-tools PASS
    （terminal=completed，16 tool calls，sample 728MB）。
- 重复生成 manifest 一致性验证 deferred（见 Known residual risks）。

## Stable merge commits and Live receipt

- 三仓 main merge commits 见上表；`router-live:agine` 在合流后 main 上重新注册生效
  （scripts/check-router-agine-live.mjs 已并入 skiff main）。
- Router health（stable dev，127.0.0.1:4001）：profile=dev，releaseCount=4，
  buildIds=[2ba4f7c3（codex-relay）, 688ae2cc（agine.ai/api）, 87c2deee（aihub）, 97aa0f3b]，
  loadedBuildIds=[2ba4f7c3, 688ae2cc, 87c2deee]；routerArtifactRoot=
  `/Users/geek/workspace/skiff/.stack/dev-home/artifacts`。
- stable chat-smoke：PASS（reply 12 chars，accepted 56.9ms / first_visible 760.9ms）。
- stable host-tools strict full：PASS（strict assertions ok，154139ms，只读 doc workspace，
  runtime PID 34421 显式传入，sample 非空）。
- 本阶段未修改 production compiler/router/runtime 代码（合流只含 scripts/checks/tests/docs），
  稳定二进制未重建；当前 stable 二进制 SHA：router 27244803…，runtime 84286bb9…。

## Legacy/fallback reverse-search ledger

- Phase 0 未引入任何 VM/ISA 语义，无 legacy 反向回退路径；全链路 engine=legacy-tree 并已在
  manifest 明确标注（不声称 VM 证据）。

## Performance/layout deltas

- n/a（Phase 0 无性能/layout 变更；baseline 口径见 requirements/benchmark-baseline.md）。

## Known residual risks owned by the next phase

1. host-tools `--check` 参数经 npm workspace 透传失效（`npm run e2e:host-tools -- --check` 实际
   执行完整对话）。不影响 gate 正确性（完整对话 + strict 断言 PASS），但 phase 2 意图的快速
   check 未生效；Phase 1 若继续使用该 selector，需改为直接 `node e2e/host-tools.mjs --check`。
2. 注入 CLI 级证明剩余 3 项（empty-answer / zero-tool-calls / missing-sample）与重复生成
   manifest 一致性未在本阶段完成 CLI 级验证（单元级已覆盖）；Phase 1 首轮 evidence epoch 补齐。
3. actor idle/lease ordering 与 exact-build fence 失败测试保持 FAIL（README §5.8，
   Phase 3A/7 修复后转绿）；Phase 0 合流后 `cargo test router` 存在这两个已知失败，不属于本阶段 gate。

## Verdict

Phase 0 complete：requirement ledger、`router-live:agine` 组合 selector、strict host-tools、
失败测试基线与 manifest 基线均已落地；隔离 Live PASS、三仓 main 合流并 push、stable closure
（chat-smoke + host-tools strict full）PASS。两项 deferred 证据（注入 CLI 级 3 项、manifest 重复生成）
由 Phase 1 首轮 evidence epoch 补齐。
