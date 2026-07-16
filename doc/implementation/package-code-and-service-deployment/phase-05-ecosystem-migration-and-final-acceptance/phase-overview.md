# Phase 05：生态迁移与最终验收

状态：`outline-only`。前四阶段验收后再按当时仓库事实细化。

## 目标

迁移Skiff仓库fixture、`skiff-packages`官方packages和`internals`实际services，删除剩余旧模型
文档/工具/测试，并完成全量与真实系统验收。

## 进入条件

- config-only service、Runtime Assembly、InProcessBoundary已分别通过阶段验收。
- remote production path已删除，assembly reload/admission可运行。
- canonical reference/architecture需要迁移的差异清单已生成。

## 预计工作域

1. 把所有service源码迁为一个或多个PackageUnit source + config-only service deployment。
2. 共享实现抽到真正的package；不为每个service复制source tree或生成手写stub。
3. 更新compiler/runtime/router/CLI/dev watch/release registry文档、examples和fixture。
4. 删除旧 `service.yml + internal/*.skiff` source publication、`service_files`、remote relay相关golden。
5. 按新行为重写测试；只验证已删除架构的测试直接删除并注明replacement。
6. 分别在 `skiff`、`skiff-packages`、`internals`提交；不记录跨仓库commit pointer，未经用户
   要求不push。

## 验证层次

### 仓库全量

- 运行Skiff根 `pnpm test` / `pnpm verify` 或届时AGENTS指定的完整命令。
- 运行artifact/identity/compiler/runtime/router/CLI所有非live suites。
- 检查仓库无旧schema、source service policy、service relay或兼容reader残留。

### 本地系统

- 在Skiff语言worktree启动独立instance验证runtime/router变更，不污染stable instance配置。
- 启动至少两个独立runtime replica，各自加载完整相同assembly；验证CPU/heap独立、MongoDB/Redis
  等实际外部依赖按deployment配置共享。
- 验证ingress负载分发、replica摘除、atomic reload、in-flight drain和provider closure一致性。

### 生态 smoke

- 迁移后的官方package与代表性private service真实build/dev/test。
- 改动影响Agine聊天链路时，按workspace AGENTS先重建/重启dev runtime，再运行
  `npm run e2e:chat-smoke`。
- 不为没有Redis依赖的fixture人为启动Redis；只验证真实声明的外部资源。

## 最终验收

- 用户源码只有package一种编译单元；service artifact/config无用户source files。
- 同一package可直接本地链接，也可被一个或多个service deployment选择。
- package call保留Local Code ABI；service call统一Boundary ABI且本轮全部进程内执行。
- 多runtime replica可水平扩CPU/内存，外部数据层按配置共享。
- code/deployment identity、state/config owner、callback/recoverable lifetime无歧义。
- 无双parser/projector/identity/dispatcher、超长文件职责回退或测试只改字段名的情况。
- 全量、live和必要chat smoke有可复现证据。

## 细化原则

跨仓库迁移按可验收服务族拆任务，不把所有fixture交给一个Agent。任何迁移发现核心contract不
成立时，回到对应架构/实现阶段建立独立前置任务；不能在生态fixture里加特例掩盖。
