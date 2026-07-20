# P4-R02：Execution Lanes Acceptance

## 角色与精确输入

高风险只读批次验收Agent。输入为权威设计§6–§8、§12、§14，`phase-plan.md`，P4-T04/T05/T06任务合同，
三个lane已合流的exact clean integration commit，以及R01与开发证据。不得修改或预设结论。

首次验收在`ee1609c` FAIL，第二次在`9809dee`因stream terminal/drop ownership环FAIL。复验还必须阅读
P4-F06/F07/F08/F09合同与合流diff，逐项确认canonical callback映射、
async typed error、callback stream item以及callback host合流断言已闭环；任何新production/Cargo/fixture变化都按
新candidate重新验收。

## 三个独立 verdict

1. **ORDINARY_ERROR**：package direct same-heap对照、provider context switch/receiver restore、parameter/return/error
   detached materialization、missing provider no-router。
2. **ASYNC_STREAM_CANCEL**：future/owned continuation显式owner、producer/consumer context、item materialization、
   backpressure/close/cancel exact-once cleanup。
3. **CALLBACK_NATIVE**：opaque capability owner/generation/lifetime、callback context restore、stable expiration errors、
   native explicit adapter与recoverable拒绝。

复验额外要求：

4. **CALLBACK_PROJECTION**：`ContractTypeId`与local interface ABI保持不同identity domain，只通过admitted typed mapping
   对齐operation name/ABI/slot/signature；不得字符串等同或按map/declaration order隐式`zip`。
5. **ASYNC_ERROR**：async unary复用T04 lane-neutral canonical error planner，typed error按schema/value plan detached，
   未声明/shape mismatch分类与sync一致。
6. **STREAM_CALLBACK_ITEM**：provider local interface在JSON wire前投影为opaque capability，内部stream carrier不做
   JSON round-trip；stream lease在projection前active，close/cancel/owner exit后稳定expired且exact-once清理。
7. **STREAM_TERMINAL_DROP**：buffer满时End/Error publication不阻塞producer或丢失顺序；未消费、外部边界拒绝、
   consumer异常退出及runtime/request drop均打破producer/runtime clone环，task/registry/lease/capability归零。

同时检查三个lane只消费R01 frozen hook，没有复制descriptor/materializer/context owner或争改中央dispatch；生产
`tokio::spawn` user-code路径均携带owned context，无current-service TLS或router fallback。

必须核对三个开发commit分别在自身exact state运行并通过：

```bash
cargo test -p skiff-runtime-host typed_execution_ordinary
cargo test -p skiff-runtime-host typed_execution_async_stream_cancel
cargo test -p skiff-runtime-host typed_execution_callback_native
```

三个过滤器必须实际执行测试（不能是0 tests），且都复用T03 fixture经过deployment/resolver/load/link/admit；R02可在
合流commit便宜复跑，但不能以T10完整gate替代各lane的最早证据。

## 输出

首行总体`PASS`或`FAIL`，并分别输出三个verdict。列blocking issues、non-blocking follow-up、证据命令、动态缺口和
残余风险。任一verdict FAIL均不解锁Wave 3；结论锚定exact commit。
