# P5-F432 Test-runner unblock and combined validation wave

状态：Ready。Runtime/Router实现后的fixture与生态验证波次。

## 直接父节点

- `P5-F431B-runtime-connect-final-mechanical-closure-result.md`
- `P5-F429B-router-downlink-only-websocket-gateway-result.md`
- `P5-F425A-skiff-websocket-authoring-compiler-checkpoint-result.md`
- `P5-F428A-agine-http-direct-body-service-result.md`
- `P5-F428B-aihub-http-correlation-service-result.md`

这些结果继续追溯到唯一权威设计。F425A首先记录test-runner optional-handler seam；F428A/B证明
它会在service source compile前遮挡；F431B和F429B已完成Runtime/Router production实现。

## 精确输入与成熟度

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `b2533567f84ec30b24c7445c32562ef7e3d2b9a1` | `757ed3d6a3b980fe937d5e09d08dde2d530b8287` |
| Internals integration | `5895085` | browser/service correlation-free integrated state |

当前仍是实现检查点：三个test-runner类型错误遮挡所有Skiff service source动态测试，旧WebSocket
fixtures/tooling尚未收敛，因此不能称为预验收候选。

## DAG

```text
F432A test-runner optional-handler compile checkpoint
  ├─> WebSocket fixture/tooling convergence
  ├─> AIHub generated identity + isolated HTTP stream combined
  └─> Agine service/browser direct-body combined
          |
          v
Runtime+Router current connect/downlink combined probe
```

F432A是短共享检查点；完成后立即扇出互不重叠的Skiff fixture、AIHub和Agine节点。任何combined
owner只验证冻结候选，不顺手修source。

本wave不访问stable/live/真实provider，不merge/rebase/push。
