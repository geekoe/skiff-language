# P5-T09A：Codex Relay Code-Free Contract

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §4–§8、§10、§14。
- 依赖：R02 exact Skiff checkpoint；与T09B/T09C并行，解锁T09D。
- 风险：高；public contract ABI/schema。
- branch：`codex/p5-t09a-codex-contract`；worktree：`/Users/geek/workspace/internals-p5-t09a-codex-contract`。
- 当前共享状态是R02 PASS的contract checkpoint输入；完成后只解锁T09D合流。使用新的开发Agent；
  证据对Skiff contract schema/CLI、该contract/schema fixture/tests变化失效。
- 五分钟内新增code-free contract authoring；不允许从provider source动态生成已发布contract。

## 写入范围与完成态

只写 `internals/codex-relay/service/contract.yml`、独立schema fixture/聚焦验证。不改Skiff source、
`service.yml`、package/deployment、client或共享scripts。

1. contract自包Codex Relay的外部HTTP operations及AIHub真实消费的
   `responsesCompletedResult`；未被production消费的`responsesCompleted`不进新contract。
2. operation stable keys、params/return/error/stream/cancel/value plans与boundary schema全部由contract拥有；
   无provider package/build/route/config/deployment字段。
3. schema closure不引用`llm-api`/`llm-providers` package nominal identity；需要的对外结构在contract中
   定义，实现wrapper留给T10。
4. contract可在删除/隐藏Codex implementation package时独立build/publish，identity稳定。

## 唯一验证 owner

```bash
P5_ARTIFACT_ROOT="$(mktemp -d /tmp/skiff-p5-t09a.XXXXXX)"
P5_SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration
SKIFF_ROOT="$P5_SKIFF_ROOT" node "$P5_SKIFF_ROOT/scripts/skiff.mjs" contract build codex-relay/service \
  --artifact-root "$P5_ARTIFACT_ROOT" --json
node --test codex-relay/service/contract.test.mjs
git diff --check
```

提交一个commit并合入Internals integration branch，回报operation/schema表、deleted-operation disposition、provider隐藏正例及自验收矩阵。
