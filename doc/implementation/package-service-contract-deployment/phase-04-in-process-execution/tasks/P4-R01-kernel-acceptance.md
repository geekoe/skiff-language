# P4-R01：Kernel Checkpoint Acceptance

## 角色与精确输入

高风险只读验收Agent。输入为：

- 权威设计`doc/architecture/package-service-contract-deployment.md` §2、§6、§7、§9、§10、§12、§14；
- `phase-plan.md`、P4-T01/T02/T03任务合同；
- T01–T03已合流的exact clean integration commit及开发自验收证据。

不得修改文件、创建commit或预设PASS。只运行必要聚焦抽查，不重复开发Agent的完整命令。

## 必验边界

1. execution image每build只链接一次，package direct与activation-relative service instruction无混淆，无legacy aggregate adapter。
2. ActivationContext按deployment/generation隔离，binding/config/state/callback mutable owner不按package build共享。
3. materializer直接消费canonical contract schema/value plan，普通graph detached，callback hook显式且recoverable fail closed。
4. callback capability字段、owner/generation/lifetime/error符合设计，无method table/native address/rebuild/fallback。
5. eval handoff显式传播owner，三个lane seam可独立写入，checkpoint fail closed不会触发旧outbound/router路径。
6. T04–T06的写入ownership确实不争中央文件；任何缺失共享API必须在R01阻塞，不能让lane各自发明。

## 输出

第一行`PASS`或`FAIL`。列出blocking issues、non-blocking follow-up、证据命令、动态测试缺口与残余风险。
PASS才解锁Wave 2；verdict锚定精确commit。
