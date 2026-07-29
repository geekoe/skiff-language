# P5-F445H I7 P8 I AIHub HTTP entry migration

状态：

```text
PAUSED_BY = S1
RECOVERABLE_CHECKPOINT = bdf7bd4adc59cd32d615e4f5498d3e764df4384e
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-S1-package-direct-http-stream-registry-closure.md`
- Internals baseline：
  `9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）
- 当前可恢复Internals checkpoint：
  `bdf7bd4adc59cd32d615e4f5498d3e764df4384e`
  （tree `324fe69f3ea58786b297e412da436c03b05d9656`）；该提交尚未集成，不是最终candidate。
- dispatch前同时记录T与S1通过后的精确Skiff commit/tree。
- Internals integration owner：`/root/phase05_internals_integration_steward`
- DAG：`T -> S1 -> I resume -> X`

当前checkpoint中的四条迁移已通过真实isolated Router/Runtime到达raw HTTP entry，但都观察到
`unknown Stream value`。这只是一条可恢复RED，不足以证明当前根因是第二个registry、错误lifetime、
heap差异、top-level alias或其它候选；根因归属必须由S1的create/lookup identity轨迹决定。

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
T+S1 Skiff candidate运行一次GREEN：

```text
SKIFF_ROOT=<P8-Skiff-worktree> \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/aihub
```

`<P8-Skiff-worktree>`在执行时替换为T与S1均通过的精确Skiff candidate绝对路径。脚本必须继续使用owned
temp artifact/Cargo root与isolated runner，不得访问stable 4000/4001、外网、真实API key或live test。
允许用户已授权的临时managed Mongo，但必须动态端口/临时目录并清理。

负例：没有显式test `http.yml`时失败；直接handler调用不再冒充route coverage；默认发现仍为51且
Gemini `defaultRun false`不执行。

## 4. Stop conditions

需要修改production handler、测试专用HTTP API、特殊URL、session/header协议、真实secret/network或
其它AIHub case时返回`TASK_SCOPE_EXPANDED`。

S1尚未完成、S1没有稳定RED、S1结论要求改动公共契约，或恢复后仍出现新的独立blocker时，不得在本
Internals任务中修改Skiff Runtime；保持checkpoint可恢复并返回主Agent重排。
