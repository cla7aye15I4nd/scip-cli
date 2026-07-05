(() => {
  "use strict";

  const LINE_HEIGHT = parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue("--line-height")
  ) || 22;
  const OVERSCAN = 60;
  const MAX_FILE_RESULTS = 400;
  const MAX_VIEW_HISTORY = 80;
  const fileMode = location.protocol === "file:";
  const appRoot = new URL("../", document.currentScript.src);
  const routeMode = fileMode || appRoot.pathname !== "/";
  const els = Object.fromEntries([
    "menu", "home-link", "sidebar", "search", "file-list", "file-note", "breadcrumb",
    "stats", "main", "empty", "editor", "code", "doc-panel", "doc-resizer", "doc-state", "doc-content",
    "code-spacer", "code-window", "status", "position-status", "language-status",
    "view-back", "view-forward", "view-history", "history-list", "clear-history",
    "agent-toggle", "agent-indicator", "agent-toggle-label", "agent-panel", "agent-close",
    "agent-run", "agent-state", "agent-usage", "agent-tasks-tab", "agent-bugs-tab", "agent-bug-count",
    "agent-tasks-view", "agent-bugs-view", "agent-running-count", "agent-pending-count", "agent-recent-count",
    "agent-running-list", "agent-pending-list", "agent-recent-list", "agent-bug-list", "agent-log", "agent-log-state"
  ].map(id => [id, document.getElementById(id)]));
  const nav = {
    code: document.getElementById("nav-code"),
    repositories: document.getElementById("nav-repositories"),
    vulnerabilities: document.getElementById("nav-vulnerabilities"),
    tasks: document.getElementById("nav-tasks")
  };
  if (routeMode) {
    const home = fileMode ? location.pathname : appRoot.pathname;
    els["home-link"].href = home;
    nav.code.href = "#/";
    nav.repositories.href = "#/repositories";
    nav.vulnerabilities.href = "#/vulnerabilities";
    nav.tasks.href = "#/tasks";
  }

  let catalog;
  let manifest;
  let currentProject = "";
  let currentId = -1;
  let currentData;
  let lines = [];
  let occurrencesByLine = [];
  let renderedStart = -1;
  let renderedEnd = -1;
  let pendingFrame = 0;
  let statusTimer = 0;
  let viewHistory = [];
  let viewHistoryCursor = -1;
  let functionDocs = new Map();
  let docObserver;
  const agentTasks = new Map();
  const agentBugs = new Map();
  const agentLogs = [];
  let agentConnected = false;
  let selectedAgentTask = "";
  let repositorySync;
  let panelResize;

  const DOC_WIDTH_KEY = "scip-doc-panel-width";
  const DEFAULT_DOC_WIDTH = 330;
  const MIN_DOC_WIDTH = 240;
  const MAX_DOC_WIDTH = 600;

  function setDocWidth(value, persist = false) {
    const width = Math.round(Math.min(MAX_DOC_WIDTH, Math.max(MIN_DOC_WIDTH, value)));
    document.documentElement.style.setProperty("--doc-width", `${width}px`);
    els["doc-resizer"].setAttribute("aria-valuemin", MIN_DOC_WIDTH);
    els["doc-resizer"].setAttribute("aria-valuemax", MAX_DOC_WIDTH);
    els["doc-resizer"].setAttribute("aria-valuenow", width);
    if (persist) {
      try { localStorage.setItem(DOC_WIDTH_KEY, width); } catch (_) { /* Storage is optional. */ }
    }
  }

  try {
    const savedDocWidth = Number(localStorage.getItem(DOC_WIDTH_KEY));
    setDocWidth(Number.isFinite(savedDocWidth) && savedDocWidth ? savedDocWidth : DEFAULT_DOC_WIDTH);
  } catch (_) {
    setDocWidth(DEFAULT_DOC_WIDTH);
  }

  const encodePath = path => path.split("/").map(encodeURIComponent).join("/");
  const projectRoot = () => `${routeMode ? "#/" : "/"}${encodeURIComponent(manifest.repoSlug)}/${encodeURIComponent(manifest.commit)}/`;
  const fileUrl = file => `${projectRoot()}${encodePath(file.path)}`;

  function parseRoute() {
    let parts;
    try {
      const route = routeMode ? location.hash.replace(/^#\/?/, "") : `${location.pathname.replace(/^\//, "")}${location.search}`;
      const [path, query = ""] = route.split("?", 2);
      parts = path.split("/").filter(Boolean).map(decodeURIComponent);
      var params = new URLSearchParams(query);
    } catch (_) {
      return { invalid: true };
    }
    const page = ["repositories", "vulnerabilities", "tasks"].includes(parts[0])
      ? parts[0]
      : parts[0] ? "repository" : "home";
    return {
      page,
      slug: parts[0] || "",
      commit: parts[1] || "",
      filePath: parts.slice(2).join("/"),
      line: Math.max(0, Number(params.get("line") || 0)),
      character: Math.max(0, Number(params.get("char") || 0)),
      hasCharacter: params.has("char")
    };
  }

  function setActiveNavigation(page) {
    nav.code.classList.toggle("active", page === "repository" || page === "home");
    nav.repositories.classList.toggle("active", page === "repositories");
    nav.vulnerabilities.classList.toggle("active", page === "vulnerabilities");
    nav.tasks.classList.toggle("active", page === "tasks");
  }

  function navigate(url, replace = false) {
    if (routeMode) {
      location.hash = url.startsWith("#") ? url.slice(1) : url;
      return;
    }
    history[replace ? "replaceState" : "pushState"]({}, "", url);
    openRoute();
  }

  function loadScript(path, globalName) {
    return new Promise((resolve, reject) => {
      delete window[globalName];
      const script = document.createElement("script");
      script.src = path;
      script.onload = () => {
        script.remove();
        const value = window[globalName];
        delete window[globalName];
        value ? resolve(value) : reject(new Error(`Missing ${globalName}`));
      };
      script.onerror = () => reject(new Error(`Unable to load ${path}`));
      document.head.append(script);
    });
  }

  function status(message) {
    els.status.textContent = message;
    els.status.classList.add("visible");
    clearTimeout(statusTimer);
    statusTimer = setTimeout(() => els.status.classList.remove("visible"), 2200);
  }

  function clearFunctionDocs(message = "Choose a source file to view its function documentation.") {
    functionDocs = new Map();
    els["doc-state"].textContent = "No file selected";
    const empty = document.createElement("p");
    empty.className = "doc-empty";
    empty.textContent = message;
    els["doc-content"].replaceChildren(empty);
  }

  function appendMarkdownInline(parent, text) {
    const pattern = /(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\([^)]+\))/g;
    let cursor = 0;
    for (const match of text.matchAll(pattern)) {
      parent.append(document.createTextNode(text.slice(cursor, match.index)));
      const value = match[0];
      if (value.startsWith("`")) {
        const code = document.createElement("code");
        code.textContent = value.slice(1, -1);
        parent.append(code);
      } else if (value.startsWith("**")) {
        const strong = document.createElement("strong");
        strong.textContent = value.slice(2, -2);
        parent.append(strong);
      } else {
        const parts = value.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
        const link = document.createElement("a");
        link.textContent = parts[1];
        if (/^(https?:|#)/.test(parts[2])) link.href = parts[2];
        parent.append(link);
      }
      cursor = match.index + value.length;
    }
    parent.append(document.createTextNode(text.slice(cursor)));
  }

  function renderMarkdown(markdown) {
    const root = document.createElement("div");
    root.className = "markdown-body";
    const sourceLines = String(markdown || "").replace(/\r\n?/g, "\n").split("\n");
    for (let index = 0; index < sourceLines.length;) {
      const line = sourceLines[index];
      if (!line.trim()) { index++; continue; }
      if (/^```/.test(line)) {
        const content = [];
        for (index++; index < sourceLines.length && !/^```/.test(sourceLines[index]); index++) content.push(sourceLines[index]);
        index++;
        const pre = document.createElement("pre");
        const code = document.createElement("code");
        code.textContent = content.join("\n");
        pre.append(code);
        root.append(pre);
        continue;
      }
      const heading = line.match(/^(#{1,3})\s+(.+)$/);
      if (heading) {
        const element = document.createElement(`h${heading[1].length}`);
        appendMarkdownInline(element, heading[2]);
        root.append(element);
        index++;
        continue;
      }
      if (/^\s*([-*_])(?:\s*\1){2,}\s*$/.test(line)) {
        root.append(document.createElement("hr"));
        index++;
        continue;
      }
      if (/^>\s?/.test(line)) {
        const quote = document.createElement("blockquote");
        const values = [];
        while (index < sourceLines.length && /^>\s?/.test(sourceLines[index])) values.push(sourceLines[index++].replace(/^>\s?/, ""));
        appendMarkdownInline(quote, values.join(" "));
        root.append(quote);
        continue;
      }
      const listMatch = line.match(/^\s*(?:([-*+])|(\d+)\.)\s+(.+)$/);
      if (listMatch) {
        const ordered = Boolean(listMatch[2]);
        const list = document.createElement(ordered ? "ol" : "ul");
        while (index < sourceLines.length) {
          const itemMatch = sourceLines[index].match(/^\s*(?:([-*+])|(\d+)\.)\s+(.+)$/);
          if (!itemMatch || Boolean(itemMatch[2]) !== ordered) break;
          const item = document.createElement("li");
          appendMarkdownInline(item, itemMatch[3]);
          list.append(item);
          index++;
        }
        root.append(list);
        continue;
      }
      const paragraph = [];
      while (index < sourceLines.length && sourceLines[index].trim() && !/^(#{1,3})\s|^```|^>\s?|^\s*(?:[-*+]|\d+\.)\s+/.test(sourceLines[index])) paragraph.push(sourceLines[index++].trim());
      const element = document.createElement("p");
      appendMarkdownInline(element, paragraph.join(" "));
      root.append(element);
    }
    return root;
  }

  function isFunctionSymbol(symbol) {
    return Boolean(symbol) && (Boolean(currentData?.functionDocKeys?.[symbol]) || (!symbol.startsWith("local ") && /\([^)]*\)\.$/.test(symbol)));
  }

  function functionMarkdown(item) {
    const stored = currentData.functionDocs?.[item.symbol] ?? currentData.docs?.[item.symbol];
    if (typeof stored === "string") return stored;
    if (stored && typeof stored.markdown === "string") return stored.markdown;
    return item.docKey
      ? "### Documentation\n\n> Waiting for the documentation agent."
      : "### Documentation\n\n> This function is not present in the generated function index.";
  }

  const listMarkdown = values => values?.length ? values.map(value => `- ${value}`).join("\n") : "- None identified.";

  function storedDocumentMarkdown(document) {
    const contract = document.contract || {};
    const implementation = document.implementation || {};
    const parameters = contract.parameters?.length
      ? contract.parameters.map(parameter => {
          const constraints = parameter.constraints?.length ? ` Constraints: ${parameter.constraints.join("; ")}` : "";
          return `- \`${parameter.name}\`: ${parameter.description}${constraints}`;
        }).join("\n")
      : "- No parameters documented.";
    const related = implementation.relatedFunctions?.length
      ? implementation.relatedFunctions.map(item => `- \`${item.scipSymbol}\` (${item.relationship}): ${item.description}`).join("\n")
      : "- None identified.";
    const bugs = document.possibleBugs?.length
      ? document.possibleBugs.map(bug => {
          const status = (bug.verification?.status || "unverified").toUpperCase();
          const evidence = bug.verification?.evidence?.length
            ? bug.verification.evidence.map(item => `${item.kind}: ${item.description} (${item.artifact})`).join("; ")
            : "No validation evidence recorded.";
          return `### [${status}] ${bug.title}\n\n**Severity:** ${bug.severity} · **Confidence:** ${Math.round((bug.confidence || 0) * 100)}%\n\n${bug.reason}\n\n**Trigger:** ${bug.trigger}\n\n**Impact:** ${bug.impact}\n\n**How to validate:** ${bug.validation}\n\n**Verification:** ${bug.verification?.summary || evidence}\n\n**Evidence:** ${evidence}`;
        }).join("\n\n")
      : "No possible bugs were identified with sufficient evidence.";
    const progress = document.status && document.status !== "completed"
      ? `> ${document.progress?.message || `Documentation is ${document.status}.`}\n\n`
      : "";
    return `${progress}## Contract\n\n${contract.summary || "This section has not been written yet."}\n\n### Parameters\n\n${parameters}\n\n### Returns\n\n${contract.returns || "Not documented."}\n\n### Preconditions\n\n${listMarkdown(contract.preconditions)}\n\n### Side effects\n\n${listMarkdown(contract.sideEffects)}\n\n### Errors\n\n${listMarkdown(contract.errors)}\n\n### Thread safety\n\n${contract.threadSafety || "unknown"}\n\n## Implementation\n\n${implementation.summary || "This section has not been written yet."}\n\n### Steps\n\n${listMarkdown(implementation.steps)}\n\n### Related functions\n\n${related}\n\n### Failure paths\n\n${listMarkdown(implementation.failurePaths)}\n\n### Complexity\n\n${implementation.complexity || "unknown"}\n\n## Possible bugs\n\n${bugs}`;
  }

  async function loadFunctionDocument(item) {
    if (!item.docKey || ["loading", "loaded", "missing"].includes(item.documentState)) return;
    item.documentState = "loading";
    try {
      const base = `generated/${encodeURIComponent(manifest.repoSlug)}/${encodeURIComponent(manifest.commit)}/docs/${encodeURIComponent(item.docKey)}`;
      let document;
      if (fileMode) {
        document = await loadScript(`${base}.js`, "__SCIP_FUNCTION_DOC__");
      } else {
        const response = await fetch(new URL(`${base}.json`, appRoot), { cache: "no-store" });
        if (response.status === 404) {
          item.documentState = "missing";
          return;
        }
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        document = await response.json();
      }
      if (currentData && item.article?.isConnected) {
        item.body.replaceChildren(renderMarkdown(storedDocumentMarkdown(document)));
        item.article.classList.toggle("documented", document.status === "completed");
        item.documentState = document.status === "completed" ? "loaded" : "loading";
      }
    } catch (_) {
      item.documentState = "missing";
    }
  }

  function applyLiveDocument(document) {
    for (const item of functionDocs.values()) {
      if (item.docKey !== document.docKey || !item.article?.isConnected) continue;
      item.body.replaceChildren(renderMarkdown(storedDocumentMarkdown(document)));
      item.article.classList.toggle("documented", document.status === "completed");
      item.documentState = document.status === "completed" ? "loaded" : "loading";
    }
  }

  function renderAgentPanel() {
    const tasks = [...agentTasks.values()].sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)));
    const running = tasks.filter(task => task.state === "running" || task.state === "preparing");
    const pending = tasks.filter(task => task.state === "queued" || task.state === "paused");
    const recent = tasks.filter(task => ["completed", "failed", "partial"].includes(task.state));
    const failed = tasks.filter(task => task.state === "failed" || task.state === "partial");
    els["agent-toggle"].classList.toggle("connected", agentConnected && !running.length && !failed.length);
    els["agent-toggle"].classList.toggle("running", running.length > 0);
    els["agent-toggle"].classList.toggle("failed", failed.length > 0 && running.length === 0);
    els["agent-toggle-label"].textContent = !agentConnected
      ? "Agent offline"
      : running.length
        ? `${running.length} task${running.length === 1 ? "" : "s"} running`
        : pending.length
          ? `${pending.length} pending`
          : "Agent idle";
    els["agent-state"].textContent = !agentConnected
      ? "Realtime service unavailable"
      : repositorySync?.state === "running"
        ? repositorySync.message
        : running[0]?.progress?.message || (pending.length ? `${pending.length} functions waiting for Codex` : "Ready for documentation work");
    const usage = tasks.reduce((sum, task) => sum + (task.usage?.inputTokens || 0) + (task.usage?.outputTokens || 0), 0);
    els["agent-usage"].textContent = usage ? `${usage.toLocaleString()} tokens` : "";

    els["agent-running-count"].textContent = running.length.toLocaleString();
    els["agent-pending-count"].textContent = pending.length.toLocaleString();
    els["agent-recent-count"].textContent = recent.length.toLocaleString();
    renderTaskRows(els["agent-running-list"], running, "No Codex task is currently running.", 20);
    renderTaskRows(els["agent-pending-list"], pending, "The documentation queue is empty.", 200);
    renderTaskRows(els["agent-recent-list"], recent, "No completed tasks yet.", 30);
    renderBugList();

    const logFragment = document.createDocumentFragment();
    for (const value of agentLogs.slice(-100)) {
      const row = document.createElement("div");
      row.className = `agent-log-entry ${value.entry.level}`;
      const time = document.createElement("time");
      time.textContent = new Date(value.entry.timestamp).toLocaleTimeString([], { hour12: false });
      const kind = document.createElement("b");
      kind.textContent = value.entry.kind;
      const message = document.createElement("span");
      message.textContent = value.entry.message;
      row.append(time, kind, message);
      logFragment.append(row);
    }
    els["agent-log"].replaceChildren(logFragment);
    els["agent-log"].scrollTop = els["agent-log"].scrollHeight;
    els["agent-log-state"].textContent = agentLogs.length ? `${agentLogs.length} events` : "Waiting for events";
  }

  function renderTaskRows(container, tasks, emptyMessage, limit) {
    const fragment = document.createDocumentFragment();
    for (const task of tasks.slice(0, limit)) {
      const row = document.createElement("article");
      row.className = `agent-task ${task.state}`;
      row.tabIndex = 0;
      row.title = "Show task log";
      row.addEventListener("click", () => loadAgentLogs(task.id));
      const dot = document.createElement("i");
      dot.className = "agent-task-dot";
      const copy = document.createElement("div");
      copy.className = "agent-task-copy";
      const name = document.createElement("strong");
      name.textContent = task.displayName;
      const detail = document.createElement("span");
      detail.textContent = `${task.progress?.stage || task.state} · ${task.file}`;
      const progress = document.createElement("span");
      progress.className = "agent-task-progress";
      const bar = document.createElement("i");
      const completed = Number(task.progress?.completedSections || 0);
      const total = Math.max(1, Number(task.progress?.totalSections || 3));
      bar.style.width = `${Math.min(100, Math.round(completed / total * 100))}%`;
      progress.append(bar);
      copy.append(name, detail, progress);
      const state = document.createElement("span");
      state.className = "agent-task-state";
      state.textContent = task.state;
      row.append(dot, copy, state);
      fragment.append(row);
    }
    if (!tasks.length) {
      const empty = document.createElement("p");
      empty.className = "agent-task-empty";
      empty.textContent = agentConnected ? emptyMessage : "Start the Python agent service to see live Codex tasks.";
      fragment.append(empty);
    } else if (tasks.length > limit) {
      const more = document.createElement("p");
      more.className = "agent-task-empty";
      more.textContent = `${(tasks.length - limit).toLocaleString()} more pending tasks are queued.`;
      fragment.append(more);
    }
    container.replaceChildren(fragment);
  }

  function renderBugList() {
    const bugs = [...agentBugs.values()].sort((a, b) => {
      const rank = { critical: 0, high: 1, medium: 2, low: 3 };
      return (rank[a.severity] ?? 4) - (rank[b.severity] ?? 4) || Number(b.confidence || 0) - Number(a.confidence || 0);
    });
    els["agent-bug-count"].textContent = bugs.length.toLocaleString();
    const fragment = document.createDocumentFragment();
    for (const bug of bugs) {
      const card = document.createElement("a");
      card.className = "agent-bug";
      card.href = routeMode ? `#${bug.url}` : bug.url;
      const head = document.createElement("div");
      head.className = "agent-bug-head";
      const title = document.createElement("strong");
      title.textContent = bug.title;
      const severity = document.createElement("span");
      severity.className = `agent-bug-severity ${String(bug.severity || "unknown").toLowerCase()}`;
      severity.textContent = bug.severity || "unknown";
      head.append(title, severity);
      const copy = document.createElement("div");
      copy.className = "agent-bug-copy";
      copy.textContent = bug.reason || bug.impact || "Potential issue reported by the documentation agent.";
      const meta = document.createElement("div");
      meta.className = "agent-bug-meta";
      const source = document.createElement("span");
      source.textContent = `${bug.repo} · ${bug.file}${bug.function ? ` · ${bug.function}` : ""}`;
      const verification = document.createElement("span");
      verification.className = "agent-bug-status";
      verification.textContent = bug.verification?.status || "unverified";
      meta.append(source, verification);
      card.append(head, copy, meta);
      fragment.append(card);
    }
    if (!bugs.length) {
      const empty = document.createElement("p");
      empty.className = "agent-task-empty";
      empty.textContent = agentConnected ? "No potential bugs have been reported for the current repositories." : "Potential bugs appear when the agent service is online.";
      fragment.append(empty);
    }
    els["agent-bug-list"].replaceChildren(fragment);
  }

  async function loadAgentLogs(taskId) {
    selectedAgentTask = taskId;
    try {
      const response = await fetch(new URL(`api/tasks/${encodeURIComponent(taskId)}/logs`, appRoot), { cache: "no-store" });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const data = await response.json();
      agentLogs.splice(0, agentLogs.length, ...(data.logs || []).map(entry => ({ taskId, entry })));
      renderAgentPanel();
    } catch (_) {
      els["agent-log-state"].textContent = "Unable to load logs";
    }
  }

  function replaceDocumentBugs(document) {
    for (const [id, bug] of agentBugs) {
      if (bug.docKey === document.docKey) agentBugs.delete(id);
    }
    const subject = document.subject || {};
    for (const [index, bug] of (document.possibleBugs || []).entries()) {
      const source = bug.source || {};
      const file = source.file || subject.file || "";
      const range = source.range || subject.definitionRange || [0, 0, 0, 0];
      const line = Number(range[0] || 0);
      const path = file.split("/").map(encodeURIComponent).join("/");
      const id = `${document.docKey}:${index}:${bug.title || "Potential bug"}`;
      agentBugs.set(id, {
        ...bug,
        id,
        docKey: document.docKey,
        repo: subject.repo,
        function: subject.displayName,
        file,
        verification: bug.verification || { status: "unverified" },
        url: `/${encodeURIComponent(subject.repo)}/${encodeURIComponent(subject.commit)}/${path}?line=${Math.max(0, line)}`
      });
    }
  }

  function initializeAgentMonitor() {
    if (fileMode || typeof EventSource === "undefined") {
      renderAgentPanel();
      return;
    }
    const source = new EventSource(new URL("api/events", appRoot));
    source.addEventListener("open", () => {
      agentConnected = true;
      renderAgentPanel();
      refreshStandalonePage();
    });
    source.addEventListener("error", () => {
      agentConnected = false;
      renderAgentPanel();
      refreshStandalonePage();
    });
    source.addEventListener("snapshot", event => {
      const data = JSON.parse(event.data);
      agentTasks.clear();
      for (const task of data.tasks || []) agentTasks.set(task.id, task);
      agentBugs.clear();
      for (const bug of data.bugs || []) agentBugs.set(bug.id, bug);
      repositorySync = data.repositorySync;
      agentConnected = true;
      renderAgentPanel();
      refreshStandalonePage();
      const active = [...agentTasks.values()].find(task => task.state === "running") || [...agentTasks.values()][0];
      if (active && active.id !== selectedAgentTask) loadAgentLogs(active.id);
    });
    source.addEventListener("task", event => {
      const data = JSON.parse(event.data);
      agentTasks.set(data.task.id, data.task);
      renderAgentPanel();
      refreshStandalonePage();
    });
    source.addEventListener("log", event => {
      const data = JSON.parse(event.data);
      agentLogs.push(data);
      if (agentLogs.length > 500) agentLogs.splice(0, agentLogs.length - 500);
      renderAgentPanel();
    });
    source.addEventListener("document", event => {
      const data = JSON.parse(event.data);
      applyLiveDocument(data.document);
      replaceDocumentBugs(data.document);
      renderAgentPanel();
      refreshStandalonePage();
    });
    source.addEventListener("state", event => {
      const data = JSON.parse(event.data);
      if (data.repositorySync) repositorySync = data.repositorySync;
      renderAgentPanel();
      refreshStandalonePage();
    });
  }

  function renderFunctionDocs() {
    docObserver?.disconnect();
    functionDocs = new Map();
    const functions = [];
    for (const occurrence of currentData.occurrences) {
      if (!(occurrence[8] & 1) || occurrence[4] < 0) continue;
      const symbol = currentData.symbols[occurrence[4]];
      if (!isFunctionSymbol(symbol) || functionDocs.has(symbol)) continue;
      const label = lines[occurrence[0]]?.slice(occurrence[1], occurrence[3]).trim() || symbol;
      const item = {
        symbol,
        label,
        line: occurrence[0],
        character: occurrence[1],
        signature: lines[occurrence[0]]?.trim() || label,
        id: `function-doc-${functions.length}`,
        docKey: currentData.functionDocKeys?.[symbol]
      };
      functions.push(item);
      functionDocs.set(symbol, item);
    }
    els["doc-state"].textContent = `${functions.length.toLocaleString()} function${functions.length === 1 ? "" : "s"}`;
    if (!functions.length) {
      clearFunctionDocs("No function definitions were identified in this file.");
      els["doc-state"].textContent = "0 functions";
      return;
    }
    const fragment = document.createDocumentFragment();
    const file = manifest.files[currentId];
    for (const item of functions) {
      const article = document.createElement("article");
      article.id = item.id;
      article.className = "markdown-doc";
      article.dataset.symbol = item.symbol;
      const link = document.createElement("a");
      link.className = "doc-function-link";
      link.href = `${fileUrl(file)}?line=${item.line}&char=${item.character}`;
      link.dataset.symbol = item.symbol;
      link.textContent = item.label;
      link.title = `Jump to ${currentData.path}:${item.line + 1}`;
      const signature = document.createElement("pre");
      signature.className = "doc-signature";
      signature.textContent = item.signature;
      const body = renderMarkdown(functionMarkdown(item));
      body.classList.add("function-doc-body");
      item.article = article;
      item.body = body;
      article.append(link, signature, body);
      fragment.append(article);
    }
    els["doc-content"].replaceChildren(fragment);
    docObserver = new IntersectionObserver(entries => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        const item = functionDocs.get(entry.target.dataset.symbol);
        if (item) loadFunctionDocument(item);
      }
    }, { root: els["doc-content"], rootMargin: "240px 0px" });
    for (const item of functions) docObserver.observe(item.article);
  }

  function scrollDocToSymbol(symbol) {
    const item = functionDocs.get(symbol);
    if (!item) return false;
    els["doc-content"].querySelectorAll(".markdown-doc.active").forEach(element => element.classList.remove("active"));
    const article = document.getElementById(item.id);
    loadFunctionDocument(item);
    article.classList.add("active");
    article.scrollIntoView({ behavior: "smooth", block: "start" });
    return true;
  }

  function renderViewHistory() {
    const fragment = document.createDocumentFragment();
    for (let index = viewHistory.length - 1; index >= 0; index--) {
      const view = viewHistory[index];
      const button = document.createElement("button");
      button.type = "button";
      button.className = `history-entry${index === viewHistoryCursor ? " current" : ""}${index > viewHistoryCursor ? " future" : ""}`;
      button.dataset.historyIndex = index;
      button.title = `${view.path}:${view.line + 1}`;
      const graph = document.createElement("span");
      graph.className = "history-graph";
      const node = document.createElement("span");
      node.className = "history-node";
      graph.append(node);
      const copy = document.createElement("span");
      copy.className = "history-copy";
      const name = document.createElement("strong");
      name.textContent = view.path.split("/").pop();
      const location = document.createElement("small");
      location.textContent = `Ln ${view.line + 1}`;
      copy.append(name, location);
      button.append(graph, copy);
      fragment.append(button);
    }
    if (!viewHistory.length) {
      const empty = document.createElement("p");
      empty.className = "history-empty";
      empty.textContent = "Open a source location to start a view trail.";
      fragment.append(empty);
    }
    els["history-list"].replaceChildren(fragment);
    els["view-back"].disabled = viewHistoryCursor <= 0;
    els["view-forward"].disabled = viewHistoryCursor < 0 || viewHistoryCursor >= viewHistory.length - 1;
    els["clear-history"].disabled = viewHistory.length === 0;
  }

  function recordView(target, file) {
    const key = `${target.slug}/${target.commit}/${file.path}?line=${target.line}&char=${target.character}`;
    if (viewHistory[viewHistoryCursor]?.key === key) {
      renderViewHistory();
      return;
    }
    if (viewHistory[viewHistoryCursor - 1]?.key === key) {
      viewHistoryCursor--;
    } else if (viewHistory[viewHistoryCursor + 1]?.key === key) {
      viewHistoryCursor++;
    } else {
      viewHistory.splice(viewHistoryCursor + 1);
      viewHistory.push({
        key,
        url: `${fileUrl(file)}?line=${target.line}&char=${target.character}`,
        path: file.path,
        line: target.line
      });
      if (viewHistory.length > MAX_VIEW_HISTORY) viewHistory.shift();
      viewHistoryCursor = viewHistory.length - 1;
    }
    els["view-history"].hidden = false;
    renderViewHistory();
  }

  function moveViewHistory(offset) {
    const next = viewHistoryCursor + offset;
    if (next < 0 || next >= viewHistory.length) return;
    viewHistoryCursor = next;
    renderViewHistory();
    navigate(viewHistory[next].url);
  }

  function showLanding() {
    setActiveNavigation("repositories");
    els.menu.hidden = true;
    manifest = undefined;
    currentProject = "";
    currentId = -1;
    currentData = undefined;
    els.sidebar.hidden = true;
    els["doc-panel"].hidden = true;
    els["view-history"].hidden = true;
    els.main.parentElement.classList.add("landing-mode");
    els.editor.hidden = true;
    els.empty.hidden = false;
    els.empty.className = "empty landing";
    els.empty.replaceChildren();
    const heading = document.createElement("div");
    heading.className = "landing-content";
    const hero = document.createElement("header");
    hero.className = "landing-hero";
    const heroCopy = document.createElement("div");
    heroCopy.className = "landing-hero-copy";
    const eyebrow = document.createElement("div");
    eyebrow.className = "landing-eyebrow";
    eyebrow.textContent = "Repository intelligence";
    const title = document.createElement("h1");
    title.textContent = "Understand every repository.";
    const intro = document.createElement("p");
    intro.textContent = "Search indexed source, follow symbols, and read generated function documentation without turning your browser into an IDE.";
    const totalFiles = catalog.projects.reduce((sum, project) => sum + (project.commits[0]?.fileCount || 0), 0);
    const totalSymbols = catalog.projects.reduce((sum, project) => sum + (project.commits[0]?.occurrenceCount || 0), 0);
    const metrics = document.createElement("div");
    metrics.className = "landing-metrics";
    for (const [value, label] of [
      [catalog.projects.length.toLocaleString(), "repositories"],
      [totalFiles.toLocaleString(), "indexed files"],
      [totalSymbols.toLocaleString(), "symbol links"]
    ]) {
      const metric = document.createElement("span");
      metric.innerHTML = `<strong>${value}</strong>${label}`;
      metrics.append(metric);
    }
    heroCopy.append(eyebrow, title, intro, metrics);
    const diagram = document.createElement("div");
    diagram.className = "landing-diagram";
    diagram.setAttribute("aria-hidden", "true");
    diagram.innerHTML = `<div class="diagram-label">symbol graph</div><div class="diagram-path path-a"></div><div class="diagram-path path-b"></div><i class="diagram-node node-a"></i><i class="diagram-node node-b"></i><i class="diagram-node node-c"></i><code>definition()</code><code>reference</code><code>docs.md</code>`;
    hero.append(heroCopy, diagram);
    const sectionHead = document.createElement("div");
    sectionHead.className = "project-section-head";
    const sectionTitle = document.createElement("h2");
    sectionTitle.textContent = "Browse repositories";
    const sectionHint = document.createElement("p");
    sectionHint.textContent = "Open the latest generated source index.";
    sectionHead.append(sectionTitle, sectionHint);
    const projects = document.createElement("div");
    projects.className = "project-grid";
    heading.append(hero, sectionHead, projects);
    for (const project of catalog.projects) {
      const card = document.createElement("section");
      card.className = "project-card";
      const cardHead = document.createElement("div");
      cardHead.className = "project-card-head";
      const mark = document.createElement("span");
      mark.className = "project-mark";
      const repoName = project.repoUrl.replace(/\/$/, "").split("/").pop() || project.slug;
      mark.textContent = repoName.slice(0, 1).toUpperCase();
      const nameWrap = document.createElement("div");
      const name = document.createElement("h2");
      name.textContent = repoName;
      const origin = document.createElement("p");
      origin.textContent = project.repoUrl;
      nameWrap.append(name, origin);
      const count = document.createElement("span");
      count.className = "revision-count";
      count.textContent = project.commits[0] ? `${project.commits[0].fileCount.toLocaleString()} files` : "Not indexed";
      cardHead.append(mark, nameWrap, count);
      card.append(cardHead);
      const revision = project.commits[0];
      if (revision) {
        const link = document.createElement("a");
        link.className = "commit-link";
        link.href = `${routeMode ? "#/" : "/"}${encodeURIComponent(project.slug)}/${encodeURIComponent(revision.commit)}/`;
        const commitCopy = document.createElement("span");
        commitCopy.className = "commit-copy";
        const revisionTitle = document.createElement("strong");
        revisionTitle.textContent = "Browse repository";
        const meta = document.createElement("span");
        meta.textContent = `${revision.fileCount.toLocaleString()} files · ${revision.occurrenceCount.toLocaleString()} symbols`;
        commitCopy.append(revisionTitle, meta);
        const arrow = document.createElement("span");
        arrow.className = "commit-arrow";
        arrow.textContent = "→";
        link.append(commitCopy, arrow);
        card.append(link);
      }
      projects.append(card);
    }
    els.empty.append(heading);
    els.breadcrumb.textContent = "Repositories";
    els.stats.textContent = `${catalog.projects.length.toLocaleString()} projects`;
    els["position-status"].textContent = "Ln 1, Col 1";
    els["language-status"].textContent = "SCIP";
    document.title = "Repositories · Source Atlas";
  }

  function prepareStandalonePage(page, breadcrumb, title) {
    setActiveNavigation(page);
    els.menu.hidden = true;
    manifest = undefined;
    currentProject = "";
    currentId = -1;
    currentData = undefined;
    clearFunctionDocs();
    els.sidebar.hidden = true;
    els["doc-panel"].hidden = true;
    els["view-history"].hidden = true;
    els.editor.hidden = true;
    els.main.parentElement.classList.add("landing-mode");
    els.empty.hidden = false;
    els.empty.className = "empty landing";
    els.empty.replaceChildren();
    els.breadcrumb.textContent = breadcrumb;
    els.stats.textContent = "";
    document.title = `${title} · Source Atlas`;
    const pageRoot = document.createElement("section");
    pageRoot.className = "product-page";
    els.empty.append(pageRoot);
    return pageRoot;
  }

  function appendPageHero(root, eyebrowText, titleText, description, action) {
    const hero = document.createElement("header");
    hero.className = "page-hero";
    const copy = document.createElement("div");
    const eyebrow = document.createElement("div");
    eyebrow.className = "page-eyebrow";
    eyebrow.textContent = eyebrowText;
    const title = document.createElement("h1");
    title.textContent = titleText;
    const intro = document.createElement("p");
    intro.textContent = description;
    copy.append(eyebrow, title, intro);
    hero.append(copy);
    if (action) hero.append(action);
    root.append(hero);
  }

  function metricCard(label, value, tone = "") {
    const card = document.createElement("div");
    card.className = `metric-card ${tone}`.trim();
    const name = document.createElement("span");
    name.textContent = label;
    const number = document.createElement("strong");
    number.textContent = Number(value).toLocaleString();
    card.append(name, number);
    return card;
  }

  function repositoryLabel(slug) {
    const project = catalog?.projects?.find(item => item.slug === slug);
    return project?.repoUrl?.replace(/\/$/, "").split("/").pop() || slug || "Unknown repository";
  }

  function showVulnerabilities() {
    const root = prepareStandalonePage("vulnerabilities", "Vulnerabilities", "Vulnerabilities");
    appendPageHero(
      root,
      "Security findings",
      "Vulnerabilities",
      "Review possible defects separately from source browsing. Prioritize by severity and verification status, then jump to the exact source location."
    );
    const allBugs = [...agentBugs.values()];
    const metrics = document.createElement("div");
    metrics.className = "metric-grid";
    metrics.append(
      metricCard("Total findings", allBugs.length),
      metricCard("Critical & high", allBugs.filter(bug => ["critical", "high"].includes(String(bug.severity).toLowerCase())).length, "danger"),
      metricCard("Unverified", allBugs.filter(bug => (bug.verification?.status || "unverified") === "unverified").length, "warning"),
      metricCard("Repositories", new Set(allBugs.map(bug => bug.repo)).size)
    );
    root.append(metrics);

    const filters = document.createElement("div");
    filters.className = "filter-bar";
    const search = document.createElement("input");
    search.type = "search";
    search.placeholder = "Search title, repository, function, or file";
    search.setAttribute("aria-label", "Search vulnerabilities");
    const severity = document.createElement("select");
    severity.setAttribute("aria-label", "Filter by severity");
    severity.innerHTML = '<option value="">All severities</option><option value="critical">Critical</option><option value="high">High</option><option value="medium">Medium</option><option value="low">Low</option>';
    const verification = document.createElement("select");
    verification.setAttribute("aria-label", "Filter by verification");
    verification.innerHTML = '<option value="">All verification</option><option value="verified">Verified</option><option value="unverified">Unverified</option><option value="refuted">Refuted</option>';
    filters.append(search, severity, verification);
    root.append(filters);
    const list = document.createElement("div");
    list.className = "finding-list";
    root.append(list);

    const render = () => {
      const needle = search.value.trim().toLowerCase();
      const rank = { critical: 0, high: 1, medium: 2, low: 3 };
      const bugs = allBugs.filter(bug => {
        const bugSeverity = String(bug.severity || "unknown").toLowerCase();
        const bugVerification = bug.verification?.status || "unverified";
        const haystack = [bug.title, bug.reason, bug.repo, bug.function, bug.file].join(" ").toLowerCase();
        return (!severity.value || bugSeverity === severity.value)
          && (!verification.value || bugVerification === verification.value)
          && (!needle || haystack.includes(needle));
      }).sort((a, b) => (rank[String(a.severity).toLowerCase()] ?? 4) - (rank[String(b.severity).toLowerCase()] ?? 4)
        || Number(b.confidence || 0) - Number(a.confidence || 0));
      const fragment = document.createDocumentFragment();
      for (const bug of bugs) {
        const card = document.createElement("a");
        card.className = "finding-card";
        card.href = routeMode ? `#${bug.url}` : bug.url;
        const marker = document.createElement("i");
        marker.className = `finding-marker ${String(bug.severity || "unknown").toLowerCase()}`;
        const main = document.createElement("div");
        main.className = "finding-main";
        const title = document.createElement("h2");
        title.textContent = bug.title || "Potential vulnerability";
        const reason = document.createElement("p");
        reason.textContent = bug.reason || bug.impact || "The agent reported a possible issue that requires review.";
        const meta = document.createElement("div");
        meta.className = "finding-meta";
        const confidence = Number(bug.confidence || 0);
        const confidencePercent = Math.round(confidence <= 1 ? confidence * 100 : confidence);
        for (const value of [
          repositoryLabel(bug.repo),
          bug.file || "Unknown file",
          bug.function || "Unknown function",
          `${String(bug.severity || "unknown").toUpperCase()} · ${confidencePercent}% confidence`
        ]) {
          const item = document.createElement("span");
          item.textContent = value;
          meta.append(item);
        }
        main.append(title, reason, meta);
        const status = document.createElement("span");
        status.className = "finding-status";
        status.textContent = bug.verification?.status || "unverified";
        card.append(marker, main, status);
        fragment.append(card);
      }
      if (!bugs.length) {
        const empty = document.createElement("div");
        empty.className = "empty-state";
        empty.textContent = allBugs.length
          ? "No findings match these filters."
          : agentConnected
            ? "No potential vulnerabilities have been reported for current repositories."
            : "Connect the documentation agent to load potential vulnerabilities.";
        fragment.append(empty);
      }
      list.replaceChildren(fragment);
    };
    search.addEventListener("input", render);
    severity.addEventListener("change", render);
    verification.addEventListener("change", render);
    render();
  }

  function runCard(task) {
    const card = document.createElement("article");
    card.className = `run-card ${task.state}`;
    const dot = document.createElement("i");
    dot.className = "run-dot";
    const copy = document.createElement("div");
    copy.className = "run-copy";
    const name = document.createElement("strong");
    name.textContent = task.displayName || task.symbol || "Documentation task";
    const detail = document.createElement("span");
    detail.textContent = `${repositoryLabel(task.repo)} · ${task.progress?.stage || task.state} · ${task.file || ""}`;
    copy.append(name, detail);
    card.append(dot, copy);
    return card;
  }

  function runColumn(title, tasks, emptyMessage, limit = 100) {
    const column = document.createElement("section");
    column.className = "run-column";
    const head = document.createElement("div");
    head.className = "run-column-head";
    const name = document.createElement("span");
    name.textContent = title;
    const count = document.createElement("span");
    count.textContent = tasks.length.toLocaleString();
    head.append(name, count);
    const list = document.createElement("div");
    list.className = "run-list";
    if (tasks.length) {
      for (const task of tasks.slice(0, limit)) list.append(runCard(task));
    } else {
      const empty = document.createElement("div");
      empty.className = "empty-state";
      empty.textContent = emptyMessage;
      list.append(empty);
    }
    column.append(head, list);
    return column;
  }

  async function requestAgentCycle() {
    try {
      const response = await fetch(new URL("api/run", appRoot), { method: "POST" });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      status("Documentation cycle requested");
    } catch (_) {
      status("Documentation agent is not available");
    }
  }

  function showTasks() {
    const action = document.createElement("button");
    action.className = "page-action";
    action.type = "button";
    action.textContent = "Run documentation cycle";
    action.addEventListener("click", requestAgentCycle);
    const root = prepareStandalonePage("tasks", "Agent runs", "Agent runs");
    appendPageHero(
      root,
      "Codex operations",
      "Agent runs",
      "Follow active documentation work, understand what is waiting, and review recently completed functions.",
      action
    );
    const tasks = [...agentTasks.values()].sort((a, b) => String(b.updatedAt).localeCompare(String(a.updatedAt)));
    const running = tasks.filter(task => ["running", "preparing"].includes(task.state));
    const pending = tasks.filter(task => ["queued", "paused"].includes(task.state));
    const recent = tasks.filter(task => ["completed", "failed", "partial"].includes(task.state));
    const metrics = document.createElement("div");
    metrics.className = "metric-grid";
    const totalUsage = tasks.reduce((sum, task) => sum + (task.usage?.inputTokens || 0) + (task.usage?.outputTokens || 0), 0);
    metrics.append(
      metricCard("Running", running.length),
      metricCard("Pending", pending.length, "warning"),
      metricCard("Completed", recent.filter(task => task.state === "completed").length),
      metricCard("Tokens", totalUsage)
    );
    root.append(metrics);
    const sync = document.createElement("div");
    sync.className = "sync-banner";
    const syncCopy = document.createElement("div");
    const syncTitle = document.createElement("strong");
    syncTitle.textContent = repositorySync?.state === "running" ? "Repository sync in progress" : "Repository sync";
    const syncMessage = document.createElement("span");
    syncMessage.textContent = repositorySync?.message || (agentConnected ? "Waiting for the next scheduled source refresh." : "Agent service is offline.");
    syncCopy.append(syncTitle, syncMessage);
    const syncState = document.createElement("span");
    syncState.className = "finding-status";
    syncState.textContent = repositorySync?.state || (agentConnected ? "idle" : "offline");
    sync.append(syncCopy, syncState);
    root.append(sync);
    const columns = document.createElement("div");
    columns.className = "run-columns";
    columns.append(
      runColumn("Running", running, "No task is running."),
      runColumn("Pending", pending, "The queue is empty.", 200),
      runColumn("Recent", recent, "No completed runs yet.", 40)
    );
    root.append(columns);
  }

  function refreshStandalonePage() {
    if (!catalog) return;
    const page = parseRoute().page;
    if (page === "vulnerabilities") showVulnerabilities();
    if (page === "tasks") showTasks();
  }

  function renderFileList(query = "") {
    const needle = query.trim().toLowerCase();
    const fragment = document.createDocumentFragment();
    const makeFileLink = file => {
      const link = document.createElement("a");
      link.className = `file-link${file.id === currentId ? " active" : ""}`;
      link.href = fileUrl(file);
      link.title = file.path;
      link.dataset.fileId = file.id;
      const icon = document.createElement("span");
      icon.className = `tree-file-icon lang-${(file.language || "text").toLowerCase()}`;
      icon.textContent = file.path.split(".").pop().slice(0, 2).toUpperCase();
      const label = document.createElement("span");
      label.textContent = needle ? file.path : file.path.split("/").pop();
      link.append(icon, label);
      return link;
    };
    if (needle) {
      let shown = 0;
      for (const file of manifest.files) {
        if (!file.path.toLowerCase().includes(needle)) continue;
        fragment.append(makeFileLink(file));
        if (++shown === MAX_FILE_RESULTS) break;
      }
    } else {
      const root = { folders: new Map(), files: [] };
      for (const file of manifest.files) {
        const parts = file.path.split("/");
        let node = root;
        for (const part of parts.slice(0, -1)) {
          if (!node.folders.has(part)) node.folders.set(part, { folders: new Map(), files: [] });
          node = node.folders.get(part);
        }
        node.files.push(file);
      }
      const activePath = currentId >= 0 ? manifest.files[currentId].path : "";
      const appendTree = (parent, node, depth, prefix) => {
        for (const [name, child] of [...node.folders].sort(([a], [b]) => a.localeCompare(b))) {
          const folder = document.createElement("details");
          folder.className = "tree-folder";
          const folderPath = `${prefix}${name}/`;
          folder.open = depth < 1 || activePath.startsWith(folderPath);
          const summary = document.createElement("summary");
          summary.innerHTML = `<span class="chevron">›</span><span class="folder-icon"></span>`;
          summary.append(document.createTextNode(name));
          folder.append(summary);
          const children = document.createElement("div");
          children.className = "tree-children";
          appendTree(children, child, depth + 1, folderPath);
          folder.append(children);
          parent.append(folder);
        }
        for (const file of node.files.sort((a, b) => a.path.localeCompare(b.path))) parent.append(makeFileLink(file));
      };
      appendTree(fragment, root, 0, "");
    }
    els["file-list"].replaceChildren(fragment);
    const totalMatches = needle
      ? manifest.files.reduce((n, file) => n + file.path.toLowerCase().includes(needle), 0)
      : manifest.files.length;
    els["file-note"].textContent = totalMatches > MAX_FILE_RESULTS
      ? `Showing ${MAX_FILE_RESULTS} of ${totalMatches.toLocaleString()} matches`
      : `${totalMatches.toLocaleString()} file${totalMatches === 1 ? "" : "s"}`;
  }

  function prepareOccurrences(data) {
    occurrencesByLine = Array.from({ length: lines.length }, () => []);
    for (const item of data.occurrences) {
      const [startLine, startChar, endLine, endChar] = item;
      const last = Math.min(endLine, lines.length - 1);
      for (let line = startLine; line <= last; line++) {
        const copy = item.slice();
        copy[0] = line;
        copy[1] = line === startLine ? startChar : 0;
        copy[2] = line;
        copy[3] = line === endLine ? endChar : lines[line].length;
        if (copy[3] > copy[1]) occurrencesByLine[line].push(copy);
      }
    }
    for (const items of occurrencesByLine) items.sort((a, b) => a[1] - b[1] || a[3] - b[3]);
  }

  const lexicalPattern = /(\/\/.*|\/\*.*|^\s*\*.*|\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b(?:alignas|alignof|asm|auto|bool|break|case|catch|char|class|const|constexpr|continue|default|delete|do|double|else|enum|explicit|extern|false|float|for|friend|if|inline|int|long|namespace|new|noexcept|nullptr|operator|private|protected|public|register|reinterpret_cast|return|short|signed|sizeof|static|struct|switch|template|this|throw|true|try|typedef|typename|union|unsigned|using|virtual|void|volatile|while)\b|\b(?:0x[\da-fA-F]+|\d+(?:\.\d+)?)\b)/gm;

  function appendLexed(parent, text) {
    let cursor = 0;
    lexicalPattern.lastIndex = 0;
    for (const match of text.matchAll(lexicalPattern)) {
      parent.append(document.createTextNode(text.slice(cursor, match.index)));
      const token = document.createElement("span");
      const value = match[0];
      const trimmed = value.trimStart();
      token.className = trimmed.startsWith("//") || trimmed.startsWith("/*") || trimmed.startsWith("*")
        ? "lex-comment"
        : value.startsWith("\"") || value.startsWith("'")
          ? "lex-string"
          : /^\d|^0x/i.test(value)
            ? "lex-number"
            : "lex-keyword";
      token.textContent = value;
      parent.append(token);
      cursor = match.index + value.length;
    }
    parent.append(document.createTextNode(text.slice(cursor)));
  }

  function appendSource(row, text, items) {
    const source = document.createElement("span");
    source.className = "source";
    let cursor = 0;
    for (const item of items) {
      const start = Math.max(cursor, Math.min(text.length, item[1]));
      const end = Math.max(start, Math.min(text.length, item[3]));
      if (end <= cursor) continue;
      appendLexed(source, text.slice(cursor, start));
      const targetFile = item[5];
      const symbol = item[4] >= 0 ? currentData.symbols[item[4]] : "";
      const syntax = item[9] || 0;
      const span = document.createElement(targetFile >= 0 ? "a" : "span");
      span.className = `token syntax-${syntax}${symbol ? " symbol" : ""}${item[8] & 1 ? " definition" : ""}${symbol && targetFile < 0 ? " external" : ""}`;
      if (symbol) span.title = targetFile < 0 ? `${symbol}\nDefinition is outside this index` : symbol;
      if (symbol) span.dataset.symbol = symbol;
      if (targetFile >= 0) {
        const target = manifest.files[targetFile];
        span.href = `${fileUrl(target)}?line=${item[6]}&char=${item[7]}`;
      }
      span.textContent = text.slice(start, end);
      source.append(span);
      cursor = end;
    }
    appendLexed(source, text.slice(cursor));
    row.append(source);
  }

  function renderWindow(force = false) {
    pendingFrame = 0;
    if (!currentData) return;
    const visibleStart = Math.floor(els.code.scrollTop / LINE_HEIGHT);
    const count = Math.ceil(els.code.clientHeight / LINE_HEIGHT);
    const start = Math.max(0, visibleStart - OVERSCAN);
    const end = Math.min(lines.length, visibleStart + count + OVERSCAN);
    if (!force && start === renderedStart && end === renderedEnd) return;
    renderedStart = start;
    renderedEnd = end;
    const fragment = document.createDocumentFragment();
    const focused = parseRoute().line;
    const currentFile = manifest.files[currentId];
    for (let index = start; index < end; index++) {
      const row = document.createElement("div");
      row.className = `line${index === focused ? " focused" : ""}`;
      row.style.top = `${index * LINE_HEIGHT}px`;
      const gutter = document.createElement("a");
      gutter.className = "gutter";
      gutter.href = `${fileUrl(currentFile)}?line=${index}`;
      gutter.textContent = index + 1;
      row.append(gutter);
      appendSource(row, lines[index], occurrencesByLine[index]);
      fragment.append(row);
    }
    els["code-window"].replaceChildren(fragment);
  }

  function scheduleWindow() {
    if (!pendingFrame) pendingFrame = requestAnimationFrame(() => renderWindow());
  }

  function centerLine(line) {
    const selected = Math.min(Math.max(0, line), Math.max(0, lines.length - 1));
    els.code.scrollTop = (selected + 0.5) * LINE_HEIGHT - els.code.clientHeight / 2;
  }

  function showProjectHome() {
    currentId = -1;
    currentData = undefined;
    clearFunctionDocs();
    els.editor.hidden = true;
    els.empty.hidden = false;
    els.empty.className = "empty";
    els.empty.textContent = "Choose a file to browse its source.";
    els.breadcrumb.textContent = manifest.repoUrl;
    document.title = manifest.title;
    els["position-status"].textContent = "Ln 1, Col 1";
    els["language-status"].textContent = "SCIP";
    renderFileList(els.search.value);
  }

  async function openRoute() {
    const target = parseRoute();
    if (target.invalid || target.page === "repositories") {
      showLanding();
      return;
    }
    if (target.page === "home") {
      const project = catalog.projects.find(item => /ffmpeg/i.test(item.repoUrl)) || catalog.projects[0];
      const revision = project?.commits[0];
      if (!project || !revision) {
        showLanding();
        return;
      }
      navigate(`${routeMode ? "#/" : "/"}${encodeURIComponent(project.slug)}/${encodeURIComponent(revision.commit)}/`, true);
      return;
    }
    if (target.page === "vulnerabilities") {
      showVulnerabilities();
      return;
    }
    if (target.page === "tasks") {
      showTasks();
      return;
    }
    setActiveNavigation("repository");
    els.menu.hidden = false;
    const project = catalog.projects.find(item => item.slug === target.slug);
    const revision = project?.commits.find(item => item.commit === target.commit);
    if (!project || !revision) {
      els.sidebar.hidden = true;
      els["doc-panel"].hidden = true;
      els["view-history"].hidden = true;
      els.editor.hidden = true;
      els.empty.hidden = false;
      els.empty.className = "empty";
      els.empty.textContent = "This repository version is not in the generated catalog.";
      return;
    }
    try {
      const key = `${target.slug}/${target.commit}`;
      if (key !== currentProject) {
        const base = `generated/${encodeURIComponent(target.slug)}/${encodeURIComponent(target.commit)}`;
        if (fileMode) {
          manifest = await loadScript(`${base}/manifest.js`, "__SCIP_MANIFEST__");
        } else {
          const response = await fetch(new URL(`${base}/manifest.json`, appRoot));
          if (!response.ok) throw new Error(`HTTP ${response.status}`);
          manifest = await response.json();
        }
        currentProject = key;
        currentId = -1;
        els.sidebar.hidden = false;
        els["doc-panel"].hidden = false;
        els["view-history"].hidden = false;
        els.main.parentElement.classList.remove("landing-mode");
        els.stats.textContent = `${manifest.fileCount.toLocaleString()} files · ${manifest.occurrenceCount.toLocaleString()} symbols`;
        renderFileList();
      }
      if (!target.filePath) {
        const firstFile = manifest.files[0];
        if (firstFile) {
          navigate(`${fileUrl(firstFile)}?line=0`, true);
          return;
        }
        showProjectHome();
        return;
      }
      const file = manifest.files.find(item => item.path === target.filePath);
      if (!file) throw new Error("file is not present in this index");
      if (file.id !== currentId) {
        els.empty.hidden = false;
        els.empty.className = "empty";
        els.empty.textContent = "Loading source…";
        els.editor.hidden = true;
        const base = `generated/${encodeURIComponent(target.slug)}/${encodeURIComponent(target.commit)}/files/${file.id}`;
        if (fileMode) {
          currentData = await loadScript(`${base}.js`, "__SCIP_FILE__");
        } else {
          const response = await fetch(new URL(`${base}.json`, appRoot));
          if (!response.ok) throw new Error(`HTTP ${response.status}`);
          currentData = await response.json();
        }
        currentId = file.id;
        lines = currentData.text.split("\n");
        prepareOccurrences(currentData);
        els.breadcrumb.textContent = `${manifest.repoUrl} / ${currentData.path}`;
        els["language-status"].textContent = currentData.language || "Plain Text";
        document.title = `${currentData.path} · ${manifest.title}`;
        els["code-spacer"].style.height = `${lines.length * LINE_HEIGHT}px`;
        els.empty.hidden = true;
        els.editor.hidden = false;
        renderedStart = renderedEnd = -1;
        renderFileList(els.search.value);
        renderFunctionDocs();
      }
      centerLine(target.line);
      els["position-status"].textContent = `Ln ${target.line + 1}, Col ${target.character + 1}`;
      renderWindow(true);
      recordView(target, file);
      const activeFile = document.querySelector(`.file-link[data-file-id="${file.id}"]`);
      if (activeFile) {
        const list = els["file-list"];
        const top = activeFile.offsetTop;
        if (top < list.scrollTop || top > list.scrollTop + list.clientHeight - activeFile.offsetHeight) {
          list.scrollTop = Math.max(0, top - list.clientHeight / 2);
        }
      }
      els.sidebar.classList.remove("open");
    } catch (error) {
      els.empty.hidden = false;
      els.empty.className = "empty";
      els.empty.textContent = `Unable to load source: ${error.message}`;
      els.editor.hidden = true;
      status("Could not load source data");
    }
  }

  async function start() {
    try {
      if (window.__SCIP_CATALOG__) {
        catalog = window.__SCIP_CATALOG__;
        delete window.__SCIP_CATALOG__;
      } else {
        const response = await fetch(new URL("generated/catalog.json", appRoot));
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        catalog = await response.json();
      }
      openRoute();
    } catch (error) {
      els.empty.textContent = `Unable to load generated catalog: ${error.message}`;
    }
  }

  document.addEventListener("click", event => {
    const link = event.target.closest("a[href]");
    if (!link || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || link.origin !== location.origin) return;
    if (routeMode) {
      if (link.hash) return;
      event.preventDefault();
      location.hash = "";
      showLanding();
      return;
    }
    event.preventDefault();
    navigate(`${link.pathname}${link.search}`);
  });
  els.search.addEventListener("input", event => renderFileList(event.target.value));
  els.search.addEventListener("keydown", event => {
    if (event.key === "Enter") els["file-list"].querySelector("a")?.click();
  });
  els.menu.addEventListener("click", () => els.sidebar.classList.toggle("open"));
  els["view-back"].addEventListener("click", () => moveViewHistory(-1));
  els["view-forward"].addEventListener("click", () => moveViewHistory(1));
  els["clear-history"].addEventListener("click", () => {
    viewHistory = [];
    viewHistoryCursor = -1;
    renderViewHistory();
  });
  els["agent-toggle"].addEventListener("click", () => {
    navigate(routeMode ? "#/tasks" : "/tasks");
  });
  els["agent-close"].addEventListener("click", () => {
    els["agent-panel"].hidden = true;
    els["agent-toggle"].setAttribute("aria-expanded", "false");
  });
  function selectAgentView(view) {
    const bugs = view === "bugs";
    els["agent-tasks-view"].hidden = bugs;
    els["agent-bugs-view"].hidden = !bugs;
    els["agent-tasks-tab"].classList.toggle("active", !bugs);
    els["agent-bugs-tab"].classList.toggle("active", bugs);
    els["agent-tasks-tab"].setAttribute("aria-selected", String(!bugs));
    els["agent-bugs-tab"].setAttribute("aria-selected", String(bugs));
  }
  els["agent-tasks-tab"].addEventListener("click", () => selectAgentView("tasks"));
  els["agent-bugs-tab"].addEventListener("click", () => selectAgentView("bugs"));
  els["agent-run"].addEventListener("click", requestAgentCycle);
  els["history-list"].addEventListener("click", event => {
    const entry = event.target.closest(".history-entry");
    if (!entry) return;
    const index = Number(entry.dataset.historyIndex);
    if (!Number.isInteger(index) || !viewHistory[index]) return;
    viewHistoryCursor = index;
    renderViewHistory();
    navigate(viewHistory[index].url);
  });
  els.code.addEventListener("scroll", scheduleWindow, { passive: true });
  els.code.addEventListener("contextmenu", event => {
    const symbolToken = event.target.closest(".symbol[data-symbol]");
    if (!symbolToken || !scrollDocToSymbol(symbolToken.dataset.symbol)) return;
    event.preventDefault();
    status(`Documentation: ${symbolToken.textContent}`);
  });
  els["doc-resizer"].addEventListener("pointerdown", event => {
    if (event.button !== 0) return;
    panelResize = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startWidth: els["doc-panel"].getBoundingClientRect().width
    };
    els["doc-resizer"].setPointerCapture(event.pointerId);
    document.body.classList.add("resizing-panels");
    event.preventDefault();
  });
  els["doc-resizer"].addEventListener("pointermove", event => {
    if (!panelResize || panelResize.pointerId !== event.pointerId) return;
    setDocWidth(panelResize.startWidth + panelResize.startX - event.clientX);
    scheduleWindow();
  });
  const finishPanelResize = event => {
    if (!panelResize || panelResize.pointerId !== event.pointerId) return;
    const width = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--doc-width"));
    panelResize = undefined;
    document.body.classList.remove("resizing-panels");
    setDocWidth(width, true);
  };
  els["doc-resizer"].addEventListener("pointerup", finishPanelResize);
  els["doc-resizer"].addEventListener("pointercancel", finishPanelResize);
  els["doc-resizer"].addEventListener("dblclick", () => setDocWidth(DEFAULT_DOC_WIDTH, true));
  els["doc-resizer"].addEventListener("keydown", event => {
    if (!['ArrowLeft', 'ArrowRight', 'Home'].includes(event.key)) return;
    const current = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--doc-width"));
    setDocWidth(event.key === 'Home' ? DEFAULT_DOC_WIDTH : current + (event.key === 'ArrowLeft' ? 20 : -20), true);
    scheduleWindow();
    event.preventDefault();
  });
  window.addEventListener("resize", scheduleWindow, { passive: true });
  window.addEventListener("popstate", openRoute);
  window.addEventListener("hashchange", openRoute);
  renderViewHistory();
  renderAgentPanel();
  initializeAgentMonitor();
  const startupParams = new URLSearchParams(location.search);
  if (startupParams.get("agent") === "1") navigate(routeMode ? "#/tasks" : "/tasks", true);
  start();
})();
