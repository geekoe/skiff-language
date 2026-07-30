# P5-F434A AIHub correlation-free HTTP stream combined result

状态：`COMBINED_FAIL`。

静态边界、AIHub client 18 项 stream suite、三项语法检查，以及 service receipt/store/workflow
前置 suite 均通过。真实 canonical `type-check` 在 AIHub Package publish 的 expression-type
model validation 阶段失败。按任务的上游失败停止规则，本文没有继续执行完整 service test、
generated identity comparison 或 isolated fake-provider combined，也没有修改任何
source、test、fixture、receipt owner 或 tooling。

最小 failure owner 是 **Skiff compiler source expression-type model owner**，具体位于
`compiler/source/src/expression_type_model.rs` 的
`materialize_target_typed_object_literal`：三个带显式 `-> Json` 返回类型的本地 generic
`encodeJson<T>(...)` call，在 target-typed `JsonObject` return literal 中没有生成 resolved
field expression type。本文不承接 repair。

## 1. 精确输入与隔离

| 输入 | Checkout / branch | Commit | Tree | 起始状态 |
| --- | --- | --- | --- | --- |
| Skiff frozen production candidate | `/Users/geek/workspace/skiff-p5-f434a-aihub-combined` / `codex/p5-f434a-aihub-combined` | `d7253103983cf4f08264d12476fe7f8d54887652` | `75e48f3454754e06d9fe849e83690381b226419b` | clean |
| Skiff task dispatch HEAD | 同上 | `b7f372fde39994cc92dc2089ed97a95e8d3ad4c6` | `7fbed774cc4cef2cf8314e719adc232956949f10` | clean；相对 frozen candidate 只新增本任务文件 |
| Internals exact candidate | `/Users/geek/workspace/internals-p5-f434a-aihub-combined` / `codex/p5-f434a-aihub-combined` | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` | clean；与任务冻结值精确一致 |

隔离总 root：

```text
/private/tmp/p5-f434a-aihub-combined.lPKF1c
```

动态命令统一使用：

```text
TMPDIR=/private/tmp/p5-f434a-aihub-combined.lPKF1c/tmp
CARGO_TARGET_DIR=/private/tmp/p5-f434a-aihub-combined.lPKF1c/cargo-target
NPM_CONFIG_CACHE=/private/tmp/p5-f434a-aihub-combined.lPKF1c/npm-cache
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f434a-aihub-combined
```

canonical workflow 创建的 store 是上述 `TMPDIR` 下的
`internals-canonical-assembly-*/ecosystem-store`；失败返回后 workflow 已自动删除该 store。
交付前又删除整个可重建临时 root，包括约 2.0 GiB 的隔离 Cargo target 与日志。

## 2. 反向搜索与保护域

对 tracked `aihub/**` 执行大小写不敏感搜索：

```text
request_id | requestId | runId | runIdFromRequestId |
correlationId | correlation_id
```

结果为 **0 match**。production、test、fixture 和 docs 都没有命中，因此不存在需要从 production
分区说明的 explicit negative-name fixture。

相对 F428 前 exact input `ed5d333b2406d5375fca8acc96f4695667c48ced`：

| 保护面 | 结果 |
| --- | --- |
| `packages/llm-api/**` | 0 diff |
| `packages/llm-providers/**` | 0 diff |
| `aihub/service/internal/managed_provider_transport.skiff` | 0 diff |
| `codex-relay/**` | 0 diff |
| AIHub `package.yml` / `api.yml` / `service.yml` / `service-api-receipt.mjs` | 0 diff |

因此 provider protocol、selected service-call surface、gateway authoring、keys/selectors 和 receipt
owner 没有被 correlation repair 改写。Internals `git diff --check` 通过。

## 3. 命令账与停止点

| 命令 / probe | Discovery | 结果 |
| --- | ---: | --- |
| whole `aihub/**` correlation reverse search | 1 tracked-tree query | PASS，0 match |
| `node --test aihub/client/*.test.mjs` | 18 | PASS，18 pass、0 fail、0 skip |
| `node --check aihub/client/app.js` | 1 file | PASS |
| `node --check aihub/client/chat-stream.mjs` | 1 file | PASS |
| `node --check aihub/client/chat-stream.test.mjs` | 1 file | PASS |
| `npm --prefix aihub/service run test:service-api` | 8 | PASS with expected skip：7 pass、0 fail、1 generated-receipt-owner skip |
| `npm --prefix aihub/service run test:package-store` | 2 | PASS，2/2 |
| `npm --prefix aihub/service run test:workflow-guards` | 13 | PASS，13/13 |
| `npm --prefix aihub/service run type-check` | canonical full fixture；进入 AIHub source validation | **FAIL**，exit 1；见第 4 节 |
| `npm --prefix aihub/service test` | command discovered | `SKIP_AFTER_UPSTREAM_FAIL`；会重走同一 canonical publish，并被已证实首错遮挡 |
| generated identity / graph comparison | task requirement discovered | `SKIP_AFTER_UPSTREAM_FAIL`；AIHub records 未生成 |
| isolated HTTP stream combined | task requirement discovered | `SKIP_AFTER_UPSTREAM_FAIL`；不得把 source fixture 当作已执行 |
| live Gemini | 1 live declaration，`test defaultRun false` | NOT RUN，运行次数 0 |

client 18 项 suite 已实际证明 exact POST body 只有业务字段、parser 接受 canonical
`{type,seq,event}`、nested reasoning/text/tool IDs 保留、terminal 与 finite-buffer fail-closed，
以及 reader cancel / `AbortError` 语义。service receipt suite 的 supervised ancestor 静态 oracle
也通过；但 49 项 `aihub_service` 和 1 项 `managed_provider_transport` non-live Skiff test 没有进入
动态执行，不能据此声称 service 或 combined PASS。

## 4. Failure classification

canonical workflow 已完成 test-runner 编译并进入真实 AIHub package source validation；这证明
F432A/F433A 的旧 test-runner/fixture compile 遮挡已解除。AIHub publish 随后返回精确三项：

```text
error: contract validation failed:
expression type model failed:
- internal.aihub_service: return object literal field `event` has no resolved expression type at 2180:12
- internal.provider_catalog: return object literal field `reasoningLevels` has no resolved expression type at 123:22
- internal.provider_catalog: return object literal field `reasoning_levels` has no resolved expression type at 124:23
```

分类：

```text
UPSTREAM_CANONICAL_PACKAGE_PUBLISH_FAILURE
  stage: build-service-packages / AIHub source contract validation
  subsystem: Skiff compiler source expression-type model
  minimum owner:
    compiler/source/src/expression_type_model.rs
    materialize_target_typed_object_literal
  downstream blocked:
    AIHub PackageArtifact / ServiceContract / ServiceDeployment
    RuntimeAssembly
    generated graph and identity comparison
    all non-live Skiff service tests
    isolated HTTP stream combined
```

这不是 F428 correlation repair 新增的 diagnostic：

- F425B 与 F425D 已在更早 exact candidates 记录同一组三项 failure；
- `streamEnvelope` 的 `event: encodeJson(event)` expression 在 F428 前已经存在，correlation repair
  只删除 outer request/run fields；
- `provider_catalog` 两个 source sites 不在 F428 写集；
- 两个 module 的 local helper 都显式声明 `function encodeJson<T>(...) -> Json`。

因此本文把首因收敛到 compiler expression fact retention，而不把三个 source site、测试期待或
receipt owner当作 combined repair。修改任何一侧都需要独立授权的新 repair leaf。

## 5. Identity 与 generated graph 矩阵

| Record / identity | F427B 冻结预期 | 本次 generated 结果 | Verdict |
| --- | --- | --- | --- |
| `ServiceProtocolIdentity` | 不变 | AIHub `ServiceContract` 未生成 | BLOCKED / NOT COMPARED |
| 五个 `ContractOperationId` | 全部不变 | contract graph 未生成；静态 receipt oracle仍锁定精确五项 | BLOCKED / NOT COMPARED |
| `PackageSchemaIndexIdentity` | 不变 | AIHub `PackageArtifact` 未生成 | BLOCKED / NOT COMPARED |
| `PackageLocalAbiIdentity` | 不变 | AIHub `PackageArtifact` 未生成 | BLOCKED / NOT COMPARED |
| 七个 `GatewayEntryIdentity` | 全部不变 | deployment 未生成；authoring与 F428 前为 0 diff | BLOCKED / NOT COMPARED |
| 七个 gateway keys / ingress selectors | 全部不变 | source authoring 0 diff，generated deployment 未生成 | SOURCE PASS / GENERATED BLOCKED |
| `PackageBuildId` / immutable ref | 必须变化 | source validation 先于 artifact write 失败 | BLOCKED / NO RECORD |
| `ServiceDeployment` revision / identity | 必须变化 | deployment 未生成 | BLOCKED / NO RECORD |
| `RuntimeAssembly` identity | 必须变化 | assembly 阶段未执行 | BLOCKED / NO RECORD |

同理，静态 authoring/receipt oracle表明 selected API 仍为五个 service-call operation、两条 events
entry 仍为 raw HTTP server stream，且 AIHub `service.yml` 没有 WebSocket authoring；但生成图未产生，
所以不能把“exactly five”或“generated WebSocket entry 为零”伪报为动态证明。

## 6. 禁令与交付

- 没有运行 `build`、`dev`、`start`、stable watch/reload、router/runtime/instance、固定端口或
  MongoDB。
- 没有运行 live fixture、真实 Gemini 或任何真实 provider。
- 没有 merge、rebase 或 push。
- Internals 没有任何写入；Skiff 唯一新增文件是本文。
- 失败后没有修 source/test/fixture、扩大 scope 或承接 repair。

result-only commit/tree 与两个最终 clean 状态由交付消息记录。
