# P5-F23E：WebSocket Generation Lifecycle Wire

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第5、8、9、10条，§6.2、§7及§12。跨generation的长连接必须
保持其建立时的完整assembly、deployment、ActivationContext与capability owner，drain/reload必须可观测；本任务冻结实现该
不变量所需的内部TS/Rust control wire，不改变公共business ABI、四对象或identity公式。

DAG节点F23E，依赖R24 PASS；完成后同时解除F03B Router consumer与F03C Runtime consumer。风险高，验收分组为WebSocket
generation lifecycle shared seam。进入代码状态与exact HEAD由R24结果及派发信息给出。

在F03B/F03C实现前由单一owner冻结TS/Rust shared control wire与cross-language corpus。写入边界仅Router protocol
schema/codec及直接tests、Rust transport/control protocol对应类型及直接tests、同一cross-language golden；不实现Router
store/gateway consumer，不实现Runtime admission/host consumer，不改business ABI、四对象、identity公式或既有request wire。

冻结语义：connect成功后Runtime按`router session + service + assembly identity + generation + entry + connection`隐式幂等
acquire；Router在client/policy/gateway close显式发送release并等待bounded ack；runtime session断开双方清理该session全部pin；
duplicate release幂等，wrong tuple/sender fail closed。wire必须携带完整typed tuple、request id和成功/typed rejection，不能复用
已结束单请求的`request.cancel`，不能用TTL/GC代替release。TS/Rust共享golden与unknown/missing/duplicate/identity mutation。

完成标准：

- TS/Rust从同一golden接受exact acquire/release/ack/rejection并逐字段round-trip；
- unknown/missing/extra/duplicate、tuple/identity/request-id/sender mutation全部fail closed；
- protocol类型与codec不读取store、registry、runtime admission或gateway状态，不出现第二lifecycle owner；
- 新增Router direct test文件后用直接Vitest路径运行，Rust用新模块exact filter运行；两侧测试数非零，并运行Router
  type-check、相关crate fmt/check、cross-language corpus及diff-check。

一个clean commit；禁止consumer、real smoke、combined/full/I16/Host/stable，不merge/push。shared wire/corpus、相关protocol
schema或identity tuple变化会使证据失效；完成后成熟度仅为Shared Interface Checkpoint。
