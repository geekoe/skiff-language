# P5-F03C：Runtime Integration Repair

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、6、8、9、10条，§5、§6.2、§7、§12及§14。
Runtime必须只加载完整同一assembly，按pinned deployment/ActivationContext执行service call，跨generation保留旧连接owner，
且admission、health、drain与atomic reload可观测。

## 输入与owner

- 依赖：P5-R02A PASS的exact F03A seam、P5-R11 PASS的committed bootstrap、P5-R13 PASS的canonical unary、
  P5-R24 PASS的typed unified WS owner checkpoint及P5-F23E shared generation lifecycle wire。与F03B并行，合流后
  先解锁最终R05，再共同解锁I02。
- DAG节点F03C；风险高，验收分组为Runtime production consumer。进入状态为F23E exact integration checkpoint，派发时给
  commit/tree/Cargo.lock。完成后仍只是Implementation Checkpoint，不能单独解除R05/I02。
- branch：`codex/p5-f03c-runtime-integration-repair`。
- worktree：`/Users/geek/workspace/skiff-p5-f03c-runtime-repair`。
- 独占`runtime/driver/**`、`runtime/host/**`及必要runtime request/transport consumer tests；不得改F03A
  /F05 shared wire与WS authoring规则、Router/compiler/test-runner。F23E transport lifecycle schema/codec/golden为冻结输入，
  只允许consumer import，不得改其字段或接受集合。只提交task branch。
- D09已把legacy Host package-test consumer/template cache与两个activation codec旧调用拆给F08/R08前置关闭；
  F03C不得恢复这些seam。其余startup/admission/request/drain/typed WS职责不变。

## 完成态

F10/R11已提前完成每次连接前的exact durable committed recovery、load/link/admit、active+committed publication与
capabilities/register bootstrap。本任务消费该唯一lifecycle/admission primitive，不回建第二startup/reconnect owner。

F13/R13提前接通strict canonical HTTP unary decode与active-route trust bridge；本任务消费该lane，不恢复legacy
RequestStart mapper或current-pointer-only lookup，R05后仍完成WS/serverStream、old-generation pin/drain与其余trust边界。

1. production config明确一个environment与一个canonical artifact root。driver不再以
   `services: Vec::new()`构造旧语义；RuntimeHost启动直接读取T01 committed state，exact resolve/load/link/admit
   generation（含empty generation 0），完成后才连接并发送capabilities + committed register。missing/tampered/
   partial state fail closed；restart收敛到同一tuple。
2. binary `assembly.activation`接入现有Runtime endpoint；prepare只stage，commit exact tuple才publish/register，
   abort清理。所有load/link/admit使用一个staged-admission primitive。删除/限制公开direct
   `admit_runtime_assembly` active-pointer path，任何production caller都不能绕过prepare/commit。
3. canonical nested assembly request routing在Rust trust boundary精确校验identity/generation/operation/ingress，
   不接受伪build/service fields。Host按generation找到已admit context；request期间artifact I/O为零。
4. active generation registry保留draining context直到所有unary/stream/WS pins释放。connect成功按F23E完整tuple隐式
   acquire，release/session disconnect幂等释放；WS receive使用connection pin的route/context，不按当前active重新lookup。
   commit B后旧A stream/WS继续A，新请求只进B，drain后删除退休generation且request path artifact I/O为零。
5. runtime capabilities、actor/spawn、health与package-test专用路径保持工作；assembly注册与replica id分离。

`extra-review`约束：把571行provisioning中的wire dispatch、load/stage、recovery、state mutation拆成可测试职责；
消除与`AssemblyAdmissionController::admit`重复workflow，而不是只重命名第二owner。

## 验证

```bash
cargo test --locked -p skiff-runtime-host --test active_runtime_assembly -- --test-threads=1
cargo test --locked -p skiff-runtime-transport --test assembly_replica_registration -- --test-threads=1
cargo test --locked -p skiff-runtime-host host::router_session::tests:: -- --test-threads=1
cargo test --locked -p runtime config::tests:: -- --test-threads=1
node scripts/check-runtime-crate-dag.mjs
git diff --check
```

新增consumer direct tests必须用新模块exact filter运行，所有filter测试数非零。聚焦探针覆盖cold start/register、binary
lifecycle、schema negative、direct-admit反搜、stream/WS old-generation pin、F23E acquire/release/session disconnect、
retired generation回收及artifact-I/O=0。提交一个clean commit及证据矩阵；禁止I02、R05 real transcript、live/chat/full/
I16/Host/stable，不merge/push。Runtime admission/host/F23E wire或Router activation schema变化会使证据失效。
