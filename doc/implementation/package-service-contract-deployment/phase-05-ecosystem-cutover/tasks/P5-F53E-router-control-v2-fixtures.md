# P5-F53E：Router Control v2 Fixtures

只改`actor-spawn-runtime-control.test.ts`、`assembly-runtime-endpoint.test.ts`、
`router-default-spawn-probe.test.ts`、`runtime-registry-dispatch.test.ts`中的普通SPI正例为canonical v2；
legacy sender拓扑测试仍通过拓扑表达，不得靠非法v1遮挡。禁止改production与`protocol.test.ts`。
运行四个命名suite、type-check/diff，提交单一commit；禁止完整gate/I02/R05。
