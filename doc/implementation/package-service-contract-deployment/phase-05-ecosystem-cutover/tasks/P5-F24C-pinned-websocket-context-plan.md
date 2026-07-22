# P5-F24C：Pinned WebSocket Context Plan

依赖F24B exact commit合流。独占`runtime/eval/src/assembly_execution/websocket_*`及直接normal-source tests，不改boundary
owner、linked plan、Router或compiler lowering。一个clean commit。

Event、Result与nested Context plan全部从pinned ServiceContract descriptor及F24B service-value plan取得；executable只保留
target、参数名/数量和contract一致性断言，不再作为Context schema owner。normal-source nominal Context必须经
compiler→deployment→eval，覆盖receive text/binary decode、connect accept encode、wrong codec/identity、Context=null与
typed zero-byte presence结构正例；canonical path仍恰好一次`dispatch_in_process_boundary`，legacy继续拒绝新Event。

跑eval/request/boundary相关精确非零tests、response corpus、fmt/diff-check；禁止real smoke/full/I16/Host/stable。
