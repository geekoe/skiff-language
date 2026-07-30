# P5-F23C：Runtime WebSocket Response Boundary

依赖D33 complete。与F23A/F23B并行；独占Rust
`runtime/{request-contract,request,eval,transport}/**`、canonical response corpus与直接tests，不改Router、Host admission、
std/compiler/deployment或四对象。独立worktree/branch，一个clean commit。

以discriminated enum/newtype表达ConnectAccept、ConnectReject与Receive，消除跨层optional bag非法状态；canonical projector
移入独立模块并删除legacy复制，legacy adapter继续拒绝`websocket.ingressEvent`。transport/request严格检查phase、HTTP/WS
metadata互斥、payload flag/bytes与context presence/codec，保留nominal Context零字节正例；最终调用仍只经
`dispatch_in_process_boundary`。Rust trust boundary按admitted route重算冻结entry/gateway identity，并补共享golden/mutation
corpus；不实现generation registry或source同步ACK。

跑受影响四个runtime crate的精确filters、corpus、legacy negative、diff-check；每个filter非零。禁止full/I16/Host/stable。
