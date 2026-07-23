# P5-F53D：Router Manifest v2 Residuals

只改Router artifact/manifest canonicalization production owner、hello manifest、README、manifest/websocket helpers
及D53列出的artifact/manifest/HTTP/WS正例测试；统一canonical service v2并更新v2错误文案。不得改
`protocol.test.ts` legacy拒绝负例或frame schema。运行所列Router suites、type-check/diff/反搜，提交单一commit；
禁止完整gate/I02/R05。
