# P5-F171：Runtime Eval Package Schema Materialization Result

状态：Completed

## 直接父任务

- `P5-F171-runtime-eval-package-schema-materialization.md`

## 交付

- 普通service call与async unary/stream lane的`CanonicalServiceBoundaryPlan`不再接收或读取
  `ServiceContract.boundary_schema`；它只借用F171A固定在
  `RuntimeAssemblyServiceCallTarget`上的admitted shared Package records。
- 参数、返回值与typed error统一把同一
  `ServiceSchemaRecords = PackageSchemaTypeId -> Arc<PackageSchemaTypeRecord>`传给runtime boundary编译，
  缺record、owner/key/id错配和未闭合引用均在provider执行前的plan构造阶段fail closed。
- server-stream在建立provider task前用同一records预检operation和item plan；`BoundaryStreamSink`仅
  `Arc::clone`已经admit的schema map，并在emit item与internal callback item物化时继续消费该只读map。
  没有clone record payload、读取filesystem/Package index或从ServiceContract重建schema。
- cancellation、backpressure、stream lease、provider task teardown、heap detach与typed-error诊断包装路径
  未改变。
- direct tests迁移到Package-owned records，并补充：
  - 普通参数、返回与typed error同时引用Package named type；
  - 相同stable key跨Package owner保持隔离，owner错配在plan阶段拒绝；
  - named stream item使用admitted record成功，缺record拒绝；
  - 既有cooperative与NotCancellable测试继续拥有取消语义证据。

## 验证

通过：

```text
rg "ContractTypeId|ContractSchemaType|boundary_schema" \
  runtime/eval/src/assembly_execution/boundary_materialization.rs \
  runtime/eval/src/assembly_execution/boundary_materialization/tests.rs \
  runtime/eval/src/assembly_execution/ordinary.rs \
  runtime/eval/src/assembly_execution/async_stream_cancel.rs
0 matches

git diff --check
passed
```

已执行：

```text
cargo check --locked -p skiff-runtime-eval
cargo test --locked -p skiff-runtime-eval boundary_materialization --no-run
```

编译已越过F171修改的production文件；当前被独立F172 WebSocket断面
`websocket_contract_plan.rs`、`websocket_ingress.rs`和`websocket_response.rs`仍引用旧
`ContractTypeId`/`ContractSchemaType`/`boundary_schema`阻断。test build还同时报告既有
`ordinary/tests.rs`、`projection.rs`和`spawn_ops/canonical_tests.rs`的PackageArtifact fixture缺少新增schema
字段。按任务边界未修改这些WebSocket及通用fixture owner；F172及后续fixture合流后应在integration统一执行
完整`cargo test -p skiff-runtime-eval`。

## 下游断面

F172应让WebSocket plan/ingress/response消费target上的同一shared Package records。完成所有eval consumer后，
统一迁移剩余PackageArtifact fixtures并运行完整eval与workspace gate。
