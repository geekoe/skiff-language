# P5-F09：Router Control-Wire Bootstrap Repair

## 输入、owner与限制

- 输入：D11完成；exact integration `7f3681022993685b5ee31d972120483b1e2b58ab` / tree
  `d2375116b5865acf80c7c0f7f820804ab8046bc4`，已包含F04A implementation checkpoint。
- 独立worktree/branch，一个clean commit，不merge/push。R10 PASS只解锁F04A真实Host恢复，不提前解锁整个F03B。
- 单一owner限于Router production Runtime endpoint/session、capability registry/identity、assembly control接线、health与
  直接tests。允许删除或降级`AssemblyRuntimeEndpoint`的production owner。
- 不改`runtime/**`、F03A/F03A2 shared wire/codec/corpus、activation store/snapshot/gateway、F05 ABI、test-runner、
  scripts、manifest或Cargo.lock；不实现F03C startup/lifecycle/request/drain；不操作stable。

## 完成态

production server只实例化统一`RuntimeEndpoint`。同一真实socket先接收binary `runtime.capabilities`，完成且冻结
`runtimeId === replicaId`身份后，才接收binary `assembly.activation/register|prepared|reject`及其余既有Runtime
消息。注册前、身份变化、非空control payload、错误direction、text或bare binary均以1008 fail closed。

`assembly.activation`必须先按共享frame type分流，再用F03A direction-aware codec解码；generic runtime validator
保持原接受集。Router发送prepare/commit/abort也只用共享binary codec，Runtime返回prepared/reject/register走同一
socket，不新增ACK或text兼容。

`RuntimeRegistry`分别拥有capability-session与committed healthy replica查询/快照；health同时暴露两者，capability
连接不等于admitted registration且不可dispatch。统一endpoint继续覆盖runtime.register/health、request/response/
cancel、connection.send与actor/spawn，禁止用逐case缩减dispatcher替代。

## 写入边界与验证

最小production写集是`router/src/router/runtimeEndpoint.ts`、`runtimeRegistry.ts`、
`assemblyRuntimeRegistry.ts`、`assemblyControlPlane.ts`、`server.ts`及`assemblyRuntimeEndpoint.ts`的删除/降级，
外加直接Router tests。若真实代码要求更多文件，必须仍属于同一endpoint/session leaf owner并在回报中逐项说明。

```bash
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- \
  tests/protocol.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/active-assembly-reload.test.ts \
  tests/assembly-replica-dispatch.test.ts
git diff --check
```

聚焦探针必须覆盖同socket `capabilities → activation/register`不掉线，register-before-capabilities、身份变化、错误
payload/direction/text/bare control均1008，双向六种activation control golden frame，health两类状态独立、未注册连接
不可dispatch，以及actor/spawn、runtime.health、response/cancel/connection.send回归。production server反搜只能实例化
统一endpoint；回报source/commit/tree、single commit/clean/lock、完整接受矩阵与extra-review。

## R10 acceptance record

F09 candidate `84e33dd2cc7df98fc5a511881f2e0cedc1d540db` / tree
`aed0fdade3547322ea85b79e2676174a35aad6b4`由独立R10判定PASS并合流为`ff7a4df`。single endpoint、capabilities-first
identity、双向binary activation、health双状态与完整generic accepted-set均通过；聚焦104/104，真实socket新增4/4。
full Router在base/candidate保留相同8个既有F03B/fixture失败，candidate未新增或改变失败；lock/corpus/shared codec未变。
