# P5-T10：Codex Relay Package / Deployment / Host Cutover

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §3–§12、§14–§15。
- 依赖：R09 PASS的exact Internals contract/workflow checkpoint；与T11/T12并行，解锁R03。
- 风险：高；HTTP ingress、AIHub service-call provider、DB/config/secret owner。
- branch：`codex/p5-t10-codex-relay`；worktree：`/Users/geek/workspace/internals-p5-t10-codex-relay`。
- 当前共享状态是R09 PASS的contract/workflow checkpoint；完成后仍是Wave 3 partial candidate。使用新的
  开发Agent；证据对Codex contract/shared workflow、owned source/package/deployment/client/tests变化失效。
- 五分钟内修改`service/package.yml`/source/API/deployment中一个真实owner；不改contract或共享workflow。

## 写入范围与完成态

独占 `internals/codex-relay/**`，但T09A的contract及T09D声明的共享workflow文件不得修改。

1. 旧service source root成为明确Package root，`package.yml/api.yml`公开所有deployment需要的HTTP
   callable及`responsesCompletedResult` provider callable；`service.yml`删除，无adapter读。
2. implementation signature直接使用Codex contract types；内部`llm-api`/`llm-providers` model通过显式
   wrapper转换，不以package nominal type满足contract。
3. `deployment.yml`将每个contract operation恰好映射一次，ingress使用唯一
   `codex-relay.localhost` Host；no-op `relay.routePre`及未使用`responsesCompleted`旧操作不保留。
4. 7个service DB objects、package transitive state/config/runtime requirements、admin/codex secret refs、timeout/
   resource/activation policy有唯一deployment binding；secret value不进artifact。
5. admin/config/login/importer只使用Host URL，删除`service.yml` identity reader、query selector、
   `x-skiff-service/version` header producer；不改OAuth 1455单例语义。
6. source/package/deployment可在AIHub implementation不存在时build/validate；内部service call不经router。

## 唯一聚焦验证 owner

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
  node scripts/test-isolated-service.mjs agine.ai/codex-relay
node --test codex-relay/admin/*.test.mjs codex-relay/lib/*.test.mjs codex-relay/scripts/*.test.mjs
node --check codex-relay/admin/*.mjs codex-relay/lib/*.mjs codex-relay/scripts/*.mjs
git diff --check
```

不操作stable/OAuth/live upstream。提交一个commit并合入Internals integration branch，回报operation mapping、contract wrapper、state/config/secret
binding、route selector反向搜索、测试及自验收矩阵。
