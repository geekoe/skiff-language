# P5-F331 Service error channel convergence probe

状态：Ready。

## 直接父节点

- frozen R0：
  `P5-F327-service-error-core-independent-acceptance-result.md`
- R1 ordinary/ingress：
  `P5-F328-service-error-ordinary-ingress-consumer-result.md`
- R2 async/stream/cancel：
  `P5-F329-service-error-async-stream-consumer-result.md`
- R3 service test effect：
  `P5-F330-service-error-test-effect-consumer-result.md`
- original R4 matrix：
  `P5-F319-service-error-channel-delta-audit-result.md`

本任务是R1–R3合流后的唯一cheap combined integration probe和F319 R4 test-only convergence owner。PASS后只
解除A5 independent acceptance及W2-W正式开工，不代表A5或Phase 5 PASS。

## 候选与写入范围

- 候选：创建worktree时integration HEAD；result必须记录commit/tree及F328/F329/F330均为ancestor。
- production写入：无。
- 允许新建一个专用test-only convergence fixture，优先
  `runtime/eval/tests/service_error_channel.rs`；若public integration surface不足，可在
  `runtime/eval/src/assembly_execution/`新建`#[cfg(test)]`模块并只为它增加`#[cfg(test)] mod`接线。
- 不回写R0–R3 production或各自co-located fixture，不修改WebSocket/compiler、request/host/transport/
  router/telemetry/std或权威设计。

## 必须验证

### 合流接线

- ordinary和async unary都产出同一fixed carrier，并由F328 central dispatcher只对internal origin import；
- async stream typed terminal经过capability carrier到program-stream consumer后调用同一R0 import；
- ingress只向上交fixed carrier；
- service effect调用同一R0 export/import，Package effect保持local；
- legacy typed response透传，generic response Protocol；没有lane-level classifier。

### R4矩阵

在真实linked `AssemblyExecutionImage`/provider heap/caller heap入口上汇合：

- B1 public exact、B2 dependency owner；
- B3 unlinked middle hop opaque raw bytes，下一linked caller恢复；
- B4/B5/B6 private/nonclosed/encode failure只生成一次Internal；
- B7 identity/payload mutation fail closed；
- B8 platform、B8a Resource Package path；
- B9 imported Internal三跳原bytes；
- S1各service自己的local stack、S2 local rethrow与remote new stack；
- T2 service effect public/Internal/opaque，T1 Package effect不wire；
- ordinary、async、stream、ingress、service effect各至少一个public或Internal真实入口；
- cancel/control/generic legacy/heap isolation负例。

不得在R4重新实现classifier；可组合运行R0–R3已有真实路径selector，并新增最小跨lane探针专门证明合流接线。

### 隐私与反搜

- fixed bytes不含private type/value、callee source/path/function/stack；
- imported未处理hop不decode/re-encode，不换correlation；
- `materialize_provider_error` passthrough、message→ProviderUnavailable legacy、service test throw direct clone、
  fixed stream downcast路径为零或仅明确local/general分支；
- Resource不在platform registry，operation contract无error set。

## 验证owner

先列selector，再运行最小不重复组合：

```bash
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast
cargo test -p skiff-runtime-eval --lib service_error_consumer --no-fail-fast
cargo test -p skiff-runtime-eval --lib assembly_execution::async_stream_cancel --no-fail-fast
cargo test -p skiff-runtime-eval --lib program_stream --no-fail-fast
cargo test -p skiff-runtime-eval --lib service_error_channel_contract_operation --no-fail-fast
cargo test -p skiff-runtime-capability-context --lib --no-fail-fast
cargo check -p skiff-runtime-eval --lib
git diff --check
```

selector必须非零。可新增一个exact convergence selector。完整eval仍有两个既知generic WebSocket blocker，
本任务不得重复完整eval或修改它们。不运行workspace/root/stable/live。

## 失败收敛

若合流后出现编译/API漂移，先收集同一状态下全部独立失败并分类；只在test-only fixture范围内修探针。production
问题返回FAIL并拆新任务，不能在本R4顺手修。不得逐个重跑完整路径。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f331-error-convergence`
- branch：`codex/p5-f331-error-convergence`
- 风险：高，test-only real-path convergence；新的一次性Agent，5分钟内先运行compile/selector发现合流断点；
- 提交test/result并返回candidate commit/tree、R4矩阵、反搜、PASS/FAIL和blocker；
- 不push、不承接A5/W2-W。

