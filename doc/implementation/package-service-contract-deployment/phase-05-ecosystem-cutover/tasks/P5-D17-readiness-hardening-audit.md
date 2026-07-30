# P5-D17：Readiness Hardening Audit

## 角色与结论

F15首个candidate被R15拒绝：DNS解析不受deadline、pending未验证canonical activation不变量、invalid UTF-8被lossy
接受，且单文件从166行膨胀到1273行。D17只读审计现有依赖、activation/health schema与模块边界；不得编辑、提交或
给F04 verdict。

结论为`DESIGN GO`，无需manifest/lock变化：activation HTTP连接已完成DNS，可捕获其`peer_addr()`，health直接按该
SocketAddr `connect_timeout`，原authority只用于Host header；barrier内零DNS且不限制hostname URL。所有connect/write/
read/size/backoff共用absolute deadline。

pending wire只做exact field/type decode，随后构造`EnvironmentActivationState`并调用production `validate()`，复用safe
generation、expected/candidate、token、assembly与participants non-empty/unique/byte-sort不变量。所有HTTP响应strict
UTF-8。冻结F15A从原base重建并拆分orchestration、HTTP transport、readiness classifier、wire decode及各自tests；不得
cherry-pick失败candidate或引入resolver thread/第二parser/readiness owner。
