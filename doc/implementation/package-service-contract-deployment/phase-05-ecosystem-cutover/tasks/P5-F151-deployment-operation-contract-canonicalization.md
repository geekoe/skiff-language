# P5-F151：Deployment Operation Contract Canonicalization

状态：Ready

## 父节点

- `P5-D85-generated-contract-operation-canonicalization-audit-result.md`

## 写入与完成标准

- canonical转换必须复用/抽取`compiler/contract::service_owned_operation_contract`唯一规则；不得在deployment复制映射。
- 允许写compiler/contract公共projection helper、deployment operation validation及各自聚焦tests。
- deployment对同一package真实generated contract canonicalize后成功。
- 递归覆盖parameter/return/error/server-stream item/callback nominal closure与version-free IDs。
- 替换为另一个合法public type、缺失/非public type继续fail closed。

先列出并运行compiler-contract/deployment聚焦selector；格式、`git diff --check`。不运行完整gate。

worktree `/Users/geek/workspace/skiff-p5-f151`，branch `codex/p5-f151-deployment-contract-canonicalization`。
提交、不push、不操作stable。

