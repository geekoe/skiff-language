# P5-P27S：Shared-target B-root Startup Probe Result

`P27S CALLBACK_PRESTART_FAILURE`

诊断候选锚定`7bb6c2af9517f2091654fd1f127e87ca6ef02f68` / tree
`3fc5ed41be62155d86365d2df46a5b1a1bbc90bb` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`；probe启动时integration HEAD为
`dbfb98ac0a10d3959d803a8a92de1c04bba66fce`。preflight可用容量41,329,295,360 bytes，高于要求的
33,207,861,248 bytes。

A、B均完成runner/smoke build；B-root的`skiff-test-runner`、`skiff-compiler`、`skiff-compiler-input`、
`skiff-compiler-source`四crate全Fresh。shared target只有两个顶层`.d`发生exact root materialization，
`changed=2/allowed=2/disallowed=0`。generation-0 bootstrap PASS，assembly identity为
`skiff-runtime-assembly-v1:sha256:4176e39122928fcf47db987c34884f2f7ab4a1833c502a33bb6fd0c861a5acf6`。

supervisor spawn后在readiness前以1退出，callback count为0，runtime未启动。Router stderr为27 bytes，SHA-256
`93fc972ac0b9348bc886b00cee8f770154ae5289c9f5e06ca22edaee18e61659`，完整保留
`sh: tsx: command not found`；Router stdout为326 bytes，SHA-256
`739ce069d23d38dcda19ced9dfd552a43b0ffaffdbc92e4f933499e1f6be8296`，显示local `package.json`存在但
`node_modules`缺失。该证据将失败归类为probe环境依赖未就绪；它没有证明production runtime/artifact-identity缺陷。

cleanup全部PASS：inner runtime root、supervisor PID、A/B路径与Git registry/storage、owned task root、shared target均
ABSENT；integration HEAD与tracked tree不变，registry before/after SHA一致，command process groups absent、foreign preserved、
errors为空。`stableOperations:0`、`fullProbeRuns:0`、`hostRuns:0`。

持久证据为`/Users/geek/workspace/skiff-phase-05-evidence/p5-p27s-7bb6c2a-shared-target-startup.json`，文件SHA-256
`4986353f8f2fdb2a4daf9beb2fe754a7740b17fd6942c91da6815bd882827f99`，内部evidence digest为
`c3dcf89033f4295534d027d2deccee970bafc24a39e5e30363bb81d3099018c3`。本结果只是非full诊断checkpoint；F21C须作为
下一独立节点先建立任务合同，不得据此宣称阶段完成。
