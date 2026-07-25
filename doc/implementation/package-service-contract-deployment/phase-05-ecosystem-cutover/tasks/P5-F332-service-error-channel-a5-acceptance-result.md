# P5-F332 Service error channel A5 acceptance结果

状态：`PASS`。Blocking issues：无。

本结论只冻结A5/W2-R runtime fixed service-error channel，并允许W2-W消费已经冻结的fixed
carrier/API；不代表W2-W、generic WebSocket决定或Phase 5通过。

## Exact candidate与只读边界

- production candidate：
  `5040224ed4729bc8f5608d1c9b7b2cabe7cc9df3`
- candidate tree：
  `11c5c49ca3c25a60719d58a7cf4429bac3cf3120`
- R4 evidence：
  `2960cfd95ff0c91a233aad2279e6adc8cf0a2f5f`
- R4 merge：
  `586b0f78544f4936fe0fc3913a4291d62de8e424`
- 验收HEAD：
  `28e1039167f3bdc730b65511060b5308a61940c3`
- 验收HEAD tree：
  `2756510f2e54b3f9b423e87ce1e6c0b764e5b039`
- worktree：
  `/Users/geek/workspace/skiff-p5-f332-error-a5-acceptance`
- branch：
  `codex/p5-f332-error-a5-acceptance`

candidate、R4 evidence、R0 production candidate及R1/R2/R3 implementation均为验收HEAD祖先。

`git diff 5040224e..HEAD -- runtime`只有：

- `runtime/eval/src/assembly_execution/mod.rs`增加两行`#[cfg(test)]` module接线；
- 新建test-only
  `runtime/eval/src/assembly_execution/service_error_convergence.rs`。

`2960cfd9..586b0f78 -- runtime`和`586b0f78..HEAD -- runtime`均无diff。因此candidate之后没有
额外runtime production变化，满足任务停止条件。

验收只读production/tests；未修改production、fixture、设计或既有结果。唯一写入是本result。

## 独立验收矩阵

| 验收面 | 结论 | 独立production与测试证据 |
| --- | --- | --- |
| 唯一owner与开放错误通道 | PASS | `CanonicalServiceErrorChannel::{export_provider_failure,import_caller_failure}`仍是唯一分类、selected codec、caller graph选择、Internal/platform materialization与逐跳stack owner；`BoundaryOperationContract`只有parameters、return、stream、cancellation、callbacks、maySuspend和effect guarantee，没有throw set。operation/service事实只用于验证与RemoteBoundary provenance，不参与错误类型分类。 |
| ordinary、async unary与central dispatcher | PASS | ordinary和async unary都创建fresh provider heap及provider-local stack scope，并在heap仍存活时只调用冻结R0 export。central dispatcher只在`InternalServiceCall` origin调用冻结R0 import；同一真实linked image上的ordinary/async fixed bytes一致。 |
| server stream与cancel | PASS | provider stream task在`producer.provider_heap`仍存活时调用同一export，再构造typed `FixedServiceStreamFailure`；consumer先按`fixed_service_failure_parts`typed分支调用同一import，只有local/general dynamic branch进入generic materializer。consumer/request cancel在biased control select中先于ready provider error，未进入export。 |
| ingress | PASS | HTTP与WebSocket ingress都进入同一resolved dispatcher并使用`Ingress` origin；failure只上交`RuntimeError::FixedServiceFailure`，不创建外部caller exception。合流探针确认ingress bytes与ordinary/async一致且ingress heap为空。 |
| service test effect与Package effect | PASS | `ContractOperation`保留setup heap/build snapshot，由`EvalContext::materialize_service_test_throw`调用同一export/import；caller heap不接收setup handle或setup-local `TypeAddr`。`PackageCallable`仍只走`materialize_local_test_throw`的request-local deep clone；fixed/service runtime failure及Package-shaped service dispatch均fail closed。 |
| B1/B2 exact public与owner | PASS | record、representation及named-union selected branch按exact execution identity/index/schema plan处理；dependency错误保留实际Package owner。公开`std.resource.ResourceError`走普通Package public typed path，不进入platform registry。 |
| B3 unlinked middle hop | PASS | 真实unlinked caller得到`local_value=None`且保留原bytes；再次export不调用correlation allocator，下一份exact linked caller image恢复caller-local carrier，同时继续保留同一raw bytes。 |
| B4/B5/B6 Internal | PASS | private、non-nameable/nonclosed generic及actual-value encode failure只产生一次fixed Internal，沿用原trace/error id；fixed bytes不含原type、字段、payload或encoder诊断。InvalidArtifact/Protocol不被Internal掩盖。 |
| B7 fail closed | PASS | 已知admitted Package的owner/key/type-id mutation、payload/branch ordinal错配及非canonical payload拒绝为Protocol/InvalidArtifact；完全未知owner仍可作为opaque转发，不按shape/name修复。 |
| B8/B8a platform与Resource | PASS | platform只从typed finite `PlatformBuiltinErrorIdentity`进入canonical payload codec；不从message/code/shape推断。Resource明确不在finite registry，并以Package public typed error通过真实ordinary入口。 |
| B9 Internal三跳 | PASS | private C错误第一次越界成为Internal；B收到可catch名义值并在未处理时直接export imported fixed cause；A/B两次import记录的bytes、traceId/errorId相同，各自建立新的本地stack。 |
| S1/S2 stack与隐私 | PASS | provider scope清空继承local frames而共享request trace/error sequence；remote import只接受以exact call site结束的纯local stack，再追加只含serviceId/operationId/errorId的RemoteBoundary。callee source/path/function/private字段不进入fixed bytes；same-request local rethrow复用原cause/source/stack/correlation。 |
| `std.service.InternalError` | PASS | inbound Internal严格要求exact caller-linked std三字段string record，materialize为caller heap中的普通名义值，exact catch命中；它同时保留raw fixed envelope，未捕获再次出界不会重包装。 |
| typed legacy seam | PASS | `OutboundResponse::FixedServiceFailure`与generic `ResponseError`是独立分支；fixed unary/stream保持原bytes。generic error无论伪造何种code/message都固定收敛为Protocol，不再推断ProviderUnavailable或typed service error。 |

## Production调用链与反搜

独立读取了以下真实生产链：

- `runtime/model/src/service_error.rs`
- `runtime/linked-program/src/assembly_execution/service_error_index.rs`
- `runtime/eval/src/assembly_execution/service_error_channel.rs`
- `runtime/eval/src/assembly_execution/{mod,ordinary,async_stream_cancel,ingress,websocket_ingress}.rs`
- `runtime/eval/src/{program_execution,program_stream,eval_context,test_effect_registry,service_dispatch}.rs`
- `runtime/capability-context/src/{stream,response,outbound_response}.rs`
- `artifact-model/src/boundary/operation.rs`

反搜结论：

1. `materialize_provider_error`在当前runtime/boundary production为零；旧provider error原样旁路未恢复。
2. ordinary、async、ingress、program-stream、service-effect和legacy response consumer production没有
   `ServiceErrorEnvelope`分类、canonical JSON codec、selected binary codec或Internal message副本；这些仍只在
   model/boundary/R0 channel owner中实现。consumer文件中的相应命中均位于test-only代码。
3. production lane没有按name/display/shape/message/code/operation contract推断service error identity。
   `PlatformBuiltinErrorIdentity::from_symbol`仍服务于既有local platform catch/projection，不被lane用于
   service response分类。
4. service effect路径不调用`deep_clone_runtime_value_carrier_between_heaps`；该helper在test-effect throw中只
   保留Package-local语义。program-stream中的deep clone仍属于local stream/arg materialization，不承载
   fixed service failure。
5. fixed stream branch通过`StreamProducerFailureRef::FixedService`直接读取typed carrier；dynamic
   `WirePayload`与local `RequestHeapOwnedStreamError`仍是独立general stream分支。fixed path不依赖
   `downcast_ref`、payload code或message。
6. `RequestException`的imported cause同时保存raw `OpaqueServiceError`与可选caller-local carrier；export在
   任何重新分类前优先返回fixed cause，保证opaque/public/Internal/platform未处理转发不重编码。

## 独立selector与执行结果

先列出上层selector并确认非零：

```text
service_error_channel_contract_operation    3
assembly_execution::async_stream_cancel    13
linked_service_effect                       4
service_error_consumer                      5
```

实际执行前又逐个列出所用exact selector；每个exact selector为1，`provider_stream_`为4。未机械重跑
F331全部命令。实际结果：

```text
ordinary/async/ingress real-lane convergence                 1/1 PASS
B3 unlinked middle hop                                       1/1 PASS
B9 ordinary public/Internal three-hop                        1/1 PASS
ordinary provider-heap-drop/caller-local materialization     1/1 PASS
known public owner/key/type-id mutation                      1/1 PASS
provider stream normal/consumer cancel/request cancel/order  4/4 PASS
provider stream task counter exact lifetime                  1/1 PASS
typed fixed stream drain without re-encode                   1/1 PASS
linked service-effect Internal matrix                        1/1 PASS
exact service-vs-Package effect target                       1/1 PASS
generic response.error Protocol negative                     1/1 PASS
ordinary exact public/Internal catch and opaque miss         1/1 PASS
ordinary representation/private/platform/Resource            1/1 PASS
dependency/representation/named-union exact selection        1/1 PASS
same-request local rethrow                                   1/1 PASS
Package typed throw remains local                            1/1 PASS
service effect setup heap/TypeAddr isolation                 1/1 PASS

合计                                                          20/20 PASS
```

另执行：

```text
git diff --check
  PASS（无输出）
```

首次构建/list只报告既有compiler-source与runtime-linker warnings；所选selector均退出0。

没有运行完整eval、workspace/root gate、stable instance、live、chat smoke或完整WebSocket selector；两个
既知generic WebSocket失败不属于A5，未重复运行。

## 结构判断

结构结论：`PASS`，无blocking第二owner或明显语义复制。

- R1–R3没有修改R0的
  `service_error_channel.rs`、其tests、selected codec、model envelope或linked index；R0唯一owner未继续
  膨胀。
- R1/R2/R3 production增量是lane adapter、typed carrier、control/lifetime与test-effect target分流；
  classifier、schema/index选择、platform codec、Internal生成和stack构造均未复制到lane。
- R4直接复用`ServiceErrorConsumerFixture`，没有再建一份classifier或linked-image owner。

Non-blocking可读性债务：

1. `ordinary/tests/service_error_consumer.rs`约1877行，
   `ordinary/tests/source_inline_effect_e2e.rs`约1494行；两者分别负责手工linked-image consumer fixture与
   source/compiler-linked effect fixture，当前职责不同且没有重复分类语义，但若继续加case，应先抽取通用
   Package/assembly fixture builder并按场景拆文件。
2. `async_stream_cancel.rs`约1334行、`program_stream.rs`约1447行、`eval_context.rs`约2084行、
   `test_effect_registry.rs`约1107行；本阶段增量仍落在各自既有lane/registry职责内，不构成blocking。
   后续继续扩展前宜把inline tests和typed carrier协作代码拆为相邻子模块，避免物理可读性继续下降。

以上是维护性改进，不影响当前唯一owner、依赖方向或A5语义。

## Blocking、non-blocking与残余风险

Blocking issues：无。

Non-blocking：

- 上述大型fixture与长lane模块存在物理可读性债务，但当前没有第二classifier、重复codec、重复index或
  lane-owned Internal/platform规则。

残余风险：

1. W2-W尚未接入request/transport/host/router/telemetry及外部`response.error` v2。A5只保证ingress上交
   fixed Rust carrier；外部frame、host producer/consumer、跨runtime transport和外部脱敏仍须由W2-W验收。
2. outbound typed seam在缺少in-process import facts时有意保留`import=None`；W2-W必须提供exact远端caller/
   transport事实，不能从generic response、code、message或target字符串补推。
3. host-boundary test effect是W2-W的T2另一半；本结论只接收service `ContractOperation`与Package-local T1。
4. generic WebSocket schema决定及两个既知失败不属于A5；本次PASS不能掩盖或替代该决定。
5. 本次按任务禁止未运行完整eval/workspace/root/stable/live；阶段级昂贵gate仍由其指定owner负责。
6. 后续若修改`ServiceErrorEnvelope`、`RequestExceptionCause`、selected codec、type-index/caller graph选择、
   channel export/import、stream fixed carrier、central dispatcher或test-effect target分流，本A5证据应按
   影响面失效并重验。

## Verdict

Verdict：`PASS`。A5/W2-R runtime channel冻结，W2-W可以消费fixed carrier/API；不得将本结果表述为
W2-W或Phase 5 PASS。
