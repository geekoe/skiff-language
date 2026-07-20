# P4-T04：Ordinary / Error In-Process Execution

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.5、§6、§7、§8、§9、§12、§14。
- 风险/验收组：高风险ordinary/error lane；T04–T06合流后由R02分别验收。
- 当前成熟度：R01已验收shared kernel；完成后是ordinary/error lane checkpoint，不是稳定候选。
- 有效证据：本任务clean commit叠加调度时exact R01 checkpoint。ordinary hook、call target、materializer、
  canonical fixture或测试变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：R01 PASS；与T05/T06并行。
- 解锁：R02。
- branch：`codex/p4-t04-ordinary-error`。
- worktree：`/Users/geek/workspace/skiff-p4-t04-ordinary`。
- 五分钟内真实edit；不得修改T03中央wiring或T05/T06模块。共享hook不足时立即报告，不自行扩ABI。

## 写入范围

独占T03预留的ordinary/error lane模块、canonical package/service executable invocation实现，以及
`runtime/host/src/loader/assembly_admission/tests/execution/ordinary.rs`；只在
任务合同明确预留的旧`service_dispatch`删除面删除被替代production edge。不得修改stream/callback、host/router、
compiler或shared kernel public API。

## 完成态

1. canonical `PackageCallable`进入exact target并复用同一`RequestHeap`和current ActivationContext；alias/identity/
   caller-visible mutation保持。
2. canonical `ServiceCall`按current activation + caller package build + slot解析唯一local provider和operation；
   切provider context后才调用provider target，返回后恢复receiver。
3.每个parameter、return和typed error按descriptor中的contract type/value plan双向detached materialize；参数数目、
  operation mode、schema/plan mismatch在provider前失败。
4. provider mutation/return alias/throw payload均不能与caller graph共享identity；typed throw在caller context中可捕获，
   provider/runtime错误不被伪装成typed business error。
5. missing binding/provider/operation/protocol不调用router/outbound，也不按display/legacy symbol重试。
6.旧`ServiceDependencySymbol -> OutboundServiceDispatch`不再是canonical eval call分支；不保留dual path。

## 最早探针与唯一验证 ownership

- 同一mutable object分别走package direct与service boundary：前者same handle并可mutation，后者不同handle且隔离。
- provider在成功、typed throw、runtime fail三种退出后，下一条caller指令均观察caller context。
- fake router/outbound hook设为panic，所有正负例保持零调用。
- host ordinary lane测试必须复用T03 typed full-chain fixture，经projection/resolver/load/link/admit后执行，不手写target。

```bash
cargo test -p skiff-runtime-eval ordinary_in_process
cargo test -p skiff-runtime-eval service_error_boundary
cargo test -p skiff-runtime-eval package_direct_same_heap
git diff --check
```

不得运行完整runtime gate。

## 回报

提交一个commit，回报call/context转换表、error分类、alias对照、legacy反向搜索、命令与自验收矩阵。
