# P5-F421A Relay protocol v5 receipt oracle result

状态：`PASS`。Relay receipt 的 checked-in non-production oracle 已收敛到 service protocol v5；
聚焦测试精确 `4/4 PASS`，protocol v4 反向搜索为 0。

## 1. Exact start 与 implementation

| 锚点 | commit | tree |
| --- | --- | --- |
| Skiff exact start | `29419bc999d441b78f1e452a454c2b24e6e30a87` | `2349d65c781c363fdccf0dada2f18d517d8d0f75` |
| Skiff task checkout | `677d4f7aa1f45df97c636843c034a067d6a5cc9e` | `7f9fe2bf0e452781749ad49dda9b87d261826962` |
| Internals exact start | `960cc4bd722cbbad41fdd5e064663ad505e4f3ac` | `33a838176990193cd01be495a7b692623baa4793` |
| Internals implementation | `4dba86731970a6798a26f3d831306c92a0bb9936` | `13f2f6e604fedbad80e0390e5408507430e28f8c` |

Skiff task checkout 的 parent 精确为记录的 Skiff exact start，且相对该起点只新增本任务文件。
Internals 启动时 HEAD/tree 精确匹配任务起点；`git merge-base --is-ancestor` 证明该起点是
implementation 的 ancestor。result-only commit / tree 由交付消息记录。

## 2. Receipt oracle 同步

`codex-relay/service/service-api-receipt.test.mjs` 的 implementation diff 精确为三处
`skiff-service-protocol-v4` 到 `skiff-service-protocol-v5` 的替换：

1. Service API receipt positive validator；
2. generated ServiceContract positive validator；
3. synthetic `serviceProtocolIdentity` fixture。

ContractOperationId 继续使用 `skiff-contract-operation-v1`，deployment identity 继续使用
`skiff-deployment-artifact-v2`。两个 service operation 与 30 个 HTTP gateway/ingress 的
闭合集合断言没有改动。没有修改 Relay production source、API/service/package manifest 或其它
ecosystem owner。

## 3. 聚焦验证

| 命令 | 结果 |
| --- | --- |
| `node --test codex-relay/service/service-api-receipt.test.mjs` | `4/4 PASS` |
| `rg -n "skiff-service-protocol-v4" codex-relay/service/service-api-receipt.test.mjs` | 0 命中 |
| `git diff --check` | PASS |

## 4. 边界

Internals implementation 只修改任务授权的一个 test，Skiff result 只新增本文档。没有运行 fresh
ecosystem proof、stable 或 live；没有启动 instance/watch registry，也没有派子 Agent或执行
merge、rebase、push。implementation 与 result 分开提交，两个 worktree 最终 clean。
