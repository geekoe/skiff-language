# P5-F53F：Node Assembly v2 Fixture

只改`scripts/tests/package-service-dev-sync.test.mjs`中RuntimeAssembly receipt的
`serviceProtocolIdentity`正例为canonical v2，并增加/保持字节级断言。运行该Node suite、syntax/diff，
提交单一commit；禁止production改动、完整gate/I02/R05。
