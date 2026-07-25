# P5-F324 Canonical service error channel core

状态：Completed。结果见
`P5-F324-canonical-service-error-channel-core-result.md`。

## 直接父节点

- current owner、真实跳点与R0完成矩阵：
  `P5-F319-service-error-channel-delta-audit-result.md`
- imported cause API：
  `P5-F321-imported-service-exception-cause-result.md`
- exact selected codec API：
  `P5-F322-selected-service-value-codec-result.md`
- representation carrier：
  `P5-F318-representation-wrap-eval-consumer-result.md`

整条引用链已追溯到唯一权威设计。本任务实现F319的R0c并冻结R1/R2/R3共同调用的唯一core API；不接任何lane。

## DAG位置与候选

- 节点：R0c，blocked-by R0a/R0b已完成；完成后解除R1 ordinary/ingress、R2 async/stream/cancel与R3
  service test effect并行consumer。
- 当前状态是implementation checkpoint；本节点高风险，合流后先运行R0 combined probe和独立验收，不能直接
  宣称A5 PASS。
- 证据基线：worktree创建时integration HEAD。`ServiceErrorEnvelope`、type index、imported cause、
  selected codec、caller package graph或platform registry变化会使本证据及后续R1–R4证据失效。

## Production写入范围

- 新建`runtime/eval/src/assembly_execution/service_error_channel.rs`
- `runtime/eval/src/assembly_execution/mod.rs`仅模块/API接线
- `runtime/eval/src/error.rs`仅canonical fixed service failure carrier/variant和安全accessor
- `runtime/eval/src/exceptions.rs`仅exact local identity/materialization复用helper
- `runtime/eval/src/program_execution.rs`仅provider-local stack scope/reset API

允许新模块co-located tests及上述文件inline tests。禁止修改ordinary、async/stream/cancel、ingress、
WebSocket ingress、test effect、capability-context、request/host/transport/router/telemetry、boundary/model/
linked-program/linker/loader/std、artifact/compiler及权威设计。

若现有低层API不足且必须改变父checkpoint公共形状，立即返回`TASK_NOT_EXECUTABLE`及最小缺口，不在eval复制
schema index、platform allowlist或codec。

## 共享API完成标准

提供一个唯一`CanonicalServiceErrorChannel`（具体Rust命名可调整）和R1–R3可调用的typed fixed carrier：

- export在provider heap仍存活时接收actual `RuntimeError`、heap、execution image、provider/caller build和
  correlation facts，返回strict `OpaqueServiceError`或明确的invalid-artifact/protocol failure；
- import接收同一fixed carrier、caller exact build graph、caller heap、call site、当前local stack及安全
  remote service/operation facts，返回caller-local`UserException`；
- `RuntimeError`中的fixed service variant只承载strict fixed error，不实现第二套分类、generic
  `WirePayload` flatten或message/code推断；
- 已imported且未catch/replaced的cause在export最先命中，逐字节返回原envelope。

## Export完成标准

### Public typed

- actual local declaration/named-union identity只通过`ServiceErrorTypeIndex.by_execution`查找；
- owner可以是provider Package或dependency Package，不能改写成service owner；
- generic/applied local error本阶段没有public row时转Internal；伪造arity/index不变量为InvalidArtifact；
- record/representation使用`Root`，named union使用index row的exact branch ordinal；
- schema closure来自linked Package code slot，不来自operation contract；
- `ServiceValuePlan::encode_binary_selected`成功才生成`PublicTypedError`；
- actual-value encode failure、private、non-nameable、nonclosed只生成一次Internal；
- index/record/owner/key/type-id损坏不能被Internal掩盖。

### Internal

- 固定脱敏message由core唯一常量拥有；不得包含原type、字段、display、encoder message、path或function；
- payload使用原cause的`traceId/errorId`，不生成第二个cause；
- exact local `std.service.InternalError`进入同一fixed Internal分支，不作为普通PublicTypedError，也不套第二层；
- imported Internal/export永远先走raw-byte forwarding。

### Platform

- 只有`PlatformBuiltinErrorIdentity`有限registry中的typed projection进入`PlatformError`；
- `std.resource.ResourceError`必须作为普通Package public error或Internal，绝不能加入platform allowlist；
- payload编码/解码按enum key选择同一个finite canonical validator/materializer；identity不能从payload
  code/message反推；
- InvalidArtifact、malformed imported envelope与schema冲突不得变成普通Internal。

## Import完成标准

### Public linked/unlinked

- strict decode后始终保留raw bytes；
- 按完整public identity查index，再用caller
  `implementation_package_build_id`和exact package-link edge选择唯一caller-local row；
- assembly中存在其它build/同package identity不等于caller已链接；无exact edge时合法public error保持
  imported `local_value=None`，所有local catch miss；
- 部分owner/key/id冲突、歧义build、branch ordinal/record/payload错误为Protocol，不能opaque fallback；
- linked record/representation/union按decode selection恢复caller-local exact carrier identity；
- linked imported exception同时保存local carrier和raw envelope，未捕获继续export原bytes。

### Internal/platform

- Internal通过caller exact `skiff.run/std` link及`std.service.InternalError` schema row构造
  `{message,traceId,errorId}`并附caller-local nominal identity；缺失/错配为InvalidArtifact/Protocol；
- Platform按envelope enum identity严格decode payload并附同一
  `PlatformBuiltinErrorIdentity.catch_identity()`；
- import创建新的caller-local source/stack，追加唯一安全
  `RemoteBoundary {service_id,operation_id,error_id}`；不导入callee site/path/function/diagnostic frame。

### Stack与correlation

- 新provider stack scope清空继承的caller local stack，但共享request trace和error-id sequence；
- local rethrow继续复用同一exception/source/stack；
- cross-service import总是新local exception stack；
- ingress尚未接线，本core不能伪造external caller exception。

## 最小探针

co-located core tests至少覆盖：

- B1/B2 record与dependency owner，representation `Root`，named union exact branch；
- B3 linked/unlinked与三跳raw-byte forward；
- B4/B5/B6 fixed Internal一次生成且无私有泄露；
- B7 owner/key/id/build/ordinal/payload mutation fail closed；
- B8 platform exact round-trip与B8a Resource不进入platform；
- B9 imported Internal raw forward且caller可exact catch；
- S2 local rethrow不变、remote import新stack；
- provider stack reset；
- no display/shape/code/message fallback。

## 验证owner

```bash
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel -- --list
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast
cargo check -p skiff-runtime-eval --lib
cargo fmt -p skiff-runtime-eval -- --check
git diff --check
```

selector必须非零。完整eval当前仍有F323记录的trace fixture及WebSocket设计blocker，不属于本节点；不得越界
修复，也不运行workspace/root/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f324-service-error-core`
- branch：`codex/p5-f324-service-error-core`
- 风险：最高；新的一次性Agent，5分钟内先创建core module/API skeleton并开始测试；
- 提交并返回export/import/public/Internal/platform/opaque/stack矩阵、API签名、focused evidence与未决缺口；
- 不push、不承接R1–R4或验收。
