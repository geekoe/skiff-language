# P5-F41A：R05 Unary Client Repair

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点F41A，依赖R05在production candidate `c808586546fddc5550f1caf7e520e849162a0946`上的精确FAIL：
真实A/B transcript已通过A旧连接×2与B新连接marker，但generation B unary `POST /probe`返回404，导致drain
oracle未执行。当前证据只支持generation lifecycle harness implementation owner，不支持Router production修复。

## 写入范围与目标

允许修改：

- `scripts/lib/package-service-generation-lifecycle-smoke-real.mjs`
- `scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs`
- 若复用现有canonical HTTP client helper确有必要，可窄改或新增单一`scripts/lib/` helper及其直接test；必须先证明
  当前仓库没有可复用owner。

必须静态闭合真实unary请求的端口、path、wire `Host` header及selector来源，使它与同一run中已激活的B assembly
HTTP ingress一致；不得用retry、fallback、hardcoded stable端口、fake Router响应或绕过production ingress修复404。
失败diagnostic必须有界保留实际method/URL/wire Host/status及脱敏、限长response body，使下一次真实失败能确定
harness还是Router owner。direct regression不得继续用忽略request参数且固定返回200的fake掩盖wire错误；必须精确断言
outbound request并覆盖404 diagnostic和成功B marker。

禁止修改fixtures、Router、Runtime、compiler、store、deployment、artifact schema、公共ABI、activation/release/四对象
语义或既有single-generation helper。若静态证据证明正确请求仍会由Router返回404，立即返回
`TASK_NOT_EXECUTABLE`及最小Router owner，不得越界修复。

## 验证与集成

开发owner只运行：

```bash
node --check \
  scripts/lib/package-service-generation-lifecycle-smoke-real.mjs \
  scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs

node --test scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs
```

禁止运行真实transcript、旧smoke、I31/I32 combined、Router/runtime/instance、stable或完整gate。提交必须包含反向搜索、
失败归因、未决问题及自验收矩阵。

从当前integration HEAD创建独立worktree/branch；不push、不merge main、不操作允许的integration untracked ledger。
首次实际修改须在启动后5分钟内发生，否则返回`TASK_NOT_EXECUTABLE`。完成后仍是implementation checkpoint，只解除
全新I32 cheap combined。

F41A direct evidence因lifecycle real client/test或其复用helper变化失效；I31 author/store evidence不因本窄修复失效。
I32必须在合流状态上覆盖outbound wire与完整orchestration direct tests，不执行真实transcript；I32 PASS后才建立新R05A
候选。
