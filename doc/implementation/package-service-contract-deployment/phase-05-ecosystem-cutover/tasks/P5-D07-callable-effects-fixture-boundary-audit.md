# P5-D07：Callable Effects Fixture Boundary Audit

## 角色与输入

由未参与F04实现的只读Agent检查F04真实consumer fixture为何不能得到Available boundary。输入为F04 dirty
worktree、D04/F04合同、compiler source callable-effects、dependency index/lowering与PackageArtifact projection。
不得编辑/提交，不给F04 verdict，不得建议artifact patch、cast、手调mutation primitive或用既有Host单测替代
isolated最终结果。

## 审计结论

这是production `compiler/source callable-effects` blocker，不是fixture-only问题：

1. `eval_call`先评估callee，而任意`DependencySourceAddress`无条件写入same-heap/unknown/suspend facts，导致已精确
   解析的package/contract call在target transfer前被污染。
2. 已解析`ContractOperation`仍走unknown external call，虽然`ContractDependencyIndex`已有exact operation descriptor。
3. 任意字段/容器写都退化为全effects Unknown；helper的直接标量参数字段写facts经canonical PackageArtifact导入
   consumer后继续污染，无法利用actual fresh record provenance。

Projection eligibility正确fail closed，不是修复owner；service target/lowering identity已精确。D07冻结F06仅改
callable-effects transfer/dependency lookup，并给出helper fresh record mutation → detached payments contract call的最小
source。审计完成只解锁F06，不解锁F04接收。
