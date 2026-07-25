# P5-F330 Service error test-effect consumer result

状态：PASS。

实现提交：`f6fa4aa3110abfb3519385012dc7c8329a0840c4`。

## 结果

- `ContractOperation`保留setup carrier/heap/build snapshot，经冻结R0 export→import；覆盖dependency public、
  private、nonclosed、encode failure、platform/Internal。
- `PackageCallable`继续只走local `materialize_local_test_throw`；service fixed carrier与Package-shaped
  service dispatch失败关闭。
- service dispatch不把setup handle/`TypeAddr` deep-clone到caller；payload由R0在caller heap重建。
- opaque fixed error encoded bytes保持，unknown public catch miss。
- 每次import创建caller-local stack和安全RemoteBoundary；synthetic provider setup frame不泄露。
- throw后response、sequence顺序及finalize行为保持。
- exact protocol/operation匹配；wrong protocol/operation/Package-shaped target不消费outcome。

## 验证

- registry list：14；registry 14/14 PASS。
- linked service-effect：4/4 PASS。
- eval library check、rustfmt与`git diff --check`：PASS。
- 原source-inline selector仍只被既知generic WebSocket schema决策遮挡；未修改compiler/WebSocket。

