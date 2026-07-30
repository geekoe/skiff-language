# P5-F420G2 Dev-sync RuntimeAssembly v2 fixture

状态：Ready。

## 直接父节点

- `P5-F420G-tooling-closure-batch.md`

F420F 的 B2 已证明唯一失败是 fake compiler receipt 仍构造
`skiff-runtime-assembly-v1:*`，而 current activation API 只接受 exact RuntimeAssembly v2。

## 唯一写入

```text
scripts/tests/package-service-dev-sync.test.mjs
本任务 result
```

从 batch exact start/tree 启动。只把该 fixture 收敛到 canonical v2 identity；不得修改
production dev-sync/activation、增加 v1 兼容或改变其余四项测试。不得派子 Agent、
merge/rebase/push/stable/live。

```bash
node --test scripts/tests/package-service-dev-sync.test.mjs
rg -n "skiff-runtime-assembly-v1" scripts/tests/package-service-dev-sync.test.mjs
git diff --check
```

预期 5/5，v1 反搜为0。实现/result 分开提交，保持 clean；范围扩张立即停止。

