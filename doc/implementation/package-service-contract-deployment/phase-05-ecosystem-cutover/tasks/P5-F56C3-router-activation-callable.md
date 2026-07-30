# P5-F56C3：Router Activation Callable

基于F56C0实现typed Host↔Router activation prepare/commit/abort request/receipt并委托既有coordinator。Router仍独占
participant选择、prepared/connected ACK与durable transition；callable不得开放ACK sets/state direct-write。
principal/capability/request correlation fail closed，abort/rollback语义不变。

写入shared wire/Router/Host control边界及聚焦测试；不实现DB store或official package。check/type/test/diff后
单commit；禁止stable/full gate。
