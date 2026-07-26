# P5-F422A Registry receipt current-generation closure result

状态：`PASS`。F422 唯一范围外的 Registry positive receipt oracle 已收敛到 current generation；
canonical `npm run test:registry` 在本次最终 implementation tree 上完成 Node 与真实 runtime 全量
执行，三类测试均为非零且零失败、零跳过。

```text
REGISTRY_STORAGE_CURRENT_PASS
```

## 1. Exact start 与 commits

| 锚点 | commit | tree |
| --- | --- | --- |
| Skiff result base | `9fc4fc5db7051bf751246fa126c38a7254d47b5b` | `aa14118b5efed0efb3ade1b4ac141fd219c2855e` |
| Skiff task checkout | `ad88776995a74120a31c192a815fa89c68ca398c` | `dc6c4407484e05fd625bd5eb00b492b16f55db46` |
| skiff-packages exact start | `1bc4504681e037fde4bfc92cd7b36f85a56b0fe0` | `d39c43c9a4c3dabaa287c01c44521b8d156cff8e` |
| skiff-packages implementation | `f73dfb8644a6d9f9751692a5eb9463928a7660b9` | `eb00877ef260d122552af1ff0491c74102adbd57` |

Skiff task checkout 相对 result base 只新增
`P5-F422A-registry-receipt-current-generation.md`。result-only commit/tree 由交付消息记录。

## 2. Receipt oracle closure

`scripts/registry-service-receipt.test.mjs` 的 implementation diff 精确同步五项 positive fresh
receipt expectation：

| receipt 字段 | current expectation |
| --- | --- |
| PackageArtifact schema | `skiff-package-artifact-v9` |
| Package build identity | `skiff-package-build-v10:sha256` |
| Package Local ABI identity | `skiff-package-local-abi-v7:sha256` |
| ServiceContract schema | `skiff-service-contract-v5` |
| ServiceProtocol identity | `skiff-service-protocol-v5:sha256` |

ContractOperationId 仍为 v1，ServiceDeployment 与 RuntimeAssembly 仍为 v2。20 项
operations/bindings、gateway/ingress 零计数及其余闭合集合断言均未放宽；五个旧 expectation 在该
文件反向搜索为 0。

## 3. Canonical 完整验证

在 implementation worktree 执行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration npm run test:registry
```

实际 exit `0`。该单一 canonical 入口先完成 Node authoring 阶段，再真实启动隔离
Mongo/router/runtime 并执行 Registry package tests；没有复用 F422 的 9/9 结果。

| 测试类 | discovered | executed | pass | fail | skip |
| --- | ---: | ---: | ---: | ---: | ---: |
| Registry source Node | 6 | 6 | 6 | 0 | 0 |
| Registry receipt Node | 1 | 1 | 1 | 0 | 0 |
| Registry service runtime | 9 | 9 | 9 | 0 | 0 |

Node TAP 总结为 `7 tests; 7 pass; 0 fail; 0 skipped`。receipt 的 fresh publish 输出
20 个 receipt operation ids、20 个 ServiceContract operations、20 个 deployment bindings，
gateway entries、deployment ingress 与 assembly gateway ingress 均为 0。runtime 输出逐项列出
9 个 `PASS`，总结为 `9 passed; 0 failed`；隔离实例使用临时 workspace 和动态端口
`46155`–`46158`。

## 4. 静态检查与边界

| 检查 | 结果 |
| --- | --- |
| `npm run type-check` | PASS |
| 五个旧 generation expectation 反向搜索 | 0 命中 |
| `git diff --check` | PASS |
| skiff-packages changed-file boundary | 仅 `scripts/registry-service-receipt.test.mjs` |

没有修改 Registry production、`.skiff` tests、manifest、20 项 operation、其它 script、lockfile
或 Skiff production/test。Skiff result 只新增本文档。没有访问 stable/live，没有 merge、rebase、
push，也没有派子 Agent。
