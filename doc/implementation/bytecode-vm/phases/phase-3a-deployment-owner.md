# Phase 3A: exact-build deployment owner cut

状态：planned；依赖Phase 2 complete

## 1. 目标

先把Runtime执行身份从provider-dependent synthetic `RuntimeAssembly`收敛到exact deployment `buildId`。
本阶段允许image内部暂时持有legacy tree program，但service provider不再成为consumer image closure或identity。

## 2. 交付物

1. `DeploymentExecutionImage` cache/critical section以exact deployment buildId为唯一key。
2. Image只闭合consumer deployment及其Package-local code/type/config/capability facts。
3. Service dependency slot只保存contract coordinate；每次boundary invocation解析release pointer并pin exact
   provider image。
4. 移除`register_closure`式“同一image注册给多个deployment buildId”行为。
5. Request/stream/callback owner pin不引用ambient generation或全局active assembly。
6. Test fixture与Live harness使用production per-build loader；`ReleaseBundle`如存在只作离线清单。

## 3. 非目标

- 不在本阶段执行bytecode或实现semantic verifier。
- 不把全部legacy artifact/reader立即删除；它们只服务尚未迁移的显式legacy execution lane。
- 不实现跨service atomic generation snapshot。

## 4. 验收

### 4.1 Focused gate

```bash
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only tooling
git diff --check
```

Loader、authoring、release pointer、routing和test fixture的exact-build tests必须同轮通过。

### 4.2 阶段专属证明

必须有真实provider A/B和consumer C场景：

1. C首次加载时provider pointer指向A；记录C image identity/content和A owner。
2. pointer原子更新到B，但C image不重建、不改变。
3. 已开始invocation/stream/callback继续pin A；更新后的新invocation使用B。
4. A/B任一加载失败不会发布半image或污染C cache。
5. 两个并发waiter共享同一次load attempt，并观察同一成功或同一失败；后续新请求才可开始新attempt。

还必须反向证明：loaded set/capability advertisement不因consumer load而虚假宣称provider build已加载；request
identity、health和telemetry没有assembly generation作为执行权威。

### 4.3 阶段专属 Live

运行与本阶段owner/routing相关的managed fixture，并要求manifest列出consumer/provider各自buildId和image pin：

```bash
node scripts/verify.mjs --only router-live:bootstrap
node scripts/verify.mjs --only router-live:session
node scripts/verify.mjs --only router-live:dispatch
node scripts/verify.mjs --only router-live:agine
```

chat与host-tools仍可执行legacy tree program，但必须通过新的per-build image owner和invocation-time provider
resolution；manifest明确标注engine为legacy，不能误报VM。

## 5. 停止条件

- consumer image内容或identity取决于load瞬间的provider pointer。
- 一个`Arc<ActiveAssembly>`仍被登记到多个deployment buildId。
- provider address被patch进consumer的persistent/immutable image。
- 为保持旧测试而保留test-only assembly admission或generation fallback。

## 6. Handoff

Phase 3B只能在本阶段per-build owner上构造linked bytecode image；不得重新引入跨service closure。
