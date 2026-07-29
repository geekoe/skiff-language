# P5-F445H I7 P8 I AIHub HTTP entry migration

状态：

```text
BLOCKED_BY = T
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-T-http-entry-combined-probe.md`
- Internals baseline：
  `9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）
- dispatch前同时记录T通过的精确Skiff commit/tree。
- Internals integration owner：`/root/phase05_internals_integration_steward`
- DAG：`T -> I -> X`

## 2. Scope

只迁移M6四条失败case：

```text
chat events HTTP route returns structured event body
chat event stream preserves per-item chunk order and full event projection
chat event stream keeps emitted items before each post-start failure
chat event stream consumer break cancels the provider ancestor chain
```

写集：

```text
aihub/service-tests/http.yml
aihub/service-tests/internal/aihub_service.test.skiff
```

test service显式声明测试route，wrapper调用`subjectImpl`；case使用
`config.require<string>("skiff.test.ingressUrl")`与普通`std.http.request/stream`进入真实Router。
删除仅为直接调用stream helper而存在的测试helper；不修改AIHub production service/http.yml、业务
handler、provider package或live Gemini case。

流式断言必须组合完整response body或解析完整SSE event，不断言网络chunk边界。post-start error保留已发
event并验证终止语义；consumer break验证provider ancestor停止。

## 3. Evidence

M6精确账本是四条同类失败的RED，不重复运行旧完整矩阵。迁移后在精确Internals candidate +
T Skiff candidate运行一次GREEN：

```text
SKIFF_ROOT=<P8-Skiff-worktree> \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs aihub
```

`<P8-Skiff-worktree>`在执行时替换为T已通过的精确Skiff candidate绝对路径。脚本必须继续使用owned
temp artifact/Cargo root与isolated runner，不得访问stable 4000/4001、外网、真实API key或live test。
允许用户已授权的临时managed Mongo，但必须动态端口/临时目录并清理。

负例：没有显式test `http.yml`时失败；直接handler调用不再冒充route coverage；默认发现仍为51且
Gemini `defaultRun false`不执行。

## 4. Stop conditions

需要修改production handler、测试专用HTTP API、特殊URL、session/header协议、真实secret/network或
其它AIHub case时返回`TASK_SCOPE_EXPANDED`。
