# P5-F271 Container projection heap-cycle precision result

结论：实现完成。`09e0dc9` 是本任务的中间实现，不是最终正确性证据：它在 artifact
边界把结构化 projection 重新折叠成 caller 参数 root，并漏掉了
`Parent -> Child -> Parent` 的真实回边。当前提交取代该实现，同时关闭这两个不健全点。

## 最终语义

- Package callable provenance 新增稳定的
  `CallerParameterProjection { index, path }`。path 只有字段名和
  `containerElement` 两种结构化 step，最多 64 层；空 path、空白字段名、未知字段和超限
  输入均 fail closed。源码位置、表达式序号和 callable 名不进入 path。
- summary 分成两层：`returnOrigins` 记录从返回值可达的全部 origin，
  `directReturnOrigins` 只记录返回值自身可能指向的 root。两者分别参与 summary replay
  和 fixed-point join。
- fresh 容器 root 与其 payload 中可达的 caller root 分开保存。字段或容器元素读取恢复
  对应 projection；当前 field-insensitive heap 只把接收者的一层直接 payload 作为保守
  direct 候选，不会把任意深度可达对象提升成字段本身。
- `Map.get`、`JsonObject.get`、字段读取以及 Array / Map / JsonObject 写入都保留结构化
  heap edge。helper 的参数写入和返回 summary 在跨模块、跨 Package replay 后仍保留该
  结构。
- cycle 检查沿 caller / fresh edge 遍历，并按同一 root 的祖先路径识别回边。普通
  “取出、修改、写回”和 fresh wrapper 不再误报；直接 self-cycle 与
  `Parent -> Child -> Parent` 间接 cycle 仍报 `UnsupportedHeapStore`。
- 控制流合并后的 direct root 若可能同时为 fresh 和 caller-owned，仍按 caller write
  保守处理。canonical detached boundary 只接受 direct root 全部为 Fresh / Constant
  的 fresh wrapper；条件分支中的 direct caller root 不能借 fresh 分支通过。
- projection path 和 `directReturnOrigins` 都计入 Package build identity；仅改变这些
  实现事实不会改变 Local ABI 或 service protocol identity。

## 回归覆盖

- 本地与跨 Package 的 Map element 修改后写回。
- 本地与跨 Package 的 fresh wrapper root / payload 分离，以及再次读取 payload 后恢复
  caller identity。
- AIHub 形状：fresh `JsonObject` 保存 caller array element，再局部修改并放入 fresh
  output array。
- Relay 形状：fresh state 传入 helper，helper 的 parameter-store replay 不产生伪 cycle。
- llm-api 两种 `materializeCompletedResult` 形状。
- direct cycle、helper field 间接 cycle、projection path 上限和条件 fresh / caller
  root 的负例。
- artifact 严格 JSON wire、必需的 `directReturnOrigins`、确定性排序，以及
  projection/direct-only 变化只改变 Package build identity。

## 真实产物

使用独立 store `/tmp/skiff-f271-real2.bO4Op7/store`，未操作 stable instance：

- `agine.ai/llm-api`：
  `skiff-package-build-v4:sha256:9abee747c85ee7d1b86f55ffa9bdd56f253226413efd283dbafdfd0c1815eca3`；
  Local ABI
  `skiff-package-local-abi-v3:sha256:a9504cf781a7bb6fd7cb1936ee596934af5f5796f09c14a57ff6c611b53c9efe`。
  `responses.materializeCompletedResult` 为 available，effect 全 false，
  `returnOrigins = [Fresh, Constant]`，`directReturnOrigins = [Fresh]`，不含
  `unsupportedHeapStore`。
- `agine.ai/llm-providers`：
  `skiff-package-build-v4:sha256:2aa8772859245e31b3816a10c2758c839a18ab709cb34f85e91c195aef86d449`；
  Local ABI
  `skiff-package-local-abi-v3:sha256:4b66201bf210e80ddc141eb9fdf6a88565f7c64902eee28265fcfa51d9a923c9`。
- `agine.ai/codex-relay`：
  `skiff-package-build-v4:sha256:c454009bd17deac1b2efcda0118c45b92cf03dd555442c94fdd0f4d8f3cb4e03`；
  Local ABI
  `skiff-package-local-abi-v3:sha256:a219ebd81a60894c766cd81092dbc6cac13c6f8d1510ddc62aeca6b803cf3d58`。
  service contract 17/17 operation available，包括
  `relayProxy.responsesCompleted`、`relayProxy.responsesCompletedResult` 和 `v1Proxy`。

## 验证

- `cargo test -p skiff-compiler-source --lib`：294 passed。
- `cargo test -p skiff-artifact-model --lib`：136 passed。
- `cargo test -p skiff-compiler-projection --lib`：35 passed。
- `cargo test -p skiff-deployment --lib projection::tests::eligibility`：4 passed。
- `cargo check --workspace`：PASS。
- `cargo test --workspace --no-run` 被已有的 compiler-core test
  `TypeRefIr::PackageSchema` 非穷尽 match 阻塞，与 F271 无关。
- 完整 `skiff-deployment` lib 为 51 passed / 1 个已有失败：
  `websocket_ingress_contract_validation_accepts_only_the_unified_abi`，与 F271 无关。
- canonical isolated smoke 能启动独立 MongoDB、Router 和 Runtime，但在业务 probe 前被
  既有 oracle 版本断言阻塞：产物是
  `skiff-service-protocol-v3:...`，脚本仍只接受
  `skiff-service-protocol-v2:...`。进程已由隔离 harness 清理；该失败不来自 F271
  provenance / cycle 路径。
