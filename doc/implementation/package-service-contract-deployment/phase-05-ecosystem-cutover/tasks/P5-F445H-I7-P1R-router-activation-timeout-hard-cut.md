# P5-F445H-I7-P1R Router activation timeout hard cut

状态：`IMPLEMENTATION_COMPLETE`。

## 1. 输入与目标

本节点承接
`P5-F445H-I7-P1D-activation-prepare-timeout-authority-result.md`，只实现Router与本仓库
operator配置/生成工具的timeout-domain hard cut。

| 项 | 值 |
| --- | --- |
| baseline commit | `564636a557c638d1b21b66fcc3394ea076243ff2` |
| baseline tree | `22b6089fc0ce22358ea28aa590bd2f01bb6caeba` |
| branch | `codex/p5-f445h-i7-p1r-router-timeout` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-p1r-router-timeout` |
| integration owner | `/root/phase05_integration_steward` |

零worktree预检确认现有错误绑定位于Router server：`requestTimeoutMs`同时传给activation
coordinator和WebSocket generation release。I7 Relay/AIHub与Agine isolated ledgers证明默认20秒会在正常
prepare完成前返回504。

## 2. 写集

允许修改：

- Router activation timeout常量、config、server、coordinator及直接tests；
- WebSocket generation lifecycle直接test；
- `router/router.example.yml`；
- runtime-stack配置renderer、deploy/dev/instance生成入口及直接Node tests；
- 本task/result。

禁止修改test-runner Rust、Host/runtime production、Internals、stable/live/network/Mongo/OAuth/browser状态。

## 3. 实现合同

1. Router配置新增`activation.prepareTimeoutMs`，缺省`120000`，显式YAML值必须是正safe integer。
2. Router CLI只有`--activation-prepare-timeout-ms`一个对应拼写；server只把该值交给
   `AssemblyActivationCoordinator`。
3. `requestTimeoutMs`继续只传给HTTP/WebSocket业务dispatch，不参与activation。
4. coordinator自己的缺省同样是`120000`，只有该预算到期才以既有timeout错误abort。
5. WebSocket generation release不再接收`requestTimeoutMs`，使用既有独立`5000`默认；不新增公开release
   配置。
6. deploy、dev init与instance配置显式生成独立activation配置；CLI显式值做正safe-integer校验。
7. 不改变control-plane对timeout 504、participants 503及其它错误的既有分类。

## 4. Gate

- 先用新增配置测试记录真实RED；
- Router typecheck与全量非live测试；
- runtime-stack config/deploy/instance直接Node tests；
- scripts syntax gate；
- `git diff --check`与反向搜索；
- 提交clean后交integration owner，不自行合并或push。
