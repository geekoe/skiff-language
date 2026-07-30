# P5-F372 Package signature local-slot owner audit

状态：Ready（只读；F368真实package链暴露的shared compiler blocker）。

## 直接父节点

- `P5-F356-compiler-owned-std-type-resolution-result.md`
- `P5-F291-open-error-compiler-consumer-checkpoint-result.md`
- `P5-H36-external-ingress-implementation-dag.md`

F368在fresh store成功发布std、http-session、track与llm-api后，发布`llm-providers`得到：

```text
std.http.stream return dependency type resolution failed:
Local ABI type slot #7 has ambiguous owners:
std.http.HttpClientStreamHandle
std.websocket.WebSocketCloseEvent
```

错误owner位于
`compiler/source/src/type_resolution_model/shape_assignability.rs::
rehydrate_package_signature_local_type`：遇到没有module owner的
`TypeRefIr::LocalType { type_index }`时跨整个dependency package猜唯一module。该猜测在std多个文件合法复用
slot 7时失败。

## 只读审计问题

1. 从`std.http.stream` source → FileIR executable signature → PackageArtifact public callable signature →
   consumer rehydrate，逐跳确认module owner在哪一步丢失；给出精确路径/函数/字段。
2. 判定目标不变量应由哪个canonical owner保证：
   - producer把跨package公开签名中的local nominal改为带module的`PublicationType`/exact symbol；或
   - consumer必须从callable owner一并携带module再解析；
   - 不得用“slot在整个package唯一”或按public/display name猜测。
3. 搜索所有可能产生同类歧义的参数、返回值、container/nullable/function/any-interface嵌套路径，确认影响面不
   只限`std.http.stream`。
4. 判断修复是否改变Package Local ABI canonical identity或artifact schema generation；列出会失效的真实
   package receipt与最小重建顺序。
5. 给出一个可执行实现任务的最小production/test owner、正负例与聚焦命令。若需要改变尚未冻结的公共artifact
   语义，明确返回`TASK_NOT_EXECUTABLE`及用户决策点；否则返回单一路径。

## 边界与交付

只读，不修改文件、不提交、不运行stable/live或完整workspace gate，不派子Agent。允许运行聚焦现有测试或
生成temporary artifact用于追踪，但不得清理他人证据目录。

审计checkpoint：

- Skiff integration：包含本task的`codex/package-service-phase-05`；
- F368真实std build：
  `skiff-package-build-v8:sha256:eb7a294930e76caabe86d73600f84da63cdb4e88ac8c763bbb64377ffe7ea69f`；
- failing consumer：Internals `packages/llm-providers`。

返回精确调用链、同类命中清单、唯一owner、建议任务边界、测试矩阵与是否需要用户决策。
