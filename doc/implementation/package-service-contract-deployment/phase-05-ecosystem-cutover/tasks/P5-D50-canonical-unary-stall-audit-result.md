# P5-D50：Canonical Unary Stall Audit Result

结论：COMPLETE，未命中D46，需F50A内存闭环探针定位。

Router已建立unary pending并发送`request.start`，20秒内缺失`response.end/error`。fixture、WebSocket与driver
不拥有receipt；canonical spawn应由Router回typed submitted response，Runtime host按rpcId校验并唤醒eval
continuation，worker执行明确不需要。`router_session`在supervisor begin后派生assembly子任务，read loop可继续
读取receipt，静态排除同步await自锁。

现有测试分别绕过session dispatch或直接注入registry，缺少
`request.start → outbound spawn.submit → dispatch submitted response → response.end`内存闭环。另有session
disconnect pending lease cleanup风险，但不能解释连接存活时的I02D timeout，冻结为non-blocking follow-up。
