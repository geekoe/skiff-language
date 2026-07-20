# P4-T07：Unified Ingress / Internal Dispatcher

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.6、§2.8–§2.10、§6.2、§12、§14。
- 风险/验收组：高风险request entry/active generation；与T08/T09合流后由R03验收。
- 当前成熟度：R02已验收execution lanes；完成后是runtime entry checkpoint，不是稳定候选。
- 有效证据：本任务clean commit及exact R02 checkpoint。host active state、request wire projection、dispatcher
  call graph、outbound removal、fixture或测试变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：R02 PASS；与T08/T09并行。
- 解锁：R03。
- branch：`codex/p4-t07-unified-ingress`。
- worktree：`/Users/geek/workspace/skiff-p4-t07-ingress`。
- 五分钟内真实edit。不得改变router wire公共语义或增加legacy route fallback；现有wire无法确定性形成selector时
  回报精确缺口。

## 写入范围

独占`runtime/request/**`、`runtime/host/**`canonical assembly request entry/active context set与旧outbound service
injection删除面；必要时独占`runtime/request-contract`/`runtime/transport`中严格`IngressSelector`投影。不得修改
router、T04–T06 lane或T09 checker。

## 完成态

1. whole-assembly admission在publish前为每个deployment建立独立ActivationContext set，并与active assembly
   generation一起原子发布；candidate失败保留旧set。
2. binary HTTP/WebSocket request metadata确定性投影canonical `IngressSelector`；缺host/method/path/protocol或
   歧义fail closed。Phase 05待迁移的build/operation/display字段不得参与target fallback。
3. request只从一个pinned `ActiveAssemblyRoute`取得activation、canonical descriptor与provider operation target；
   请求期间reload不混合generation。
4. ingress与internal service call调用同一个production `InProcessBoundary` dispatcher symbol，差异只在caller/
   payload adaptation；request layer不复制provider lookup/materializer。
5. request path零artifact/pointer/load/link I/O；empty/no active/missing selector稳定失败，不进入旧route registry。
6. assembly执行不再注入`OutboundServiceContext`/router sender作为service call capability；package-test/Phase 05旧
   consumer保持其既有精确fail-closed fence，不建dual path。
7. cancel、stream response与request supervision绑定同一request generation/context lifetime。

## 最早探针与唯一验证 ownership

- ingress/internal两个真实入口的dispatcher spy记录同一symbol/contract operation；target查找零额外resolver read。
- request A pin generation N，reload N+1后A继续N，新request用N+1；failed reload不替换context set。
- 变异buildId/operationAbiId/target display不重定向canonical ingress；fake legacy registry panic保持零调用。

```bash
cargo test -p skiff-runtime-request assembly_ingress
cargo test -p skiff-runtime-host in_process_request_entry
cargo test -p skiff-runtime-host active_generation_context
git diff --check
```

不得运行完整runtime gate或live instance。

## 回报

提交一个commit，回报wire→selector规则、generation/context状态机、single-dispatcher证据、旧route/outbound反向搜索、
命令与自验收矩阵。
