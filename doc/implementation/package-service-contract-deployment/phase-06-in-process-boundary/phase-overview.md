# Phase 06：InProcessBoundary Execution

状态：outline-only；Phase 05 验收后再细化

## 输入

- 已解析的 RuntimeAssembly、ActivationContext templates和service binding vectors。
- ServiceContract value/callback/stream/error/cancel descriptors。

## 目标

- service call进入provider ActivationContext，按linkable value plan detached materialize参数与返回值。
- 显式传播 ActivationContext through await continuation、stream producer/consumer和callback dispatch；禁止
  thread-local“当前service”。
- request-scope `any I`/native handle只通过 callback capability crossing，落实owner、generation、lifetime和
  expired/unavailable error。
- Ingress和内部service call走同一contract/binding dispatcher；production remote selection/fallback不可达。

## 验收边界

- caller引用identity、alias和原地mutation不能穿过service boundary。
- callback切回capability owner，返回后恢复receiver context；cancel/stream close使capability按契约失效。
- package direct call仍保留same-heap mutation，不被强制linkable/recoverable。
- 当前所有production service edge都是InProcessBoundary，缺provider不经router。
- 旧remote relay实现与fixtures可暂留到Phase 07物理删除，但必须有结构gate证明production不可达。

## 细化前复查

复查 eval、boundary codec、transport、stream/cancel、recoverable、native capability和router relay；区分可复用
的transport-neutral contract与必须删除的remote执行路径。
