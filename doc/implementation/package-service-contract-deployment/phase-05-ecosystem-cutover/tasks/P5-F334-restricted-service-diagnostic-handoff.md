# P5-F334 Restricted service diagnostic handoff

状态：Completed。结果见
`P5-F334-restricted-service-diagnostic-handoff-result.md`。

## 直接父节点

- 当前 wire/observability owner 与 RΔ 范围：
  `P5-F333-wire-observability-delta-audit-result.md`
- 已冻结 runtime service error channel：
  `P5-F332-service-error-channel-a5-acceptance-result.md`

父节点已沿 F280/F319/F331 连接唯一权威设计。本任务只实现 F333 的 RΔ corrective prerequisite，不改变
`ServiceErrorEnvelope`、错误分类、逐跳新栈或 telemetry wire 语义。

## DAG、候选与写入边界

- blocked-by：F332 A5 PASS；production 基线
  `677305be0a0fa6f490a937fefdc7fd4e7cab1b35`。
- 当前为实现检查点。完成后解除 F335 shared response.error v2/telemetry checkpoint；尚不接 host、
  Router 或 telemetry service，不形成 W2-W 稳定候选。
- 风险：高，provider heap 生命周期与受限诊断所有权。

允许 production 写入：

- `runtime/capability-context/src/{telemetry.rs,lib.rs}`；
- `runtime/eval/src/assembly_execution/{service_error_channel.rs,ordinary.rs,async_stream_cancel.rs}`；
- `runtime/eval/src/{eval_context.rs,program_execution.rs}`仅在 service test effect 或上下文接线确有需要时；
- `runtime/eval/src/capabilities.rs`只允许补充新 typed capability re-export。

允许测试写入上述文件的 co-located tests。禁止修改 model/service envelope、request/host/transport、Router、
telemetry service、compiler/std、权威设计及 generic WebSocket。若 typed handoff 无法在此依赖方向形成，
返回 `TASK_NOT_EXECUTABLE`，不得改成 generic JSON callback 或扩张 owner。

## 完成标准

1. capability-context 定义一个 typed、clone-safe 的 restricted service diagnostic value 与 sink seam。它至少
   携带当前 provider service/operation/activation（或等价 typed owner）、同一 `traceId/errorId`、本地
   source、完整当前 service stack 与有限 safe cause kind；不得携带本地错误 payload/display、provider heap
   handle、`RuntimeValue`、`TypeAddr`或 generic JSON attrs。
2. 现有 `emit_native` 用户 surface 不承担该职责。新的 sink 是 runtime internal seam；在 F335/H 接入 host
   前允许默认丢弃，但 eval production 必须真实调用，测试须用 recording implementation证明调用。
3. ordinary、async unary、server stream及 `ContractOperation` test effect 的每个 provider export hop，都在
   provider heap销毁前恰好提交一份本 service restricted diagnostic。cancel/control、成功返回和
   `PackageCallable`不得提交。
4. A→B→C 的未捕获 fixed error：B export同一个 `OpaqueServiceError` bytes和同一 `traceId/errorId`，同时提交
   B自己的 local source/stack；不得复制 A 的 stack、重新生成 correlation或再次包装
   `std.service.InternalError`。
5. 新生成 public/platform/Internal error的 diagnostic correlation必须等于最终 fixed envelope；private、
   nonclosed与 encode failure的原始类型、值和显示字符串不得进入 diagnostic safe fields。
6. sink失败或缺省 sink不得改变 service error结果、分类、correlation或 exact encoded bytes；不得用
   message/code反推 fixed 类型。
7. 删除 ordinary 的 test-only平行记录 owner，或把它改为只观察同一 production typed sink；async/stream/
   test-effect不得各自发明第二个 record shape。

## 最小探针与验证 owner

必须覆盖：

- ordinary public或Internal：sink一次且发生在 heap drop 前，diagnostic correlation等于 fixed bytes；
- async unary和server stream各一次，cancel/success为零；
- imported fixed/Internal三跳：raw bytes不变、每跳各自 local stack、无 callee source；
- service test effect提交，Package effect不提交；
- sink故障不遮蔽原错误；
- private sentinel、source/path/function/stack不进入 fixed bytes；本地 stack只存在 typed diagnostic。

先列 selector，确保非零，再运行：

```bash
cargo test -p skiff-runtime-capability-context --lib -- --list
cargo test -p skiff-runtime-capability-context --lib --no-fail-fast
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic -- --list
cargo test -p skiff-runtime-eval --lib restricted_service_diagnostic --no-fail-fast
cargo test -p skiff-runtime-eval --lib service_error_consumer --no-fail-fast
cargo test -p skiff-runtime-eval --lib assembly_execution::async_stream_cancel --no-fail-fast
cargo test -p skiff-runtime-eval --lib service_error_channel_contract_operation --no-fail-fast
cargo check -p skiff-runtime-eval --lib
git diff --check
```

可新增一个统一 exact selector，避免重复运行现有大 fixture；不得运行完整 eval/workspace/root/stable/live。
`ServiceErrorEnvelope` bytes、F332 R0 API、sink value shape或 export timing变化会使本任务证据失效。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f334-restricted-diagnostic`
- branch：`codex/p5-f334-restricted-diagnostic`
- 新的一次性开发 Agent；5 分钟内先实现 typed value/sink 与 ordinary 调用，不能先跑完整测试或重做设计；
- 提交代码与
  `P5-F334-restricted-service-diagnostic-handoff-result.md`，返回 commit、自验收矩阵、exact-bytes/逐跳栈/
  lane/negative 证据和未决问题；
- 不 push，不承接 F335 或独立验收。
