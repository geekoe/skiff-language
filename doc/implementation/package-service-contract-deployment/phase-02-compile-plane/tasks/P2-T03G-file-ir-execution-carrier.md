# P2-T03G：File IR Execution Type Representation

## 目标

让File IR executable signature只消费T03F exact facts并生成方案A的确定性execution representation，删除从
AST/display text重新lower dependency types的第二owner。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”中File IR execution type
representation规则与“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T03E、T03F。
- 独占`compiler/lowering` executable-signature/type projection、必要的窄source→lowering/driver handoff与
  直接File IR tests。
- 不修改source type semantics、PackageArtifact projection或integration fixtures。

## 完成态

1. 唯一投影规则：Local保留local `TypeRefIr`；Container递归生成现有native/container execution shape；
   Nullable递归保留；Contract leaf固定生成无参数builtin/native `unknown`。
2. opaque `unknown`不携带alias/stable key/ContractTypeId/schema，不参与ABI、boundary eligibility或protocol
   identity；File IR中contract来源的`ServiceSymbol`归零。
3. executable declaration lowering不再从AST type text重新解析参数/返回；全function/impl method都从T03F事实
   取得精确签名，再做一次execution projection。
4. 参数名/顺序、arity、receiver、return与`may_suspend`保持；unrelated external type symbol存在与否不能改变
   contract-typed executable的结果。
5. 审计其它File IR type-bearing位置：可消费同一projection的必须复用；本阶段不支持的显式fail closed，不得
   留下display/ServiceSymbol fallback。runtime boundary consumer暂时不可用不构成compat理由。

## 聚焦验收

- direct File IR tests覆盖provider wrapper、consumer/private helper、nested container/nullable、impl receiver，
  并比较external symbol空/非空时输出相同。
- 反向证明无法从execution representation恢复ContractTypeId，旧qualified external fallback归零。
- 运行lowering/driver最小测试/check、changed-file rustfmt和`git diff --check`，不运行Phase gate。

## 执行合同

- DAG：波次9b关键路径；与T04B并行，完成后解除R10I。风险：高；execution handoff独立验收面。
- worktree：`/Users/geek/workspace/skiff-p2-t03g-file-ir-exec-carrier`；分支：
  `codex/p2-t03g-file-ir-exec-carrier`；从含T03E/T03F的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；T03F facts、lowering executable schema、
  TypeRefIr或File IR identity变化即失效。

