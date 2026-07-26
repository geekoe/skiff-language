# P5-F382 Interface suspension projection audit

状态：Ready（只读）。

## 直接父节点

- `P5-F380-relay-interface-receiver-and-gateway-completion-blocker.md`

本节点只审计public instance interface与实现的`maySuspend` canonical owner，不修改Relay业务或compiler。
此前“去掉显式yield”的设计不等于“没有挂起点”；不要重新引入`yield`作为规避方案。

## 精确问题

Relay真实实现的`responsesCompleted`为`maySuspend=true`，对应interface FileIR为false，PackageArtifact
拒绝二者。必须回答：

1. source interface是否已有表达“调用可能挂起”的语法或effect声明；
2. 若有，Relay只是漏写声明，还是compiler没有把声明投影进FileIR/public callable signature；
3. 若没有，canonical owner应是：
   - interface显式ABI；
   - package/service/file级effect；
   - 绑定具体实现时由compiler验证并投影；
   - 或`maySuspend`不应参与interface conformance/identity；
4. 多个实现具有不同挂起行为时，调用方、actor调度和ABI兼容应看到什么稳定语义。

## 审计要求

1. 搜索`skiff/doc/reference`及直接架构文档对async、effect、suspension、stream.next、interface method的现有
   规则，明确引用，不根据Relay单点发明新语义。
2. 从Relay source → interface/implementation FileIR → public instance projection → Package Local ABI /
   identity validator逐跳列出`maySuspend`字段在哪里生成、在哪里比较。
3. 全量检查当前真实package/service interface：
   - interface method声明与implementation `maySuspend`一致/不一致数量；
   - 是否已有挂起实现成功通过的source写法；
   - 影响面是否仅Relay。
4. 确认改变`maySuspend`是否改变Package Local ABI、build、ServiceContract或operation identity，以及需要重建
   的最小DAG。
5. 给出唯一后继：
   - 若现有语言规则已决定修复，返回`TASK_EXECUTABLE`和最小production/test边界；
   - 若确实缺少语言语义，返回`TASK_NOT_EXECUTABLE`，只列需要用户选择的具体方案及可观察差异。

## 边界与交付

- Skiff与Internals production只读；
- 可用clean Relay worktree `68c7d679`生成temporary FileIR/artifact probe，但不得改/提交其文件；
- 不操作stable/live、外部上游；
- 不派子Agent。

在本任务Skiff worktree写
`P5-F382-interface-suspension-projection-audit-result.md`并本地commit，worktree clean；不
merge/rebase/push。
