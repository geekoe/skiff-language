# P5-F327 Service error core independent acceptance结果

状态：`PASS`。Blocking issues：无。

本结论只验收并冻结R0 canonical service-error core API，解除R1 ordinary/ingress、R2
async/stream/cancel和R3 service test-effect consumer；不代表A5、W2-W或Phase 5通过。

## Exact candidate与只读边界

- production candidate commit：
  `49d9ab300f331f7662abfe8e6a0345f93c97f816`
- production candidate tree：
  `ec596389abb5583fcfffc198205a657df5d4f616`
- 验收HEAD：
  `dc9cfafeaafbce34003ee26d8b0a03e3ae3becbd`
- 验收HEAD tree：
  `a69ced856591b1be5539198899c412843a71d7c6`

F321、F322、F324及production candidate均为验收HEAD祖先。
`git diff --name-status 49d9ab30..dc9cfafe -- runtime/model runtime/boundary runtime/eval`
无输出；candidate之后这三个production域没有diff，满足停止条件。

验收未修改production、fixture、设计或既有证据；唯一写入是本result。

## 独立验收矩阵

| 验收面 | 结论 | 独立production证据 |
| --- | --- | --- |
| Canonical ownership与依赖 | PASS | fixed envelope、finite platform identity和imported cause唯一实现在`runtime/model/src/service_error.rs`；selected codec唯一实现在`runtime/boundary/src/service_value_plan.rs`；type index唯一DTO owner在`runtime/linked-program/src/service_error_index.rs`，linker只负责admission构建；eval orchestrator唯一实现在`runtime/eval/src/assembly_execution/service_error_channel.rs`。在本channel依赖链上，eval以normal dependency消费model、boundary和linked-program，linker仍仅为dev-dependency；聚焦eval构建通过，没有依赖cycle。 |
| 父checkpoint真实接入 | PASS | `assembly_execution/mod.rs`无条件编译production core；core直接读取`RequestException::fixed_service_error`、构造`RequestException::imported`，并直接调用`encode_binary_selected`/`decode_binary_selected`及`AssemblyExecutionImage::service_error_types`。这些调用不在tests中重写；R0 API自身尚无module外lane caller，符合R1–R3未接线的边界。 |
| Export分类 | PASS | local throw保留actual carrier/catch identity，schema只在第一次出界检查。public record/representation/named union由exact execution key、provider exact Package graph、canonical schema record和selected codec共同决定；dependency error保留其实际Package owner。private、non-nameable、nonclosed和actual-value encode failure只生成固定Internal；imported fixed cause在任何重新分类前直接返回原bytes。InvalidArtifact及index/record/context不变量错误不被Internal吞掉。 |
| Import与catch | PASS | inbound先持有strict decoded envelope和原始bytes。public identity只按完整owner/key/type-id查表，再按caller build的exact Package edge选build；无exact edge保持`local_value=None`，不会从assembly其它build或同package id猜materialization。linked value从decoded root/ordinal选择exact local declaration/branch，恢复caller-local carrier，因而exact catch命中；opaque catch miss。 |
| Mutation/fail-closed | PASS | 已知Package的key/owner/type-id mutation、未知caller build、public payload trailing byte、named-union ordinal越界分别收敛为Protocol或InvalidArtifact；完全未知owner保持合法opaque。malformed correlation、platform identity/payload错配和非canonical platform bytes均拒绝，不退回Internal。boundary selected probe还覆盖same-shape branch ordinal、wrong selection、payload mismatch与trailing bytes。 |
| Stack与隐私 | PASS | local `throw`由`RequestException::local`要求exact catch identity、非空本地stack和correlation；local rethrow clone同一exception。provider scope API只清空继承的local stack，保留request trace/error sequence。remote import要求caller stack只含local frame且末帧为exact call site，然后只追加`serviceId/operationId/errorId`的`RemoteBoundary`。fixed envelope没有source、stack、path、function或display字段；Internal只含固定message与correlation。 |
| 三跳与Internal | PASS | linked inbound同时保留caller-local value和原始fixed bytes；再次export优先读取fixed cause。public三跳及Internal再次出界保持bytes、traceId和errorId；每次import各自创建本service local stack。exact local`std.service.InternalError`忽略用户选择的message/private字段并生成单层fixed Internal；inbound Internal要求caller exact std record和精确三字段string plan，恢复为普通可catch名义值。 |
| Platform与Resource | PASS | core的platform payload codec只接受`PlatformBuiltinErrorIdentity` enum作为选择输入，并按该identity严格验证字段和canonical bytes；不从payload、message或code反推identity。finite registry没有`std.resource.ResourceError`，`RuntimeError::ResourceError`也不产生platform catch projection；公开的std ResourceError走普通Package public typed path。 |

production core内没有operation-specific error set，也没有按display/name/shape/static throw/message/code或
operation contract猜错误类型。`operation_id`只作为validated provenance和安全remote frame事实，不参与分类。

## 结构判断

`runtime/eval/src/assembly_execution/service_error_channel.rs`为1157行，co-located tests为1435行。
行数本身不构成结论；本次按职责逐段检查：

- 对外生产surface只有两个orchestrator方法及两个typed context；
- local classifier、public/Internal/platform import都只从这两个方法进入；
- graph/index代码只校验并消费linked-program的canonical index，不构建第二份index；
- selected codec仍由boundary拥有，eval没有复制binary codec；
- stack DTO与provider scope仍分别由model和program execution拥有；
- platform payload codec是channel内部唯一enum-keyed codec，没有lane级副本；
- tests从第931行起集中使用一个assembly/package fixture builder，没有为每个case复制classifier。

因此这些是单一service-error channel owner下的私有协作职责，不是已经独立且相互混杂的多个owner；
R1–R3只需要调用冻结的export/import API，不会被迫复制graph、codec、classifier或stack分支。
结构结论为PASS。

Non-blocking follow-up：若R1–R3接线使文件继续增长，可把当前私有的graph/index validation和platform
payload codec拆成`service_error_channel`子模块，并把tests的assembly fixture移到独立fixture文件。
这是物理可读性改进，不改变owner、API或依赖方向，当前不阻塞R0。

## 独立探针

先列出selector并确认非零：

```text
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel -- --list
  13 tests, 0 benchmarks

cargo test -p skiff-runtime-model --lib imported -- --list
  3 tests, 0 benchmarks

cargo test -p skiff-runtime-boundary --lib selected -- --list
  4 tests, 0 benchmarks
```

实际执行：

```text
cargo test -p skiff-runtime-model --lib imported --no-fail-fast
  PASS，3 passed / 0 failed / 81 filtered out

cargo test -p skiff-runtime-boundary --lib selected --no-fail-fast
  PASS，4 passed / 0 failed / 177 filtered out

cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast
  PASS，13 passed / 0 failed / 155 filtered out

git diff --check
  PASS（无输出）
```

其中独立重点抽查了两组mutation：

1. 已知owner的key/type-id、caller build、public payload和union ordinal mutation必须Protocol/
   InvalidArtifact，而完全未知owner仍opaque；
2. malformed correlation、identity与payload不匹配的platform envelope、非固定Internal message必须拒绝，
   不能sanitize成Internal。

eval命令报告既有compiler-source、runtime-linker warning及test-only `unused_mut` warning；selector退出为0，
没有来自本验收写入的production变化。

没有运行完整eval、workspace/root gate、stable instance、live、chat smoke或R1–R4 consumer测试；这些超出
本任务只读R0边界，且R1–R3尚未接线。未push。

## Blocking、残余风险与Verdict

Blocking issues：无。

残余风险：

1. ordinary/ingress、async/stream/cancel及service test effect尚未成为R0 API的production caller；真实provider
   heap drop时序、stream carrier、cancellation control分流和test setup heap隔离仍分别由R1、R2、R3验收；
2. response.error、host/transport/router/telemetry外部frame与脱敏属于W2-W，本结果没有接收；
3. 后续若修改`ServiceErrorEnvelope`、`RequestExceptionCause`、selected codec、type-index lookup、
   exact caller graph选择、platform registry或export/import API，必须使本R0验收及其下游证据失效并重验。

Verdict：`PASS`。R0 canonical service-error core可以冻结并解除R1/R2/R3；不得把本结果表述为A5或
Phase 5 PASS。
