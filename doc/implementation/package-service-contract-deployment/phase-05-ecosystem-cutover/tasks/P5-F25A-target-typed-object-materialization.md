# P5-F25A：Target-typed Object Materialization Facts

依赖D36 complete。独占`compiler/source/**`及直接tests，不改lowering/runtime/fixture/artifact/Router。独立worktree/branch，
一个clean commit。

每个target-typed object literal必须持久化resolved target、最终record或discriminated-union唯一branch、字段顺序与递归
materialized expression facts。缺失nullable字段生成显式`SyntheticNull`；缺失required、extra、ambiguous branch和无目标
object literal按冻结静态语义编译失败。目标类型递归传入nested nominal Context、connection policy及其nullable子字段；
明确Map/Json target继续保留map materialization fact。不得加入WS名字特判或修改runtime接受集合。

直接tests覆盖accept四字段、accept省略biz/policy补null、reject、nominal Context、nested policy、Map/Json、targetless/
ambiguous/extra/missing-required negatives及source fact snapshots。只跑compiler/source精确tests、check/fmt/diff-check；
禁止lowering实现、real smoke/full/I16/Host/stable。
