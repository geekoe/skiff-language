# P5-F350 External ingress ecosystem migration audit

状态：Ready（只读，跨仓库）。

## 直接父节点

- `P5-H35-external-ingress-surface-separation.md`
- 真实service批次：`P5-H33-c2-real-service-revalidation-batch.md`

## 目标

只读盘点Skiff、skiff-packages和Internals中所有HTTP/WebSocket external ingress authoring，区分真正
service-call API与external-only handler，形成后续按service并行迁移清单。

必须回答：

1. 每个service的`api.yml`中哪些function只因HTTP/WebSocket ingress而公开，哪些仍被service dependency
   调用，不能误删。
2. 当前`service.yml`使用`operation`、`handler`或其它旧shape的精确范围。
3. HTTP raw/typed/stream及WebSocket connect/receive各有哪些真实consumer、测试和workflow。
4. 移除external-only API后各ServiceContract预期operation数变化；Registry、Account、Relay、AIHub、
   Agine及官方packages分别受何影响。
5. 哪些consumer依赖同一compiler/runtime缺口，必须等待shared checkpoint；哪些service目录可以并行迁移。
6. F269已有Internals测试服务迁移怎样保存，哪些证据会失效；不得修改或干扰其worktree。

## 范围与写入

只读检查：

- `/Users/geek/workspace/skiff-phase-05-integration`
- `/Users/geek/workspace/skiff-packages-phase-05-integration`
- `/Users/geek/workspace/internals-phase-05-integration`

若某integration worktree不存在，记录实际仓库/分支状态并只读使用可用候选。不得修改任何production、test、
service manifest或lockfile。

只允许在本Skiff审计worktree新增：

- `P5-F350-external-ingress-ecosystem-migration-audit-result.md`

result记录三个仓库exact commit/tree、逐service表格、共享blocker、可并行迁移批次和验证入口。
不运行build/stable/live，不push。提交result并返回commit。

## Worktree

- `/Users/geek/workspace/skiff-p5-f350-ingress-ecosystem-audit`
- `codex/p5-f350-ingress-ecosystem-audit`
