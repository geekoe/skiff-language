# P5-T12：Agine Package / Deployment / Clients / Smoke Cutover

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §3–§12、§14–§15。
- 依赖：R09 PASS exact checkpoint及T07 exact `skiff-packages` integration；与T10/T11并行，解锁T09E。
- 风险：高；Agine API owner split、AIHub stream consumer、HTTP/WS clients、chat证据。
- branch：`codex/p5-t12-agine-clients`；worktree：`/Users/geek/workspace/internals-p5-t12-agine`。
- 当前共享状态是R09 PASS的contract/workflow checkpoint；完成后仍是Wave 3 partial candidate。使用新的
  开发Agent；证据对Agine/AIHub contracts、shared workflow、owned source/deployment/clients/tests变化失效。
- 五分钟内修改package/source/API/deployment或client Host owner；不改T09C contract/T09D workflow。

## 写入范围与完成态

独占 `internals/agine/**`，排除T09C contract、T09D已冻结的package scripts。

1. 旧service source root迁为Package root，public API显式导出HTTP/WS contract操作可映射callables；
   `AgineSocket` type不再被当作隐式service operation owner，删除`service.yml`。
2. contract-owned public request/event types与service-local `root.api.agine` models通过聚焦wrapper转换；不机械
   替换61个文件的内部model引用，也不让package nominal type进contract。
3. Agine只依赖AIHub ServiceContract编译managed LLM/web search/provider catalog calls，不读AIHub
   package/deployment；stream/cancel/context沿Phase 04 canonical binding。
4. deployment映射所有contract operations，绑定12个service DB objects、transitive package requirements、
   config/secret/resource/policy，ingress使用`agine.localhost`。
5. web client/host/E2E HTTP/WS URL只使用Agine Host，删除service/version query/header、`keepRouterQuery`
   等fallback；不恢复旧Agine gateway。
6. chat smoke在WS建立后先执行真实`provider/list`并断言`aihub/gpt-5.5`，再完成
   session/create/chat/send/get业务链。开发任务只修脚本与静态测试，真实stable smoke归V01。
7. `agine/client/e2e/provider-list-smoke.mjs`提供独立main-only live入口；其self-test与chat smoke self-test
   使用fake transport验证Host URL和最终断言，不连接stable。
8. owned client/host AGENTS与README删除service/version query/header、旧gateway/rewrite说明，保留动态
   worktree端口与浏览器约束并改为`agine.localhost`；不新建README。

## 唯一聚焦验证 owner

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-t12-cargo.XXXXXX)"
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  npm --prefix agine/service run type-check
CARGO_TARGET_DIR="$P5_CARGO_TARGET" SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  npm --prefix agine/service test
npm --prefix agine run type-check:client
npm --prefix agine run type-check:host
npm --prefix agine/client run test:logic
node --test agine/client/e2e/provider-list-smoke.test.mjs
node --test agine/client/e2e/api.chat-smoke.test.mjs
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
git diff --check
```

不运行build/dev/start/stable/chat smoke。提交一个commit并合入Internals integration branch，回报operation mapping、model wrapper、state/
dependency binding、Host route反向搜索、smoke最终断言及自验收矩阵。
