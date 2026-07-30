# P5-F329 Service error async, stream and cancel consumer result

状态：PASS。

实现提交：`7710ce183fe0b0fbe353dcc736403d30c323308a`。

## 结果

- async unary与server-stream provider terminal都在provider heap存活时调用冻结的R0 export。
- fixed stream terminal与dynamic local/general producer error成为明确typed分支；fixed分支不依赖
  `WirePayload` downcast或字符串code。
- terminal carrier保存raw `OpaqueServiceError`以及原caller build/executable/site/stack provenance；consumer
  直接调用R0 import，不decode/re-encode。
- consumer/request cancellation、ready provider error继续使用原biased ordering；cancel分支不export
  Internal/Platform。
- capability outbound新增明确fixed response variant；generic `ResponseError`与它分离。
- legacy service dispatch只透传typed fixed error；generic error固定收敛为Protocol，不读取message分类。
- B1/B3/B6/B8/B9、S1/S2分类复用F327唯一core；lane没有第二classifier。

为实现真正typed stream producer，任务范围扩展一处
`runtime/native/src/dispatch/file.rs`机械适配；它只把旧dynamic producer构造接入新typed wrapper，不改变
native错误分类或业务语义。

## 验证

- capability-context：33/33 PASS。
- async/stream/cancel：13/13 PASS。
- program-stream：4/4 PASS。
- service-dispatch：5/5 PASS。
- capability/native/eval library checks、`rustfmt --check`、`git diff --check`：PASS。
- lifecycle覆盖normal end、consumer/request cancel、cancel-vs-ready-error排序、task counter回落、stream
  cleanup exactly-once与outbound lease清理。

