# P5-F55A：Host / Driver Legacy Consumer Removal

独立worktree，独占runtime host/driver旧consumer。移除旧service closure/program loader/route/service context、
driver LP2 facade与重复`http_boundary`；先把仍用于request heap的`RuntimeMemoryBudgets`迁到canonical host config
owner，再删空artifact/program caches及maintenance伪消费。保留assembly admission/execution、shared HTTP boundary、
request heap语义。删除旧测试而非shim。

运行host/driver check、active assembly request/reconnect/full-chain正例、静态负反搜、rustfmt/diff，提交单一commit。
不得改loader/linker/linked-program/artifact model，禁止I02/R05/full gate。
