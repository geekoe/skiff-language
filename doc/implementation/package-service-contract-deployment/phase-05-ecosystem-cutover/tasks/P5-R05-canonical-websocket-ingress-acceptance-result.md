# P5-R05：Canonical WebSocket Ingress Acceptance Result

结论：FAIL。唯一真实transcript命令只运行一次，未重试、未修改候选、未操作stable。

- production candidate：`c808586546fddc5550f1caf7e520e849162a0946`
- production tree：`3db51a012b77137a992a01a8b3c2e10944f57f68`
- docs HEAD：`cbad2f47ca5df092e8ef65182144ed6acd00deca`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

唯一命令：

```bash
node scripts/run-package-service-generation-lifecycle-smoke.mjs \
  --probe r05-generation-lifecycle \
  --replicas 1 \
  --checkout "$PWD"
```

约16.6秒后exit 1。A activation/connect、B activation、A receive两次A marker及B connect/receive B marker
均已通过；随后generation B unary production请求`POST /probe`返回HTTP 404，而合同要求200及JSON B
marker。因该断言终止，正常路径的`close B → pin=1 → close A → pin=0/inFlight=0/no pending`未建立；finally
cleanup不能替代drain oracle。

当前最小implementation owner是F41 generation lifecycle harness的真实unary client/diagnostic。direct test
注入fake `requestUnary`固定返回200，且失败未保留实际wire Host与404 body，现有证据不足以把责任转交Router。
I31 author/store 1/1与既有静态证据仍有效；R05动态证据失败，不解锁Cargo.lock refresh、I02或R02。

修复必须进入新预验收周期：先由F41A direct evidence闭合，再由全新I32 owner运行合流状态上的cheap combined，
之后才允许全新R05A Agent再次运行一次完整transcript。
