# P5-D36：WebSocket Result Materialization Audit Result

状态：complete；R26 exact candidate保持只读clean。

首因在compiler source/lowering，而非fixture、F24 shape/boundary plan、projector或transport。target-typed object literal虽按
目标类型通过assignability，source只保留源码实际字段和匿名record，没有持久化最终record/union branch或缺失nullable字段的
`SyntheticNull`；lowering随后无条件生成`ExprIr::MapLiteral`，interpreter返回`HeapNode::Map`。F24B正确要求canonical
record/union为exact Object，故R26 accept return在detach前失败。

当前fixture已显式给出accept四字段，手工补字段无效。reject根对象、nominal Context与non-null policy嵌套object会被同一
缺口遮挡。nullable contract字段在service value中必须作为显式Null存在，不能用transport header的absent倒推放宽boundary。

冻结修复：F25A由compiler/source持久化target、唯一branch与递归materialized fields，对缺失nullable生成SyntheticNull并
拒绝missing-required/extra/ambiguous/targetless；F25B只消费该fact把record/union递归降为Construct，明确Map/JSON才是
MapLiteral。R27接收source→IR/heap语义，I25刷新受影响cheap combined，R28只运行一次真实smoke。无需公共设计决策。
