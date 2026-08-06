# P5-F445H-I7-P1R Router activation timeout hard cut result

状态：

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

```text
PASS
P1R_COMPLETE = YES
P1T_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

## 1. 结果

Router的三个timeout domain已经在production wiring中解耦：

| 域 | 最终owner |
| --- | --- |
| external business request | `requestTimeoutMs`，继续只进入HTTP/WebSocket dispatch |
| activation prepare | `activation.prepareTimeoutMs`，缺省`120000` |
| WebSocket generation release | lifecycle独立缺省`5000` |

`AssemblyActivationCoordinator`不再读取业务request预算。Router server也不再把业务预算传给generation
release。没有新增release公开配置、fallback、dual-read或旧cross-wiring。

## 2. 配置与工具

- Router YAML接受`activation.prepareTimeoutMs`；缺字段或旧配置使用`120000`。
- 显式YAML的零、负数、小数、字符串、对象及超safe-integer值fail closed。
- Router/dev/deploy的唯一CLI拼写是`--activation-prepare-timeout-ms`。
- runtime-stack renderer要求调用方显式传入正safe integer。
- deploy未显式传值时生成`120000`；instance旧配置未声明`activation`时也归一化为`120000`。
- checked-in Router example、dev init、instance init和deploy均显式写出独立activation配置。

## 3. 动态证据

真实RED：

```text
router/tests/config.test.ts
4 failed / 25 passed
```

失败明确显示旧实现不读取、不返回也不校验`activation.prepareTimeoutMs`。

最终GREEN：

| 命令 | 结果 |
| --- | --- |
| Router聚焦config/coordinator/WebSocket lifecycle | `43 passed` |
| `pnpm --filter @skiff/router type-check` | PASS |
| Router全量`pnpm test` | `59 files / 842 tests passed` |
| runtime-stack config/deploy/instance Node tests | `25 passed` |
| `pnpm --dir scripts type-check` | PASS |

假时钟正反例证明：7秒业务request配置不改变activation配置；prepare超过20秒仍保持pending，只有到120秒
activation预算才abort；WebSocket release仍在独立5秒预算到期。

## 4. 静态证据与边界

- 反向搜索没有`prepareTimeoutMs: config.requestTimeoutMs`或
  `releaseTimeoutMs: config.requestTimeoutMs`；
- `requestTimeoutMs`只保留业务gateway传递；
- control-plane timeout分类代码未修改，504/503/其它错误路径保持原状；
- 未修改test-runner Rust、Host/runtime production或Internals；
- 未运行stable/live/network/Mongo/OAuth/browser，也未push。

最终commit/tree由Git handoff记录；result不自引用自身commit。
