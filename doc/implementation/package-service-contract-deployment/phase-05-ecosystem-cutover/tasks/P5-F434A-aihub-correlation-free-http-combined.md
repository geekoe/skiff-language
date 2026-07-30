# P5-F434A AIHub correlation-free HTTP stream combined

状态：Ready。中高风险只读combined owner。

## 直接父节点

- `P5-F428B-aihub-http-correlation-service-result.md`
- `P5-F428C-aihub-http-correlation-client-result.md`
- `P5-F427B-aihub-http-correlation-owner-audit-result.md`
- `P5-F433A-current-websocket-fixture-convergence-result.md`

F428B/C冻结service/client实现；F427B冻结wire与identity矩阵；F433A及F432A解除此前
test-runner compile遮挡。引用链继续到唯一权威设计。

## 精确候选与职责

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff | `d7253103983cf4f08264d12476fe7f8d54887652` | `75e48f3454754e06d9fe849e83690381b226419b` |
| Internals | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` |

本节点只验证同一merged candidate；唯一允许写入是Skiff中的本leaf result。不得修source、test、
fixture、receipt owner或tooling。失败时返回`COMBINED_FAIL`及最小owner，不顺手实现。

## 必须验证

1. whole `aihub/**` production/test/docs对
   `request_id|requestId|runId|runIdFromRequestId|correlationId|correlation_id`反搜为零；
   若显式negative fixture存在须与production分区。
2. AIHub client完整18项stream suite及syntax通过，精确POST body只有业务字段，parser只接受
   `{type,seq,event}`，Abort/cancel与nested provider/tool IDs不回归。
3. AIHub service source/receipt/package-store/workflow guard、type-check和全部non-live tests在本
   Skiff checkout真实执行；live Gemini fixture保持`defaultRun false`且运行次数为零。
4. 在隔离临时store/build root生成PackageArtifact、ServiceContract、ServiceDeployment和
   RuntimeAssembly，并按F427B第6节逐项比较：
   - `ServiceProtocolIdentity`、五个operation ID、
     `PackageSchemaIndexIdentity`、`PackageLocalAbiIdentity`、七个GatewayEntryIdentity、
     keys/selectors均保持冻结值；
   - Package build/ref、deployment revision/identity、assembly identity因correlation-free source
     真实改变。
5. 生成图exactly五个service-call operation；`/v1/chat/events`与`/chat/events`是raw HTTP
   server stream；AIHub WebSocket entry为零。
6. 使用现有fake/test-double完成一次isolated HTTP stream combined：
   - request-start seq0、provider items原序、finish后`[DONE]`；
   - post-start error保留已发items、使用next seq、无`[DONE]`；
   - pre-start error是有限JSON且无correlation字段；
   - cancel/disconnect终止provider work。
7. provider/service-call保护面相对F428前无diff；不访问stable、live、真实provider或固定端口。

## 命令与证据

至少执行父审计第8节命令：

```bash
node --test aihub/client/*.test.mjs
node --check aihub/client/app.js
node --check aihub/client/chat-stream.mjs
node --check aihub/client/chat-stream.test.mjs
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run test:service-api
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run test:package-store
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run test:workflow-guards
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service run type-check
SKIFF_ROOT=<assigned-skiff-worktree> npm --prefix aihub/service test
git diff --check
```

记录每条命令discovery/pass/fail/skip、临时root与identity对照。若上游失败，停止其遮挡的后续动态
证明并做一次failure classification。

## Worktree与交付

- Internals：`/Users/geek/workspace/internals-p5-f434a-aihub-combined`
- 分支：`codex/p5-f434a-aihub-combined`
- Skiff/result：`/Users/geek/workspace/skiff-p5-f434a-aihub-combined`
- 分支：`codex/p5-f434a-aihub-combined`

新增并提交`P5-F434A-aihub-correlation-free-http-combined-result.md`。返回result commit/tree、
verdict、identity矩阵和两个clean状态。不得修改Internals或Skiff production、merge、rebase、
push、stable/live；完成后不得承接repair。
