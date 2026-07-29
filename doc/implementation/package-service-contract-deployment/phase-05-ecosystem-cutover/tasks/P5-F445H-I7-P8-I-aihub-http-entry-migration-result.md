# P5-F445H I7 P8 I AIHub HTTP entry migration result

状态：

```text
PASS
INTERNALS_INTEGRATED = YES
INTERNALS_PRODUCTION_CHANGE = NO
DEFAULT_TESTS = 51/51
LIVE_TEST_EXECUTED = NO
```

## 1. Identities

- Skiff docs baseline：
  `7eb8690589a89d2c8d0be66198e4f02327fed6c7`
  （tree `230846fb2378bb5c54e1afa8f574d2c363572606`）
- 最终GREEN使用的Skiff candidate：
  `8e30f514caa3f219f4a77452684359d4a5ddbdd5`
  （tree `d34517add43505cf1d6e9f38e34fef6ffa110128`）
- Skiff packages candidate：
  `1dd97eb0a2a6d129a912d578e2977469b86c34b4`
  （tree `b5c87af353891ad71294e99d2a104dafbaa32455`）
- Internals baseline：
  `9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）
- Internals implementation leaf：
  `698e272a86b74434d508102bf3bb7e624c45cedc`
  （tree `3ef3edf2dbe39928565226764f84b7d48a66a578`）
- Internals integration：
  `14d3d2d0a7171a57fde6c5dd19b8d7eb4903ccca`
  （tree `d58b732d45354f0b01efc24285a20ec3464f1b72`）

Internals integration包含implementation leaf，且最终worktree clean。

## 2. Implementation

AIHub test service新增显式raw HTTP test routes，wrapper调用`subjectImpl`。四条原先直接调用stream
handler/helper的case改为：

- 从`config.require<string>("skiff.test.ingressUrl")`取得普通动态HTTP origin；
- 以`std.http.request`或`std.http.stream`经过真实Router与Runtime；
- 对完整response body或完整SSE frame做断言，不依赖网络chunk边界；
- 保留post-start error之前已发event、终止语义和consumer break向provider ancestor传播取消的覆盖。

仅供直接stream调用的helper已删除。最后一个RED来自已经不被真实入口消费的model-list
`std/http.request` outcome；只删除该漂移声明，保留实际消费的`std/http.sse` outcome、
unused-effect严格检查和全部业务断言。

没有修改AIHub production `service/http.yml`、handler、provider、其它case、Skiff production、公共协议、
特殊URL或测试专用header/session。

## 3. Actual write set

相对Internals baseline的累计写集只有：

```text
aihub/service-tests/http.yml
aihub/service-tests/internal/aihub_service.test.skiff
```

累计diff为`178 insertions; 66 deletions`。最终integration提交的写集与leaf一致。

## 4. Final GREEN

执行：

```text
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
node scripts/test-isolated-service.mjs agine.ai/aihub
```

结果：

```text
default discovered = 51
passed = 51
failed = 0
skipped = 0
```

发现数由`aihub_service.test.skiff`的50条与
`managed_provider_transport.test.skiff`的1条组成。`gemini.live.test.skiff`只有1条live case，文件声明
`test defaultRun false`，本次没有执行，也没有读取真实API key、secret或访问外网。

先前一次GREEN复验在test suite启动前因isolated Runtime收到`SIGTERM`而中断；它没有产生case证据。
在更新后的精确Skiff candidate上允许的一次干净重试成功，未出现
`unknown Stream value`、unused effect或其它case失败。

## 5. Isolation and cleanup

最终成功运行只使用动态loopback端口、owned临时artifact/Cargo root与临时managed Mongo。运行完成后确认：

- 临时workspace `skiff-test-runtime-ueFcXi`已删除；
- Mongo、Router、Runtime和supervisor进程全部退出；
- 本地端口lease目录为空；
- 未使用stable 4000/4001、共享Mongo、OAuth或browser。

完整执行账本：

```text
/Users/geek/workspace/P5-F445H-I7-P8-I-red.log
SHA-256 b930ea549fcaed6690cd1a7bf2b3086367c02fab4d92e31776669bb425f9ffe6
```
