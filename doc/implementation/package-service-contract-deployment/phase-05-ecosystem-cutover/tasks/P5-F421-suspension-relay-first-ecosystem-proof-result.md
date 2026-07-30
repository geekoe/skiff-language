# P5-F421 Suspension Relay-first fresh ecosystem proof result

状态：预检失败；未启动 fresh rebuild。

```text
TASK_SCOPE_EXPANDED
N5_FAIL
```

## 1. 精确 blocker

任务冻结的 Skiff integration input 是：

```text
commit  9f39580655ecbd433235cdb7de19d823d670d4a9
tree    d20cd4ccd8f11042a1f4bc6dac69d3ccda1116b9
```

启动时 `/Users/geek/workspace/skiff-phase-05-integration` 的实际值却是：

```text
commit  c728c2220e4b6936d7c78c508584cbc4399cb745
tree    89357d258b19adfdfa3fbf8db2823c2db9c7f4de
```

`9f395806..c728c222` 的 diff 只有：

```text
A doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F421-suspension-relay-first-ecosystem-proof.md
```

production diff 为零，但叶子任务要求三个 input 的 commit/tree 精确匹配，并明确规定任一 input
不匹配即停止、不得自行吸收漂移。因此本 gate owner 没有把 task-only drift 当作等价 input，也没有
移动 integration ref、改写 task input 或从冻结 commit 绕过 actual-root gate。

最小 successor owner 是 phase-05 integration coordinator：恢复该 integration root 到冻结的
exact commit/tree，或签发一份显式重新冻结到实际 commit/tree 的后继 gate task。两种动作都不属于
本节点的只读 production 权限。

## 2. 启动快照

| repo / checkout | actual commit | actual tree | status | 与冻结输入 |
| --- | --- | --- | --- | --- |
| Skiff task checkout | `c728c2220e4b6936d7c78c508584cbc4399cb745` | `89357d258b19adfdfa3fbf8db2823c2db9c7f4de` | clean | `HEAD^` 精确为冻结 Skiff input；checkout 相对 parent 只新增 task 文件 |
| Skiff integration | `c728c2220e4b6936d7c78c508584cbc4399cb745` | `89357d258b19adfdfa3fbf8db2823c2db9c7f4de` | clean | **mismatch**：要求 `9f395806...` / `d20cd4c...` |
| Internals integration | `baf0c907ee26e48a5fb4c153825c233bde3a6234` | `13f2f6e604fedbad80e0390e5408507430e28f8c` | clean | exact |
| skiff-packages integration | `0972e65604cd4cfd45bcdb289cfe5019f57dc265` | `1849f97a1f1217b95e6e349bc529eaaf220a62f4` | clean | exact |

四个 checkout 的 Git index lock 均不存在。进程快照中没有发现除本次只读诊断命令本身之外、
命令行指向三个 integration root 的 Git/Cargo/Node/pnpm writer。

相关 tracked lock blobs：

| repo | lock | blob |
| --- | --- | --- |
| Skiff | `Cargo.lock` | `f484516657ce13f88081ee0c57e437b227bbae31` |
| Internals | `agine/package-lock.json` | `fbda18b9af992941e61b9a50ead5fdc2a01c0d86` |
| Internals | `packages/agent/scripts/provider-name-migration/Cargo.lock` | `838d22c7313cdc24faff6898ff1c779821c52dfb` |
| Internals | `shared-client/package-lock.json` | `f0506c15020d58bb8b14758b41ee1fc0d7ddc65b` |
| Internals | `skiff-platform/client/package-lock.json` | `632af075535bdb4a7ba87663e3196c94339f8af7` |
| skiff-packages | tracked root/subtree lock search | none |

N4 executable candidate
`29419bc999d441b78f1e452a454c2b24e6e30a87` 是冻结 Skiff input 和实际 Skiff integration
HEAD 的 ancestor。candidate 到两者排除 phase-05 task/result 文档后的 production diff 都为空。
共享 target `/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target` 未被本任务使用。

## 3. 停止点、遮挡节点与实际计数

最后成功阶段是 instruction/直接父节点读取；gate 前置预检第 1 项失败。第 2–5 项没有继续作为
PASS 条件执行。

由于停止发生在创建 task-owned temporary root 之前：

- temporary root：未创建；
- source mirror / mirror tree：`0`；
- fresh artifact store：未创建；
- canonical publish 命令：`0`；
- 第一个 canonical 命令及 stderr：未调用，因此无 stderr；
- package artifact / contract / protocol / deployment / assembly records：均为 `0`；
- Relay-only assembly：`0`；
- interface/concrete pair、callback、mapping、consumer receipt：均为 `0`；
- canonical mutation negatives：`0`；
- sibling wave：未启动，独立 production compatibility error 未观察到；
- `final-receipt.json`：未生成，因为唯一 fresh rebuild 从未开始。

被遮挡节点为全部 rebuild DAG：

```text
std
  -> llm-api
  -> llm-providers
  -> Relay
  -> Relay-only RuntimeAssembly / Relay verdict
  -> {Agent, http-session, track, AIHub, Account, Registry}
  -> Agine
  -> complete RuntimeAssembly
  -> ecosystem census / negatives / reverse search
```

## 4. 边界声明

没有修改 Skiff、Internals 或 skiff-packages 的 production、test、fixture、manifest 或 oracle；
没有创建或修改 source mirror；没有访问 stable/live/instance/watch/MongoDB、旧 artifact store、
旧 receipt、旧 lock 或 waiver；没有派子 Agent；没有执行 merge、rebase 或 push。

本失败不声称发现 production compatibility 错误。唯一 blocker 是启动时 Skiff integration
commit/tree 与任务冻结输入不精确相等。未满足 input gate，因此不能写
`PHASE_05_ECOSYSTEM_PROOF_COMPLETE`。
