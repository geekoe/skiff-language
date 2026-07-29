# P5-F445H I7 P8 I AIHub HTTP entry migration

状态：

```text
PASS
INTERNALS_INTEGRATED = YES
DEFAULT_TESTS = 51/51
```

## 1. Parent and baseline

- 直接父节点：
  `P5-F445H-I7-P8-S3-deferred-response-sink-propagation.md`及其PASS result
- Internals baseline：
  `9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）
- 最终Internals leaf：
  `698e272a86b74434d508102bf3bb7e624c45cedc`
  （tree `3ef3edf2dbe39928565226764f84b7d48a66a578`）
- 最终Internals integration：
  `14d3d2d0a7171a57fde6c5dd19b8d7eb4903ccca`
  （tree `d58b732d45354f0b01efc24285a20ec3464f1b72`）
- 最终GREEN使用的Skiff candidate：
  `8e30f514caa3f219f4a77452684359d4a5ddbdd5`
  （tree `d34517add43505cf1d6e9f38e34fef6ffa110128`）
- Internals integration owner：`/root/phase05_internals_integration_steward`
- DAG：`T -> S1 diagnostic -> S2 -> S3 -> I -> X`

S2/S3闭合stream argument transport与existing response sink后，四条迁移经真实isolated
Router/Runtime全部GREEN。完成结果：
`P5-F445H-I7-P8-I-aihub-http-entry-migration-result.md`。

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
保留T证据、S1 diagnostic并已集成S2/S3最终GREEN结果的Skiff candidate运行一次GREEN：

```text
SKIFF_ROOT=<P8-Skiff-worktree> \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/aihub
```

最终以`/Users/geek/workspace/skiff-phase-05-integration`的
`8e30f514caa3f219f4a77452684359d4a5ddbdd5`运行该命令，结果为默认测试
`51 passed; 0 failed; 0 skipped`。脚本使用owned temp artifact/Cargo root与isolated runner，未访问
stable 4000/4001、外网、真实API key或live test；临时managed Mongo、Router、Runtime、supervisor、
动态端口、lease与临时目录均已清理。

负例：没有显式test `http.yml`时失败；直接handler调用不再冒充route coverage；默认发现仍为51且
Gemini `defaultRun false`不执行。

完整执行证据：
`/Users/geek/workspace/P5-F445H-I7-P8-I-red.log`
（SHA-256 `b930ea549fcaed6690cd1a7bf2b3086367c02fab4d92e31776669bb425f9ffe6`）。

## 4. Stop conditions

需要修改production handler、测试专用HTTP API、特殊URL、session/header协议、真实secret/network或
其它AIHub case时返回`TASK_SCOPE_EXPANDED`。

S1继续保持`TASK_NOT_EXECUTABLE`。S2未最终GREEN、S3未明确
`I_RESUME_UNBLOCKED=YES`、S2/S3要求改动公共契约，或恢复后仍出现新的独立blocker时，不得在本
Internals任务中修改Skiff Runtime；保持checkpoint可恢复并返回主Agent重排。
