# Phase 04：In-Process Execution Plane

状态：complete；P4-A01独立验收PASS，详细DAG、证据与残余风险见`phase-plan.md`和`phase-result.md`

## 输入

- 已解析 RuntimeAssembly、ActivationContext templates、service binding vectors 和 contract value plans。
- Phase 02 已将当前语言尚不支持的 error/stream/callback source lane 显式标成 unsupported；本阶段执行语义的
  正例从 canonical typed artifacts/RuntimeAssembly 生产边界开始，不在 runtime 中发明 authoring 语法。

## 完成态

- service call 切换 provider ActivationContext，并按 linkable value plan detached materialize 参数和返回值。
- ActivationContext 显式传播到 await、stream、callback 与 cancel；不依赖 thread-local current service。
- request-scope `any I`/native handle 只通过 callback capability crossing，owner/generation/lifetime fail closed。
- Ingress 与内部 service call 使用同一 contract/binding dispatcher。
- 所有 production service edge 都是 InProcessBoundary；remote selection/fallback 不可达。

## 预期波次

1. canonical assembly execution image 与 ActivationContext、binding ABI、materialization、capability table kernel
   合成一个共享检查点。
2. ordinary/error、async/stream/cancel、callback/native 三类 lane 并行。
3. ingress/internal dispatcher cutover、router remote relay retirement 与 checker engine 并行；合流后注册真实
   production subjects 并要求零违规，再进入 runtime/router gate、live smoke 与独立验收。

## 阶段验收

- service boundary 不共享 caller 引用 identity、alias 或原地 mutation。
- package direct call 仍保持 same-heap mutation，不被强制 linkable/recoverable。
- callback 返回 owner 后恢复 receiver context，cancel/close/owner exit 使 capability 稳定失效。
- 缺本地 provider 不经 router fallback。
