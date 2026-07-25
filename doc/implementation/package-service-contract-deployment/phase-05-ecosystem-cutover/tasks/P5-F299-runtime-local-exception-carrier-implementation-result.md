# P5-F299 Runtime local value and exception carrier实现结果

状态：Implemented checkpoint with upstream gaps。

任务提交：`a4f489f6f579590d251ec615acc0ba85fe5f3302`。

集成提交：`12a1f7c231d0385cbacfbe4cb6a5a41245102c30`。

## 直接任务与权威链

- `P5-F299-runtime-local-exception-carrier.md`
- 任务继续引用F300、F297、F284与F280父链。

## 已完成

- `RuntimeValueCarrier`贯通slot、heap sidecar、object/array/map、assignment、record construct、
  local package/interface argument与return、stream item、native plan及test effect；
- actual applied nominal、三类named-union branch、representation owner与nested argument进入exact
  instantiated `CatchIdentity`；
- initial throw以actual carrier构造request-local `RequestException`，持有required instruction site、
  local call stack、trace/error correlation及local cause；
- exact catch消费Exception节点，不经wire encode/decode；catch miss传播同一Exception；
- rethrow保留同一cause、source、stack与correlation；
- boundary只做local value/plan adapter与local Exception fail closed，未实现service wire、
  `InternalError`转换或telemetry。

## 验证

- runtime model：PASS，80/80；
- runtime boundary：PASS，177/177；
- `git diff --check`：PASS；
- 本任务范围内旧`TypeIdentity`与local envelope路径反搜：零；
- eval代码在诊断用临时compatibility bridge下通过编译，bridge已删除且未提交。

## 上游/下游缺口

1. compiler lowering仍把primitive-backed representation直接构造擦除成裸payload表达式，因此
   `throw R("x")`到runtime时没有真实`R` carrier identity。runtime当前正确fail closed，不能从static
   type/shape猜回。需先审计并实现canonical representation construct IR/lowering handoff。
2. `runtime/capability-context`及native consumer仍导入已删除的
   `runtime/model::error::TypeIdentity`；需独立consumer迁移后才能运行标准eval owner入口。
3. F299开发分支还未包含后来合入的F301 compiler consumer；integration已包含F301，但F302的std
   WebSocket决策分支仍会遮挡部分compiler-backed eval入口。

上述节点合流后才能形成runtime local carrier预验收候选并进入`A5-runtime-channel`独立验收。

