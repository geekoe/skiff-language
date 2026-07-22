# P5-F23E：WebSocket Generation Lifecycle Wire

依赖R24 PASS；在F03B/F03C并行实现前由单一owner冻结TS/Rust shared control wire与cross-language corpus。只拥有
Router protocol schema/codec和Rust transport/control protocol对应类型及直接tests，不实现Router store/gateway consumer，
不实现Runtime admission/host consumer，不改business ABI、四对象或identity公式。

冻结语义：connect成功后Runtime按`router session + service + assembly identity + generation + entry + connection`隐式幂等
acquire；Router在client/policy/gateway close显式发送release并等待bounded ack；runtime session断开双方清理该session全部pin；
duplicate release幂等，wrong tuple/sender fail closed。wire必须携带完整typed tuple、request id和成功/typed rejection，不能复用
已结束单请求的`request.cancel`，不能用TTL/GC代替release。TS/Rust共享golden与unknown/missing/duplicate/identity mutation。

一个clean commit；只跑两侧codec/corpus、crate/Router精确tests与diff-check，不跑consumer、full/I16/Host/stable。
