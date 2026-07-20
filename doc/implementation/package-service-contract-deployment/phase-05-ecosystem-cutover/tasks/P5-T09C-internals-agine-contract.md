# P5-T09C：Agine Code-Free Contract / API Owner Split

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §4–§8、§10、§14。
- 依赖：R02 exact Skiff checkpoint；与T09A/T09B并行，解锁T09D。
- 风险：高；Agine public HTTP/WS contract与现有service-local API范围较大。
- branch：`codex/p5-t09c-agine-contract`；worktree：`/Users/geek/workspace/internals-p5-t09c-agine-contract`。
- 当前共享状态是R02 PASS的contract checkpoint输入；完成后只解锁T09D合流。使用新的开发Agent；
  证据对Skiff contract schema/CLI、该contract/schema fixture/tests变化失效。
- 五分钟内新增code-free contract authoring；不机械复制607个`root.api.agine.*`内部引用。

## 写入范围与完成态

只写 `internals/agine/service/contract.yml`、contract-owned schema fixture/聚焦验证。不改service
implementation、package/deployment、client/host、package scripts或共享workflow。

1. contract只收录真实外部HTTP/WS操作及其closed schema，不把整个service-local
   `root.api.agine` 命名空间当public aggregate。
2. HTTP request/session/chat与WS connect/receive/event的params/return/error/stream/cancel/value plan精确；
   `AgineSocket` type本身不代替callable operations。
3. contract schema不引用implementation package-local nominal identity或provider deployment。T12通过显式wrapper
   在contract types与内部model间转换。
4. 隐藏Agine implementation source/artifact后contract可独立build/publish。

## 唯一验证 owner

```bash
P5_ARTIFACT_ROOT="$(mktemp -d /tmp/skiff-p5-t09c.XXXXXX)"
P5_SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration
SKIFF_ROOT="$P5_SKIFF_ROOT" node "$P5_SKIFF_ROOT/scripts/skiff.mjs" contract build agine/service \
  --artifact-root "$P5_ARTIFACT_ROOT" --json
node --test agine/service/contract.test.mjs
git diff --check
```

提交一个commit并合入Internals integration branch，回报public operation/closure表、内部API disposition、provider隐藏正例及自验收矩阵。
