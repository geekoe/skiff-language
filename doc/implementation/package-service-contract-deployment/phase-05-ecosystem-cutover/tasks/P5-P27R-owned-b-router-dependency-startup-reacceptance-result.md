# P5-P27R：Owned B Router Dependency Startup Reacceptance Result

`P27R PASS`

证据锚定`35f93c9e1fdad20a95daf39ad07e26c126a90512` / tree
`7bdb6d49731edcf8ca3a1a84d02a91f2f09662b2` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。production helper blob为
`cf2ee2dd1ca23216795f60329639f90613fcab35`；Router package/lock blob分别为
`0eebb71f172cf1a71982ca6b97142ef55859f610`与`5fb4ed8bd3b3a21370ae94cd568f9447b6d6b5b2`。

owned B上的dependency helper恰好调用一次：locked/offline install与B-local tsx验证均code 0、signal null，调用数分别为
1，tracked tree与lock内容不变。随后generation-0 bootstrap、supervisor及Router/Runtime readiness全部PASS，empty callback
恰好1次并返回`P27R_EMPTY_CALLBACK_OK`；source runner、Host、full与stable调用数均为0。该动态正例关闭了P27S的
`tsx: command not found` startup blocker，没有发现新的production owner。

cleanup为PASS：B worktree/path/Git admin、dependency materialization、inner workspace、task root、component PID/PGID、
端口与lease全部ABSENT；marker/nonce/dev+ino在删除前验证，registry与integration identity前后不变，foreign state preserved，
errors为空。

持久证据为
`/Users/geek/workspace/skiff-phase-05-evidence/p5-p27r-35f93c9-owned-b-router-startup.json`，文件SHA-256为
`5259efa942af11840307efc87865cb0895329d7af9e931eb770ade9e56720f4a`，内部evidence digest为
`959f5527fbe213bd9652ebb279ef2d4a5096845e4f9f1a7b4da8312fb3d89522`。P27R只关闭F21C/P27S的
dependency-preparation→startup边界并解锁R21C与replacement I16，不是G16、F04或阶段verdict。
