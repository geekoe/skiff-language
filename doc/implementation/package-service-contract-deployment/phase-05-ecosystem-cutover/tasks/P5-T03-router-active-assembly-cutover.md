# P5-T03：Router Active-Assembly / Host Ingress Cutover

## 权威输入与DAG

- 设计：`doc/architecture/package-service-contract-deployment.md` §2.6、§6.2、§10、§12–§15。
- 依赖：R01 PASS exact checkpoint；与T02/T04/T05同级，解锁R02。
- 风险：高；public ingress、reload atomicity、replica dispatch。
- branch：`codex/p5-t03-router-active-assembly`。worktree：`/Users/geek/workspace/skiff-p5-t03-router`。
- 当前共享状态是R01 PASS的implementation checkpoint；完成后仍只是R02 batch candidate。使用新的开发
  Agent；证据对T01 wire、router production/tests/config或依赖变化失效。
- 五分钟内编辑production文件；T01 control fixture不足时回报，不自行扩wire。

## 写入范围

独占 `router/**`。不改Rust runtime、scripts/compiler/test-runner、T01 golden fixture语义或verify注册。

## 完成态

1. router startup/reload只读environment active-assembly pointer及T01 typed records，构建immutable active
   snapshot；旧service pointer/manifest/serviceAssembly projection不参与production。
2. control广播只携带exact environment/generation/assembly ref；concurrent reload合并，candidate失败
   保留旧snapshot/control generation。
3. runtime registry按exact assembly identity/generation管理healthy replicas；同assembly多个replica可共享
   新请求，不按service/build/target/protocol建立分离activation registry。
4. HTTP/WS gateway用request Host/method/path直接匹配RuntimeAssembly global ingress；相同path的不同
   Host可并存。`X-Skiff-Service`/`X-Skiff-Version`/`X-Skiff-Release`/query selector/
   rewrite-to-service不再选择target，也不作fallback。
5. request envelope携带assembly generation与canonical ingress/contract operation identity；router不读provider
   package/display target，不恢复Phase 04已退役service relay。
6. health暴露active assembly/generation、replica identity/state/in-flight；不将per-service registrations伪装为
   assembly health。

## 最早探针与唯一验证 owner

- Host collision fixture：Codex/AIHub同`GET /v1/models`、AIHub/Agine同`/ws`按Host唯一选中。
- legacy header/query/rewrite mutation不能改变target；缺失/未知Host fail closed。
- 两replica轮询/故障摘除、stale generation register拒绝、failed reload保留旧dispatch。

```bash
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- \
  tests/active-assembly-reload.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/host-ingress.test.ts
git diff --check
```

不跑全量router selector/verify。提交一个commit并合入Skiff integration branch，回报snapshot/registry/gateway symbol、旧选择器反向
搜索、动态探针与自验收矩阵。
