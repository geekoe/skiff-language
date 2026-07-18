# P2-T03C：Contract Call Type Checking

## 目标

让`payments.charge(...)`在source typed analysis阶段按同一validated `ServiceContract`的operation descriptor
完成参数和返回类型检查，而不是仅解析出operation ID后交给lowering。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“ServiceContract nominal types”、
“调用模型”与“Package编译”章节。

## 依赖与写域

- 依赖P2-T03A、P2-T03B。
- 独占`compiler/source`中的contract-call expression typing、小型新模块和直接测试。
- 不修改lowering、projection、compiled/projection-input或integration fixtures。

## 完成态

1. contract call按qualified alias与operation stable key解析唯一descriptor，验证arity、每个参数、返回类型及
   当前语言支持的suspend/call形态。
2. comparison复用T03B的canonical contract-aware type projection；contract nominal按`ContractTypeId`比较，
   builtin/container/nullable递归比较，不退化为字符串或结构碰巧相等。
3. unknown alias/operation、参数数目或类型错误、返回使用不匹配、unsupported inline contract shape均给出
   source compile error；不能先生成ServiceCallRef再把错误推迟到runtime。
4. resolved target继续只表达已经typed成功的真实contract operation；失败调用不能进入used operation集合。
5. 新逻辑从超长`expression_type_model.rs`拆到职责明确模块，assignability规则只有一个owner。

## 聚焦验收

- source tests覆盖正确调用、unknown operation、wrong arity、wrong argument、wrong return use与contract nominal
  mismatch。
- 运行source crate聚焦测试/检查及`git diff --check`；不运行Phase gate。

## 禁止项

- 不引入动态/无类型service symbol、provider target、remote/local binding选择或runtime stub。
- 不复制ServiceContract validator或T03A索引。

## 执行合同

- DAG：波次8c关键路径；完成后与T03D/T04A/R10H共同解除R10I。风险：高；source typed-contract验收组。
- worktree：`/Users/geek/workspace/skiff-p2-t03c-contract-call-typing`；分支：
  `codex/p2-t03c-contract-call-typing`；从含T03A/T03B的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试或扩大搜索。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；T03A facts、T03B type projection或expression
  typing owner变化即失效。
