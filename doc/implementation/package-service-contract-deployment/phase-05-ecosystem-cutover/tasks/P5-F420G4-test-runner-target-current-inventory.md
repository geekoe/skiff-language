# P5-F420G4 Test-runner target current inventory

状态：Ready。

## 直接父节点

- `P5-F420G-tooling-closure-batch.md`

F420F 的 B4 已证明 test-runner 当前有两个独立 integration target：
`package_service_contract_deployment` 与 `canonical_std_seed_bootstrap`；后者不是 recursive wrapper，
旧 test 仍断言只有一个 target。

## 唯一写入

```text
scripts/tests/test-runner-runtime-isolation.test.mjs
本任务 result
```

从 batch exact start/tree 启动。更新 inventory oracle 精确接受两个 current target，并继续证明：
canonical cutover target ungated、没有 recursive wrapper、没有多余 target。不得修改 Cargo
manifest、test-runner、其它 test；不得派子 Agent、merge/rebase/push/stable/live。

```bash
node --test scripts/tests/test-runner-runtime-isolation.test.mjs
git diff --check
```

预期 3/3。实现/result 分开提交，保持 clean；范围扩张立即停止。

