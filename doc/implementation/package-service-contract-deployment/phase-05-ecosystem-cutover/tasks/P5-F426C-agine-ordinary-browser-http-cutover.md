# P5-F426C Agine ordinary browser HTTP cutover

状态：Ready。中风险consumer迁移。

## 直接父节点

- `P5-F426-connect-wire-and-http-consumer-wave.md`

HTTP request/response contract由F425D写入`agine/protocol/http.ts`；本文不得自行发明第二套shape。

## DAG位置

依赖F425D，与F426A/B并行。完成后ordinary browser RPC不再需要WebSocket；Host、host-file与legacy
receive cleanup仍不解除。

## 写入范围

仅允许：

- `agine/client/src/stores/appStore/{configActions,chatActions,messageActions}.ts`
- `agine/client/src/components/ToolCallCard.tsx`
- `agine/client/src/lib/{http,toolproviderApi,threadHostBindings,protocol,socket,ws}.ts`
- 上述owner的直接Vitest tests
- ordinary browser相关的`agine/client/e2e/**` helper/spec/mock；不得改变Host或host-file业务协议
- 必要时Agine专用的heartbeat配置/adapter文件
- 本leaf result

禁止修改`agine/protocol/**`、service、Host、`hostFileApi.ts`、file picker/browser pane、
shared-client production或其它service。

## 必须实现

1. F425D列出的22个ordinary-user operation全部使用literal HTTP path与
   `agine/protocol/http.ts` payload；不得继续调用`socket.request/send`或发送旧event envelope。
2. 复用现有cookie/session、service/version selector、finite error envelope和logging owner；HTTP body
   不发送user identity。
3. agent/provider/chat/toolprovider/client tool result的成功与错误可观察行为保持；browser tool result
   精确使用`executor:"client"`。
4. chat async event、run terminal和真实主动通知仍由WebSocket下行消费；不得删除listener或connect。
5. Agine browser不再发送应用层JSON `ping`。只能在Agine本地禁用；若必须修改shared-client production，
   返回`TASK_SCOPE_EXPANDED`并给出最小shared owner。
6. 更新chat smoke/two-host cleanup/machine browser helper中属于这22项的direct WebSocket RPC；仍属Host或
   host-file的调用保持不变。
7. 不删除通用cookie WebSocket helper，除非反向搜索证明它已无任何保留Host/host-file用途；最终legacy
   cleanup由后继owner完成。
8. production反向搜索中，这22项不再经过WebSocket；`socket.request/send`只可剩明确Host/host-file或
   test mock允许项。

## 验证

至少覆盖HTTP path/payload、session/error、agent delete、chat stop/update/model/pin/delete/usage、
toolprovider与client tool result、mock/E2E transport选择、无应用层ping及WebSocket下行仍工作。

运行实际client入口：

```bash
npm run type-check --workspace @agine/client
npm run test:logic --workspace @agine/client
git diff --check
```

linked worktree不运行browser live、stable chat smoke或真实Host。记录真实discovery/pass/fail/skip。

## 交付

在Internals提交implementation；在Skiff任务worktree新增并提交
`P5-F426C-agine-ordinary-browser-http-cutover-result.md`。返回两个commit/tree、自验收矩阵与clean
状态。不得merge/rebase/push/stable/live。

