# P5-F306 Representation constructor carrier handoff audit

状态：Completed。结果见
`P5-F306-representation-constructor-carrier-audit-result.md`。

## 直接父节点与权威链

- runtime local carrier结果：
  `P5-F299-runtime-local-exception-carrier-implementation-result.md`
- compiler producer结果：
  `P5-F296-applied-nominal-compiler-consumer-result.md`
- strict shared model验收：
  `P5-F295-applied-nominal-model-acceptance-result.md`

父链继续引用F293与唯一权威type/error设计。

## 角色与边界

这是只读cross-layer handoff审计。已知primitive-backed representation constructor在lowering中被擦成
裸payload，导致runtime不能为`throw R("x")`建立actual nominal carrier identity。不得修改/提交文件，
不得运行测试，不操作stable/live。

## 必须回答

1. 从source representation constructor validation到lowering、File IR、linked IR、type plan与eval逐跳
   追踪actual value和nominal type；定位首次身份损失及所有被遮挡下游。
2. 检查现有File IR/linked表达式是否已有语义正确、无保留字段约定的typed wrap/construct表示可复用；
   不得建议借用record field、display string、static throw type或shape恢复。
3. 若必须新增/改变shared expression DTO，给出唯一最小canonical shape及理由，并明确：
   - record construct与representation wrap的区别；
   - payload仍保持原runtime值，仅carrier增加exact instantiated identity；
   - generic representation、nested representation与named-union concrete branch如何处理；
   - source site/throw site是否受影响。
4. 判断File IR schema/format/identity generation是否必须升级；列出artifact-model/identity、compiler、
   linked-program/linker、eval各自最小owner与串行checkpoint。
5. 搜索除direct constructor外的representation producer（native return、boundary decode、call return、
   field/container projection、test effect）是否已由F299覆盖，避免重复owner。
6. 给出最小正负测试和combined probe；若shared wire/identity选择超出既有设计而需要用户决定，整理最小
   选项，不自行决定。

只允许`rg`、文件读取和git只读检查。返回精确`file:line`、推荐DAG与是否需要用户决策；不承接实现。
