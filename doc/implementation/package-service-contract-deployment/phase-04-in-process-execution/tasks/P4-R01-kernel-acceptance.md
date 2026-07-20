# P4-R01：Kernel Checkpoint Acceptance

## 角色与精确输入

高风险只读验收Agent。输入为：

- 权威设计`doc/architecture/package-service-contract-deployment.md` §2、§6、§7、§9、§10、§12、§14；
- `phase-plan.md`、P4-T01/T02/T03任务合同；
- T01–T03已合流的exact clean integration commit及开发自验收证据。

首次验收在`ef14a08` FAIL。复验还必须完整阅读P4-F02/F03/F04合同及三任务合流后的exact clean commit；原三项
blocking issue必须逐项给`RESOLVED/UNRESOLVED`，且任何新production/Cargo/fixture变化都按新候选重新验收。

不得修改文件、创建commit或预设PASS。只运行必要聚焦抽查，不重复开发Agent的完整命令。

## 必验边界

1. execution image每build只链接一次，package direct与activation-relative service instruction无混淆，无legacy aggregate adapter。
2. ActivationContext按deployment/generation隔离，binding/config/state/callback mutable owner不按package build共享。
3. materializer直接消费canonical contract schema/value plan，普通graph detached，callback hook显式且recoverable fail closed。
4. callback capability字段、owner/generation/lifetime/error符合设计，无method table/native address/rebuild/fallback；
   model/eval/binary/recoverable exhaustive delegate与DB/spawn/queue persistent拒绝owner完整。
5. eval handoff显式传播owner，opaque callback只进入冻结hook，三个lane seam可独立写入，checkpoint fail closed不会
   触发旧outbound/router路径。
6. 共享typed fixture真实经过deployment/resolver/load/link/admit且没有手写resolved target；T04–T06各有预声明
   lane测试文件。
7. T04–T06的写入ownership确实不争中央或fixture root；任何缺失共享API必须在R01阻塞，不能让lane各自发明。
8. assembly target真正提供interpreter/type/const/nested-call executable projection；动态fixture至少执行canonical
   executable并通过真实中央hook到达typed lane结果，不只断言地址/ready。
9. capability generation/owner drain释放payload且保留稳定expired tombstone；request/stream/cancel/owner exit与
   materialization失败rollback均exact-once，无active entry泄漏。
10. assembly与legacy linker共用native/interface/receiver call semantic validator；tampered签名/slot/ABI在admission前
    fail closed，无两套复制校验漂移。

## 输出

第一行`PASS`或`FAIL`。列出blocking issues、non-blocking follow-up、证据命令、动态测试缺口与残余风险。
PASS才解锁Wave 2；verdict锚定精确commit。
