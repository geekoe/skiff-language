# P1-A01：Phase 01 独立只读验收

## 角色

验收Agent不得参与T01–T08开发，不得修改文件。它使用原始用户目标、canonical架构文档、总体实现计划、
Phase 01计划和T01–T08全部任务文件，对当前integration commit的完整production路径判定PASS/FAIL。

## 必查范围

1. 四对象目标没有被新的共同aggregate或generic DTO破坏。
2. identity/canonical JSON/nominal derivation/package preimage只有文档指定owner，跨语言consumer只是CLI或
   typed adapter。
3. effect Unknown显式、完整透传且fail-closed；没有empty/placeholder被当成无effect。
4. type closure kernel真正被boundary/recoverable/spawn生产路径消费，而不是只新增未使用抽象。
5. PackageUnit production/test只有一个builder；identity字段矩阵与mutation tests一致。
6. runtime/router真实load path验证unit/service assembly内容，不只验证fixture或diff。
7. 临时Publication/Service对象只在ledger允许范围，未新增规则owner，删除阶段明确。
8. T08证据对应当前commit；按需要运行廉价聚焦抽查，不机械重跑全部昂贵gate。

## 输出格式

第一行必须是`PASS`或`FAIL`。

- `FAIL`：逐项给出blocking issue、任务文档条款、production代码证据、影响和建议修复方向。
- `PASS`：列出核查过的关键production证据、使用的命令、未覆盖的动态风险。
- 可列non-blocking follow-up，但本阶段直接放大的重复、隐式契约、未删除双路径或缺少验收证据不能降级。

测试通过、diff很小或开发Agent自验收均不能替代独立判断。
