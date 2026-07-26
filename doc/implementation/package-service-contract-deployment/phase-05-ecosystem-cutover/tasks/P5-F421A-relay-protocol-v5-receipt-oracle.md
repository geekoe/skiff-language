# P5-F421A Relay protocol v5 receipt oracle

状态：Ready（N5 前置的非生产 oracle 同步）。

## 直接父节点

- `P5-F420I-final-n4-gate-result.md`
- `P5-D93-suspension-current-base-reconciliation-audit-result.md` 第 8.6 节

N4 已通过并解除 F421。D93 已确认 current Internals Relay receipt test仍有两个 protocol v4
positive validator及其 synthetic fixture；terminal ServiceContract/protocol generation已经是v5。
该同步不是 production migration或设计决策，必须在 Internals独立提交。

## 精确起点

- Skiff task/result repo：
  `29419bc999d441b78f1e452a454c2b24e6e30a87` /
  `2349d65c781c363fdccf0dada2f18d517d8d0f75`；
- Internals implementation：
  `960cc4bd722cbbad41fdd5e064663ad505e4f3ac` /
  `33a838176990193cd01be495a7b692623baa4793`。

启动时验证 exact commit/tree与 clean状态。

## 唯一写入

Internals：

```text
codex-relay/service/service-api-receipt.test.mjs
```

Skiff：

```text
本任务 result
```

不得修改 Relay production source、api/service/package manifest、其它 ecosystem owner或 Skiff实现；
不得派子 Agent、merge/rebase/push/stable/live。

## 实现与验证

1. 两个 generated receipt validator的 service protocol prefix从v4收敛到v5。
2. 同一 test的 synthetic `serviceProtocolIdentity` fixture从v4收敛到v5。
3. ContractOperationId继续为v1；deployment继续为v2；两项 operation与30项 HTTP gateway断言不变。

```bash
node --test codex-relay/service/service-api-receipt.test.mjs
rg -n "skiff-service-protocol-v4" codex-relay/service/service-api-receipt.test.mjs
git diff --check
```

预期4/4，v4反搜0。Internals implementation单一commit；Skiff result单一独立commit。两个 worktree
都保持clean，不运行fresh ecosystem proof。

