# P5-D45：Canonical Actor Control Design Result

用户决策已写入唯一权威设计：
`doc/architecture/package-service-contract-deployment.md` §2第11条及§12。

canonical actor/spawn control必须携带完整ActivationIdentity；Runtime从当前ActivationContext填充。Router只按发送者
exact assembly registration及active/draining generation snapshot验证，禁止按serviceId、package build、display name或
legacy `runtime.register`推断。active generation可发起新control；被显式pin的draining generation仅在原pin生命周期内
继续使用原ActivationContext，drain完成后fail closed。

实现DAG为F45B shared identity/wire checkpoint，随后F45C Runtime与F45D Router并行，最后F45E真实I02 actor probe。
shared control wire变化使R05B证据失效；I35 PASS后用全新R05C一次重建，再运行I02。
