# P5-F433A Current WebSocket fixture and oracle convergence

状态：Ready。中风险fixture/tooling迁移。

## 直接父节点

- `P5-F433-d4-current-residue-wave.md`

父节点冻结旧source命中闭集；F424A result拥有connect-only fixture终态。

## 写入范围

只允许：

- `test-runner/fixtures/package-service-{websocket-smoke,websocket-generation-a,websocket-generation-b,i02-spawn-submit}/**`
- `test-runner/tests/package_service_contract_deployment.rs`
- `scripts/tests/package-service-i02-combined.test.mjs`
- 因四fixture真实build/identity变化而必须机械更新的：
  - `scripts/lib/package-service-{ecosystem-smoke-oracle,i02-combined-real}.mjs`
  - `scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs`
  - 直接`package-service-*.test.mjs`
  - `cross-system-fixtures/package-service-ecosystem/**`
- 本leaf result

禁止修改test-runner production、Runtime/Router/compiler/std、非package-service tooling、
Internals或skiff-packages。若需要新的production owner，返回`TASK_SCOPE_EXPANDED`。

## 必须实现

1. 四个目录改为current `kind: test` service fixture，并用strict singleton `service.yml.websocket`
   引用private connect callable；authoring不含id/routes/receive/message/context。
2. private connect callable只接受
   `std.websocket.WebSocketConnectRequest`和`connectionId: string`，返回non-generic
   `std.websocket.WebSocketConnectResult`；accept无Context，可用connectionId作为business identity。
3. `api.yml`不公开connect callable；保留各fixture原本真实业务public marker/submit surface。
4. 删除receive branch、client-message echo和`sendTextToConnection(event.receiveEvent...)`。这些
   package/source fixture只证明current authoring/compile/identity；真实HTTP上行触发下行由后继
   combined probe拥有。
5. `ecosystem_http_fixture_uses_two_gateway_entries_without_ws_compat`的inline package仍应是纯HTTP
   zero-operation fixture：直接删除旧WS API/source，不为它伪造service entry。
6. I02和ecosystem script assertions改为current private connect、无旧API export/receive；不能仅
   放宽regex。
7. 通过真实生成命令更新确因source/service/identity变化的package/deployment/assembly/generation
   oracle；不得手工保留旧digest或改无关baseline。
8. completion反搜中旧generic/receive/context spelling为零；current connect type和
   `websocket.connectRequest/connectionId`必须正向命中四fixture。

## 验证

本Agent是以下聚焦证据的唯一owner：

```bash
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  ecosystem_http_fixture_uses_two_gateway_entries_without_ws_compat
cargo test -p skiff-test-runner --test package_service_contract_deployment \
  i02_submit_probe_is_private_http_gateway_not_service_operation
node --test scripts/tests/skiff-source-test-suite.test.mjs
node --test scripts/tests/package-service-i02-combined.test.mjs
node --test scripts/tests/package-service-*.test.mjs
cargo fmt --all -- --check
git diff --check
```

若最后一个Node glob重复前两条，可记录实际discovery但不得省略其余package-service owner。不要运行
stable/live或完整N5。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f433a-websocket-fixtures`
- 分支：`codex/p5-f433a-websocket-fixtures`

启动后5分钟内实际修改。提交implementation，再新增并提交
`P5-F433A-current-websocket-fixture-convergence-result.md`。返回commit/tree、生成identity差异、
测试discovery和clean状态。不得merge、rebase、push、stable/live或承接combined。
