# P5-F445H-I7-P1D Activation prepare timeout authority

状态：`READY_FOR_DOCUMENTATION_CUTOVER`。

## 1. Parent chain and purpose

本节点承接I7真实隔离验证的timeout preflight。现有实现把Router `requestTimeoutMs`同时用于external
business request和RuntimeAssembly activation prepare，导致正常的resolve/load/link/admit控制事务被业务
request预算提前abort。P1D只先修正文档合同，不修改production或tests。

```text
I7 isolation timeout
  -> P1 read-only preflight
  -> P1D authority cutover
  -> P1 Router/config/test-runner implementation
  -> focused acceptance
```

## 2. Frozen baseline and ownership

| 项 | 值 |
| --- | --- |
| Skiff baseline commit | `54ef44d0ed6a22f495be3509c273d24852521cf1` |
| Skiff baseline tree | `bb1a8f719e5d49db74db02164c5f0d76db209ebb` |
| integration branch | `codex/package-service-phase-05` |
| leaf branch | `codex/p5-f445h-i7-p1d-timeout-docs` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p1d-timeout-docs` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检确认：现有canonical architecture/reference只把`requestTimeoutMs`与deployment
`policy.timeoutMs`定义为external request预算，没有要求它们控制assembly activation或WebSocket generation
release。因此P1D没有遇到相反权威要求，无需设计升级。

## 3. Frozen timeout domains

1. `requestTimeoutMs`只属于external business request的平台cap。
2. Deployment `policy.timeoutMs`只参与一个external request的effective deadline，并且只能收紧平台cap。
3. RuntimeAssembly activation prepare是控制面事务，只使用Router operator配置
   `activation.prepareTimeoutMs`。缺省为`120000`毫秒；显式值必须是正safe integer。
4. Prepare budget覆盖participants完成resolve/load/link/admit并返回prepared ACK的等待。只有该预算到期时，
   coordinator才以timeout原因abort pending activation并让control endpoint返回504。
5. Test-runner/activation client使用独立deadline，必须严格大于Router prepare budget；默认组合为
   `activation.prepareTimeoutMs = 120000`，建议client使用`150000`。
6. 普通HTTP/WebSocket dispatch deadline保持现有request规则，不被prepare budget改变。
7. WebSocket generation release使用独立release timeout；是否公开配置可由实现维持现状，但不得读取
   `requestTimeoutMs`、deployment `policy.timeoutMs`或activation prepare budget。
8. Skiff未发布，删除旧cross-wiring，不保留alias、fallback、dual-read或旧错误绑定。

## 4. Write scope

本任务只修改：

```text
doc/architecture/package-service-contract-deployment.md
doc/architecture/runtime-deployment-topology.md
doc/reference/runtime.md
router/README.md
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/
  phase-overview.md
  phase-plan.md
  tasks/P5-F445H-I7-P1D-activation-prepare-timeout-authority.md
  tasks/P5-F445H-I7-P1D-activation-prepare-timeout-authority-result.md
```

禁止修改production、config parser、tests、fixtures或其它repo；不运行build/test/live/stable，不访问network、
Mongo、OAuth或browser。

## 5. Completion and downstream handoff

P1D完成要求：

- canonical architecture、reference、Router README与Phase计划表达同一三预算域；
- request/deployment timeout不再具有activation/release含义；
- prepare默认、validation、504/abort owner与client ordering精确；
- Markdown fences配对、`git diff --check`与相关反向搜索通过；
- task/result交integration steward合流。

P1D PASS只解除P1 production implementation，不代表错误绑定已经从代码删除，也不恢复I7的timeout相关
动态证据。
