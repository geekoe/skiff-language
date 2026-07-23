# P5-D54：Runtime Config Two-Failure Audit Result

结论：COMPLETE。D54A为production loader解析错误：canonical prefix含`:sha256`，loader二次切分后必然比较失败；
由artifact-identity typed helper统一解析。D54B为过期test-only services注册断言；production只注册active
RuntimeAssembly，删除旧register段并保留loader revision断言，真实registration由既有lifecycle/full-chain覆盖。
