# P5-F429 WebSocket connect execution consumer wave

状态：Ready。F426A wire后的并行实现检查点。

## 直接父节点

- `P5-F426A-websocket-connect-current-wire-result.md`
- `P5-F425A-skiff-websocket-authoring-compiler-checkpoint-result.md`
- `P5-F424A-skiff-connect-outbound-owner-audit-result.md`

三者继续引用 `P5-F425-downlink-websocket-implementation-checkpoint.md`，最终追溯到唯一权威设计
`doc/architecture/package-service-contract-deployment.md` 与其中引用的 gateway/runtime adapter
边界。F426A冻结 wire；F425A冻结 authoring/deployment/identity；F424A冻结 consumer owner和测试缺口。

## 精确输入与成熟度

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `1f52b2f5053830134e59bfa6f5c67d787078efa2` | `d859b21fbbbf8c1c3db724af53ebf3654e0c3a94` |

当前是实现检查点：authoring与connect wire已完成，Runtime/Host和Router execution尚未实现。F426A
记录的 Router全量30个旧 gateway/receive fixture failure是本wave的已知consumer缺口，不是可接受
终态。

## DAG

```text
F429A Runtime/Host current connect execution + sole-entry activation
F429B Router current assembly gateway + downlink-only lifecycle
       \________________________________________________________/
                                |
                                v
                 D4 fixture/tooling convergence
                                |
                                v
                    cheap combined path probe
```

F429A只写 Rust runtime/Host owner；F429B只写 Router gateway/router owner，禁止双方修改F426A
跨语言 protocol files和shared corpus。完成任一leaf只形成局部实现检查点；两者合流前不是
预验收候选。

## 共享冻结语义

- 每个service零或一个compiler-owned WebSocket entry；author不能选择entry id。
- connect handler可省略；省略时Router直接accept，零runtime dispatch/acquire。
- 有handler时只执行connect，结果只有accept/reject、可选business identity与policy，无Context。
- text/binary client data第一步close `1003`，零parse/queue/runtime dispatch；ping/pong/close归协议层。
- 四个send native不接收entry id，HTTP/service/actor/connect activation都从sole entry解析。
- fan-out key是`(serviceId, websocketEntryId, businessIdentity)`，忽略version/build。
- direct send也必须验证pinned assembly/service/entry/generation owner。
- 不恢复receive/message、service operation、`ContractOperationId`或legacy manifest fallback。

本wave不修改test-runner fixture、Internals或skiff-packages，不运行stable/live/instance、完整N5或
最终gate，不merge/rebase/push。
