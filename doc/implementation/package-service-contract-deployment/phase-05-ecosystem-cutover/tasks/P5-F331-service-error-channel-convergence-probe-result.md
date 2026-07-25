# P5-F331 Service error channel convergence probe结果

状态：`PASS`。Blocking issues：无。

本结果只接收R1–R3合流后的cheap combined integration probe及F319 R4 test-only convergence；
只解除A5 independent acceptance和W2-W正式开工，不代表A5、W2-W或Phase 5通过。

## Exact candidate与祖先

- integration candidate commit：
  `5040224ed4729bc8f5608d1c9b7b2cabe7cc9df3`
- integration candidate tree：
  `11c5c49ca3c25a60719d58a7cf4429bac3cf3120`
- worktree：
  `/Users/geek/workspace/skiff-p5-f331-error-convergence`
- branch：
  `codex/p5-f331-error-convergence`

祖先检查全部通过：

- R0 production candidate：
  `49d9ab300f331f7662abfe8e6a0345f93c97f816`
- F328 implementation：
  `6d743481dc7a762a4ac64d9ae5224f5bfc4ff2ef`
- F328 result：
  `f41c7d1bac454cd51372875a00a5ad036635a212`
- F329 implementation：
  `7710ce183fe0b0fbe353dcc736403d30c323308a`
- F329 result：
  `4e19d05dd8bec41d6900fa62f59b95570d769d82`
- F330 implementation：
  `f6fa4aa3110abfb3519385012dc7c8329a0840c4`
- F330 integration merge：
  `b7971d3e62095fe204b492cbf2e00c0fa8addab9`

F330 result文档本身已在candidate commit中。P5-F331没有production写入；唯一runtime接线变化是
`assembly_execution/mod.rs`中的`#[cfg(test)]` module声明，实际fixture为新建的
`assembly_execution/service_error_convergence.rs`。

## 新增exact convergence probe

任务给定的`service_error_channel_contract_operation` selector在candidate上原为零。没有回写F327–F330
co-located fixture；新增独立test-only模块后，该selector为3：

1. 在同一真实linked `AssemblyExecutionImage`、provider heap和caller heap上执行ordinary真实dispatcher、
   async-unary production lane入口和ingress真实dispatcher；ordinary/async/ingress得到逐字节相同的
   `OpaqueServiceError`，async再由同一R0 importer恢复caller-local carrier。随后同一bytes穿过typed
   stream import carrier和`OutboundResponse::FixedServiceFailure`，没有decode/re-encode。
2. 用真实unlinked caller接收public fixed error，确认middle hop为opaque且原bytes不变；再次export时
   correlation allocator不可达；把carrier交给下一份exact linked caller image后恢复local carrier，
   同时仍保存原bytes。
3. `ContractOperation` registry只交出typed fixed throw，caller heap不接收setup handle；
   `PackageCallable`不能进入service dispatch path并fail closed。

该probe只组合现有真实入口和冻结R0 API，不实现classifier、codec或platform registry。

## 合流接线

| 接线面 | 结论 | production与探针证据 |
| --- | --- | --- |
| ordinary与async unary | PASS | `ordinary::execute_service_call`和`async_stream_cancel::execute_provider_unary`都在provider heap存活时调用`CanonicalServiceErrorChannel::export_provider_failure`并返回`RuntimeError::FixedServiceFailure`；新增convergence probe以同一resolved target逐字节比较两条入口。 |
| central dispatcher与ingress | PASS | `dispatch_in_process_boundary`只对`InternalServiceCall`调用R0 import；`Ingress`直接返回fixed carrier。ordinary真实入口得到caller-local exception，ingress真实入口保持`FixedServiceFailure`且caller heap为空。 |
| stream producer与consumer | PASS | provider terminal在provider heap仍存活时调用同一R0 export，再构造`fixed_service_failure_with_import`；`program_stream::materialize_consumed_stream_error`只读取typed parts并调用同一R0 import。typed carrier selector、program-stream selector及新增跨lane carrier probe均保持原bytes。 |
| service test effect与Package effect | PASS | `EvalContext::materialize_service_test_throw`只调用同一R0 export/import；registry保留setup heap snapshot。`PackageCallable`仍只允许local payload并调用`materialize_local_test_throw`，fixed/service runtime failure均fail closed。 |
| legacy response | PASS | `OutboundResponse::FixedServiceFailure`原样转成`RuntimeError::FixedServiceFailure`；generic `OutboundResponse::Error`固定为Protocol，不读取code/message分类。 |
| classifier ownership | PASS | ordinary、async、ingress、program-stream、test-effect和service-dispatch lane没有`ServiceErrorEnvelope`/`PlatformBuiltinErrorIdentity`/Internal message分类分支；分类仍只有R0 channel。 |

## R4矩阵

| R4 | 结论 | 证据 |
| --- | --- | --- |
| B1 public exact | PASS | R0 record/representation/named-union；ordinary linked public exact catch；新增ordinary/async/ingress exact bytes convergence；linked service effect public throw。 |
| B2 dependency owner | PASS | R0 dependency record保留actual Package owner；ordinary dependency-owned `std.resource.ResourceError`保持public Package identity。 |
| B3 unlinked middle hop | PASS | 新增exact probe真实执行unlinked import→raw export→下一linked image恢复，bytes/traceId/errorId均不变。 |
| B4/B5/B6 private/nonclosed/encode failure | PASS | R0 selector证明三类只生成一次fixed Internal；ordinary private和linked service-effect private/nonclosed/encode matrix证明consumer不二次包装。 |
| B7 identity/payload mutation | PASS | R0 owner/key/type-id/payload/ordinal mutation与ordinary owner/key/type-id mutation均Protocol或InvalidArtifact；完全未知owner仍opaque。 |
| B8 platform | PASS | R0 finite platform payload/identity及ordinary File platform；linked service effect platform均调用同一channel。 |
| B8a Resource Package path | PASS | Resource不在platform registry；R0和ordinary均把`std.resource.ResourceError`作为普通Package public typed error。 |
| B9 imported Internal三跳 | PASS | R0 imported Internal原bytesforward；ordinary public/private三跳证明每个provider只export一次且两个import record bytes相同。 |
| S1/S2 stack | PASS | provider stack scope清空继承local frames；ordinary三跳为B/C/A分别创建local stack；local rethrow复用同一source/stack/correlation，remote import创建新local stack和安全`RemoteBoundary`。 |
| T2 service effect | PASS | linked selector覆盖public、Internal matrix、opaque raw forward和platform；exact selector证明`ContractOperation` typed target。 |
| T1 Package effect | PASS | registry selector证明Package typed throw只local deep-clone，fixed/service failure不wire，Package-shaped target不能进入service dispatch。 |
| lane真实入口 | PASS | ordinary public/Internal、async unary public、ingress public/private、stream typed terminal→program-stream import、service effect public/Internal均有真实入口证据。 |
| negative | PASS | cancel/control ordering、generic legacy Protocol、opaque catch miss、provider/setup heap隔离、stream cleanup及typed-vs-dynamic分支均通过。 |

## 隐私与反搜

反搜结论：

- `ServiceErrorEnvelope` wire字段只有public identity/encoded payload/correlation，或fixed Internal payload，
  或finite platform identity/payload/correlation；没有source、path、function或stack字段。
- ordinary private与ingress private probe明确拒绝
  `PrivateFault`、`provider-private-secret`、provider source和function进入fixed bytes；Internal message固定为
  `Internal service error`。
- production lane中`OpaqueServiceError::decode`、`canonical_json_bytes`、
  `encode_binary_selected`和`decode_binary_selected`均为零；这些只由R0/model/boundary owner使用。
  imported unhandled hop只读取`fixed_service_error()`/`encoded_bytes()`并clone typed carrier，不生成新
  correlation。
- `materialize_provider_error`在`runtime/eval`和`runtime/boundary`反搜为零。
- legacy mapper没有`error.message`→ProviderUnavailable；generic `ResponseError`直接Protocol。
- `deep_clone_runtime_value_carrier_between_heaps`在test-effect throw路径只位于
  `materialize_package_effect`→`materialize_local_test_throw`；service throw读取setup heap并调用R0。
- fixed stream production路径没有`downcast_ref::<Fixed...>`；`StreamProducerFailureRef::FixedService`与
  `Dynamic`是明确typed分支，只有local/general dynamic branch继续使用generic `WirePayload`。
- `PlatformBuiltinErrorIdentity`有限enum明确注释并实现Resource缺席；`BoundaryOperationContract`字段只有
  parameters、return、stream、cancellation、callbacks、maySuspend和effect guarantee，没有error set。

合流lane中没有按name/display/shape/message/code/operation contract推断错误类型。

## Selector与执行结果

先列selector并确认非零：

```text
assembly_execution::service_error_channel            13
service_error_consumer                                5
assembly_execution::async_stream_cancel              13
program_stream                                        4
service_error_channel_contract_operation              3
skiff-runtime-capability-context --lib               33

linked_service_effect                                 4
service_dispatch::tests                               5
test_effect_registry::tests                          14
ingress_hands_fixed_failure_up_without_importing...   1
```

实际执行：

```text
cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast
  PASS，13 passed / 0 failed
cargo test -p skiff-runtime-eval --lib service_error_consumer --no-fail-fast
  PASS，5 passed / 0 failed
cargo test -p skiff-runtime-eval --lib assembly_execution::async_stream_cancel --no-fail-fast
  PASS，13 passed / 0 failed
cargo test -p skiff-runtime-eval --lib program_stream --no-fail-fast
  PASS，4 passed / 0 failed
cargo test -p skiff-runtime-eval --lib service_error_channel_contract_operation --no-fail-fast
  PASS，3 passed / 0 failed
cargo test -p skiff-runtime-capability-context --lib --no-fail-fast
  PASS，33 passed / 0 failed

cargo test -p skiff-runtime-eval --lib linked_service_effect --no-fail-fast
  PASS，4 passed / 0 failed
cargo test -p skiff-runtime-eval --lib service_dispatch::tests --no-fail-fast
  PASS，5 passed / 0 failed
cargo test -p skiff-runtime-eval --lib test_effect_registry::tests --no-fail-fast
  PASS，14 passed / 0 failed
cargo test -p skiff-runtime-eval --lib \
  ingress_hands_fixed_failure_up_without_importing_an_external_caller --no-fail-fast
  PASS，1 passed / 0 failed

cargo check -p skiff-runtime-eval --lib
  PASS
rustfmt --edition 2021 --check \
  runtime/eval/src/assembly_execution/service_error_convergence.rs \
  runtime/eval/src/assembly_execution/mod.rs
  PASS
git diff --check
  PASS（无输出）
```

命令只报告既有compiler-source/runtime-linker warnings；没有新增warning或production失败。

没有运行完整eval、workspace/root gate、stable instance、live、chat smoke或完整WebSocket selector；
任务已知的两个generic WebSocket blocker未被重复运行或修改。未push，也未承接A5或W2-W。

## Verdict

Verdict：`PASS`。Blocking issues：无。

R0–R3已经在同一candidate上汇合到一个fixed carrier、一个central internal importer和一个R0
classifier/importer owner；R4 matrix与隐私反搜均通过。该结论只解除A5 independent acceptance和W2-W
正式开工，不得表述为A5、W2-W或Phase 5 PASS。
