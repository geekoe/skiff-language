# P5-F445H-I7-P0 official packages prepared consumer gate

状态：`COMPLETE / FAIL`。

本节点是 I7 official-package prepared consumer 的只读 gate owner。它从 I7R 冻结的
`skiff-packages` Phase 05 candidate 运行离线、非零的 `http-session,track` consumer gate，
只负责发现第一处当前契约 blocker；不修改或重新实现 package、Skiff 或 Internals 代码。

## 1. 直接父节点与冻结输入

直接父节点：

- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`
- `P5-F445H-I6K-R4-independent-current-scope-reacceptance-result.md`

两者继续追溯到唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。I6 已满足
`I6_ACCEPTED = YES / I7_UNBLOCKED = YES`。

冻结输入：

| Repository | Commit / tree | 用途 |
| --- | --- | --- |
| Skiff integration | `54fb087f122c53aed5c017260c7bca43e2b54404` / `008d3a05927cdf845004db980d1b46de263612be` | current compiler、std、CLI 与 test runner |
| `skiff-packages` prepared candidate | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` / `44081bd0498919086c13adea97c07722cb768352` | official-package consumer |

`skiff-packages/main@5defc94161cee14def1a6bbb340308004e65b741`
不是本 gate candidate；不得在 main 重做 prepared branch 的既有 Phase 05 修复。

## 2. Gate、边界与完成标准

唯一动态命令：

```bash
CARGO_NET_OFFLINE=true \
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
  node scripts/test-packages.mjs --list http-session,track

CARGO_NET_OFFLINE=true \
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
  node scripts/test-packages.mjs http-session,track
```

第一条必须列出 `tests/http-session/session.test.skiff` 与
`tests/track/track.test.skiff` 两个非零 root，并证明 offline /
`externalRequests=false`。第二条按依赖顺序先发布并测试 `http-session`，再执行
`track`。

禁止 production/test/runner 修改、依赖安装、network、stable/live、MongoDB、push，
以及在其它 repo 吞并失败。首个失败必须分类为 package source、Skiff contract、
environment 或 gate infrastructure；若是 repo-local source blocker，只记录最小 owner、
被遮挡范围和恢复顺序。

PASS 才能设置 `P0_COMPLETE = YES` 并解除 Agine A 与最终 J 的 package 前置。
FAIL 不解除任何下游。

## 3. Evidence lifecycle

gate 使用 detached disposable worktree 和 runner-owned fresh artifact/Cargo roots。运行结束必须
清理临时 roots 和 worktree，保持 Skiff 与 package integration worktree clean。Skiff compiler、
std、CLI/test runner、package candidate、package roots或 runner变化都会使证据失效。

