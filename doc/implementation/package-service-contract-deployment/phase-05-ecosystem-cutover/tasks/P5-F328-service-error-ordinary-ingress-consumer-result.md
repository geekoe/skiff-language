# P5-F328 Service error ordinary and ingress consumer result

状态：PASS。

实现提交：`6d743481dc7a762a4ac64d9ae5224f5bfc4ff2ef`。

## 结果

- ordinary provider使用fresh heap和provider-local stack scope，并在heap drop前调用冻结R0 export；旧generic
  provider-error passthrough已删除。
- central dispatcher只对internal fixed failure执行caller import；Ingress与WebSocket ingress原样向上交
  fixed carrier，不创建external caller exception。
- internal import使用caller heap/call site/current stack与resolved service/operation facts。
- public/Internal A→B→C保持exact bytes、traceId/errorId；B/C分别重建本地stack。
- provider heap销毁后caller value有效；linked public/Internal exact catch成功，unlinked public catch miss，
  local rethrow保持imported exception。
- wrong owner/key/type id为Protocol；non-fixed provider boundary result失败关闭。
- representation、private/Internal、platform File以及dependency-owned Resource路径覆盖。

## 验证

- ordinary selector：17；ordinary 15/17 PASS，剩余仅两个既知generic WebSocket compiler blocker。
- 新service-error consumer：5/5 PASS。
- ingress：1/1，WebSocket ingress：8/8，boundary materialization：3/3 PASS。
- eval library check、scoped rustfmt与`git diff --check`：PASS。

