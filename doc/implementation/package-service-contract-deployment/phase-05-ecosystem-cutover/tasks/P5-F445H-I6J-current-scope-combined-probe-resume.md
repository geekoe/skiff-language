# P5-F445H-I6J current-scope combined probe resume

状态：`READY`。

本节点是 I6 merged candidate 的便宜 combined probe owner，不是 production、test 或 fixture
实现节点。它只在同一个冻结 baseline 上重建所有 I6 parent 的非零聚焦 selector、四包 locked
接线和必要反向搜索；PASS 才解除 independent I6 acceptance。

## 1. 直接父节点与追溯

直接父结果：

- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`
- `P5-F445H-I6E2R-http-current-scope-resume-result.md`
- `P5-F445H-I6E3-websocket-current-scope-resume-result.md`
- `P5-F445H-I6E4R2-time-eval-fixture-closure-result.md`
- `P5-F445H-I6E5-file-current-scope-resume-result.md`
- `P5-F445H-I6E6-actor-current-scope-resume-result.md`
- `P5-F445H-I6D-host-operation-current-scope-result.md` 的 D2 response-sink checkpoint
- `P5-F445H-I6S-service-timeout-scope-reduction-result.md`

以上节点经 `P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md` 与
`P5-F445H-I6R-current-scope-refresh-preflight-result.md` 追溯到 phase-05 DAG，并继续追溯到唯一
权威设计 `doc/architecture/package-service-contract-deployment.md`。本合同不重定义任何 scope、
timeout、winner、cleanup、service 或 wire 语义。

## 2. 冻结候选与父提交

| 项 | commit / tree |
| --- | --- |
| merged baseline | `f12ee51b3c77635d8d182e09152c995ae0ac35d0` / `ea44d0e04f89b22573c6bd2dd63569ad20bdc808` |
| branch | `codex/p5-f445h-i6j-combined-probe` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6j-combined-probe` |

只读预检已确认以下 implementation/design commits 全部是 merged baseline 的祖先：

```text
E1 carrier       ba66719e03cbabde2e159b94761cc1a1c71b35d2
HTTP             860a2a9cd098a531354891591fbe386b8e0ad7b3
WebSocket        dbab9c4bc90ff167d4a266cf9209f80f1544b334
time fixture     80a94bb886dbec35290aaf5ad4861fbab58d4b6b
file             05c5e2bfb567ebf2468e9e8039782339b2577bfa
Actor            b90be5785cd2a2f69c658592fd10ca28220ef69e
response sink    44568435aee8e59fe2437c0cbc3e0a60f1315f50
service decision a45389c6083ddd5b57b6d2ed202c1b3816f8f468
```

任何 production、test、fixture、Cargo manifest 或 lockfile 变化都会使本结果失效。

## 3. 旧合同为何需要 resume

I6R §8.6 要求在三个 consumer merge 前新增
`runtime/host/tests/f445h_i6_current_scope.rs`、记录真实 RED，再在合流 commit 上运行该 binary。
当前 baseline 已完成所有 parent merge，且该 test 文件不存在。本节点又明确禁止修改 tests，
因此不能伪造 merge-before RED，也不能把缺失 binary 的零 selector 当作 PASS。

本 resume 不改变原 combined cases，而是使用各 parent 已提交的 hermetic fake/paused-time receipt
在同一 merged tree 上聚合重建它们。它是便宜合流探针，不替代后续独立 I6 acceptance 的四个完整
crate gate。

## 4. 唯一允许写集与禁止项

唯一允许写集：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6J-current-scope-combined-probe-resume.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F445H-I6J-current-scope-combined-probe-result.md
```

禁止修改 production、tests、fixtures、Cargo manifests、`Cargo.lock`、权威设计和任何父结果。禁止
full crate/stage gate、network/server、stable/live、MongoDB、依赖安装、push、merge 或 rebase。
Cargo 必须 `CARGO_NET_OFFLINE=true`；构建缓存只允许位于本 worktree。

## 5. 非零 selector 与 combined probe

下列每个 selector 都先 `--list`、再执行；listing 与 execution 必须精确相同且非零：

```bash
cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt -- --list
cargo test -p skiff-runtime-eval f445h_i6_carrier_delivery_receipt -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --nocapture
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --list
cargo test -p skiff-runtime-eval f445h_i6_time_projection_to_pending -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --list
cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --nocapture
cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --list
cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture
cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --nocapture
cargo test -p skiff-runtime-eval \
  f445h_e4r7_stream_deadline_pending_unary_preserves_inherited_request_carrier -- --list
cargo test -p skiff-runtime-eval \
  f445h_e4r7_stream_deadline_pending_unary_preserves_inherited_request_carrier -- --nocapture
```

预期 inventory 为 `5 + 11 + 7 + 6 + 7 + 1 + 6 + 1 + 4 + 15 + 4 + 1 = 68`
tests。随后只运行一次四包 locked 接线：

```bash
cargo check -p skiff-runtime-capability-context -p skiff-runtime-native \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
```

这 24 条 selector 命令加 1 条 locked check 构成本节点唯一动态 combined probe，共 25 条命令。

## 6. 静态边界

```bash
rg -n '\$/cancelRequest|-32800|CancelError' runtime router std
rg -n 'service_dispatch|outbound_service' runtime/host/tests
rg -n 'consumer dependency timeout|callee operation timeout|ServiceTimeoutConfig' \
  doc/architecture/package-service-contract-deployment.md doc/reference/runtime.md \
  runtime/host/tests runtime/eval/src
git diff --check
```

peer cancellation、legacy service relay 或第一版独立 dependency/callee timeout 不得成为 combined
receipt。搜索结果必须按 production/test/doc 分类，不能只按命中数机械判定。

## 7. 完成标准与停止条件

PASS 要求：

1. baseline/tree 与父 ancestry 不变；
2. 12 个 selector 的 listing/execution 均非零且数量一致，总计 68/68；
3. 四包 locked check PASS；
4. 静态检查未发现 peer cancel、legacy relay 或 service timeout scope-reduction 逆转；
5. 除本 task/result 外没有 tracked 写入，worktree clean；
6. result 固化精确命令、exit、计数、失败分类、commit/tree 与
   `READY_FOR_INDEPENDENT_I6_ACCEPTANCE`。

任一 selector 零匹配、失败、数量不一致，locked check失败，或静态边界逆转时立即分类为
`MERGED_PROBE_FAIL`，只提交 result，不修 production/tests。若命令需要 network、stable/live、
Mongo、Cargo/lockfile、production/test/fixture 写入或 full gate，返回
`TASK_NOT_EXECUTABLE`。本节点不得自行进入独立 acceptance。
