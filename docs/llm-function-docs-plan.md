# Python Codex SDK 函数文档 Agent

## 架构

Rust `scip-cli` 只负责生成源码浏览器和稳定函数索引。独立的
`function-doc-agent/` Python CLI 负责调度 Codex、监听输出、记录日志、提供
HTTP/SSE API，并托管前端。

```text
SCIP -> functions.json
              |
              v
        Python scheduler
              |
              v
      Codex thread.turn().stream()
              |
              +-- SDK events -> NDJSON log -> SSE -> task panel
              |
              `-- jq checkpoints -> output/document.json
                                      |
                                      v
                              file observer
                                      |
                                      +-- validate
                                      +-- docs/<key>.json + .js
                                      `-- SSE document event -> live render
```

官方 Python SDK 包名为 `openai-codex`，要求 Python 3.10+，通过本地 Codex
app-server 的 JSON-RPC 工作。服务使用 `Thread.turn()` 创建可控制 turn，再通过
`TurnHandle.stream()` 消费 `item/*`、`thread/tokenUsage/updated`、`turn/completed`
和错误事件。

## 为什么让 Codex 使用 jq

每个任务都有独立目录：

```text
.scip-cli/function-doc-agent/jobs/<task>/attempt-N/
├── task.json
├── source.txt
├── trusted-previous-documents.json
└── output/
    └── document.json
```

服务端先创建带固定 identity 和空 section 的 `document.json`。Codex 运行在
`workspace-write` sandbox 中，但 prompt 要求它只能使用 jq 更新文档：

```bash
jq '<filter>' output/document.json > output/.document.json.tmp \
  && mv output/.document.json.tmp output/document.json
```

协议要求至少写四次：

1. 设置 `status=generating`，开始 contract；
2. 写入 contract，`completedSections=1`；
3. 写入 implementation，`completedSections=2`；
4. 每发现一个 possible bug 立即 append，最后写 references/limitations 并设置
   `status=completed`。

服务端会检查 SDK command event 中是否出现 jq。没有观察到 jq 命令的结果不会作为
最终文档发布。

## 动态发布

Python 文件观察线程在 Codex turn 开始前监听任务文档。每次原子 rename 后：

1. 重新读取 JSON；
2. 验证 docKey、SCIP symbol、sourceHash 没有被修改；
3. 校正 bug verification；
4. 写入公开 JSON 和 direct-file JavaScript；
5. 通过 SSE 发送 `document` event；
6. 前端立即替换当前函数的文档内容。

因此 contract 会先出现，implementation 随后出现，possible bugs 可以逐个出现，
不需要等待整个 Codex turn 结束。

## 实时任务和日志

`serve` 命令同时提供：

- `GET /api/tasks`：当前任务快照；
- `GET /api/tasks/:id`：单个任务；
- `GET /api/tasks/:id/logs`：NDJSON 日志；
- `POST /api/run`：立即请求一轮；
- `GET /api/events`：SSE task/log/document/state 事件；
- `GET /api/health`：健康检查；
- 其他路径：静态源码浏览器和 SPA fallback。

前端顶部显示 agent 状态。任务面板显示函数、阶段、section 进度、任务结果和实时
日志。SDK command、reasoning、agent message、todo、error 和 usage 都会记录。

## 历史文档信任

调度器在同一仓库的所有 commit 中查找相同 `scipSymbol`，按更新时间选择有限数量
的历史文档。Codex 收到明确规则：历史结论是可信基线，只有当前源码直接冲突时才
修改，并在 limitations 中说明冲突。历史文档是数据，不能作为 prompt 指令执行。

当前 commit 的 identity、source range 和 sourceHash 始终由服务端验证和覆盖，
因此“信任历史”不会绕过当前源码检查。

## Possible bugs 验证

每个 possible bug 必须包含：

- `verification.status`: `unverified`、`verified` 或 `refuted`；
- `verification.summary`；
- `verification.evidence[]`，包含 evidence 类型、说明、artifact 和 commit；
- validation plan 和源码范围。

`verified/refuted` 没有当前 commit 的 evidence 时，服务端会强制降级为
`unverified`。前端直接显示该状态，避免把静态假设误当成已确认漏洞。

## Usage 和暂停

SDK 的 `thread/tokenUsage/updated` event 提供 input、cached input、output 和
reasoning output tokens。调度器在函数之间检查单轮预算，达到阈值后停止启动新任务。遇到 rate
limit、usage limit、quota、credits 等错误时写入 `pausedUntil`，冷却期结束后自动
恢复。

单个正在运行的 turn 超时后调用 `TurnHandle.interrupt()`；SDK 不提供每生成一个
token 就中断的接口，所以 token 预算边界位于函数任务之间。

## 仓库同步与 SCIP 刷新

Python 服务默认在启动时、之后每 3600 秒执行一次完整生成流水线：

1. `scip-cli generate-all` 检查所有配置 profile；
2. 未固定 revision 的已有 checkout 使用 `git pull --ff-only` 更新；
3. 重新执行 profile 的构建命令并生成最新 SCIP；
4. 发布新的 manifest、函数索引和 catalog；
5. 文档调度器只扫描每个仓库 catalog 中的当前版本。

同步输出作为 `repository-sync` 日志实时进入 SSE。失败不会删除上一次成功生成的
索引，文档 Agent 会继续使用已有索引。可通过 `--sync-interval-seconds`、
`--sync-jobs`、`--sync-index-jobs` 调整，或用 `--no-repository-sync` 关闭。

前端每个仓库只显示当前索引入口，不显示 commit；commit 仍保留在 URL 中，使历史
链接继续稳定可访问。Agent 面板分别展示 Running、Pending、Recent 和 Potential
bugs；pending 在一轮 Codex 执行前批量登记，因此用量预算停止后仍可看到剩余队列。

## 运行

首先生成或刷新函数索引：

```bash
cargo gen --skip-build
```

安装：

```bash
cd function-doc-agent
python -m venv .venv
source .venv/bin/activate
python -m pip install -e .
```

常驻服务：

```bash
function-doc-agent serve --root .. --web-root web
```

单轮运行：

```bash
function-doc-agent run --root .. --web-root web
```

状态：

```bash
function-doc-agent status --root ..
```

默认监听 `127.0.0.1:8787`。需要 `jq`、Python 3.10+、`openai-codex` 和已经可用的
Codex 登录。SDK 会自动安装其匹配的 Codex CLI runtime；凭据不会写入状态或日志。
