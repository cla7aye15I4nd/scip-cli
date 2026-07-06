(() => {
  "use strict";

  const LINE_HEIGHT = parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue("--line-height")
  ) || 22;
  const OVERSCAN = 60;
  const MAX_FILE_RESULTS = 400;
  const appRoot = new URL("../", document.currentScript.src);
  // Hash routes work on every static host without server-side SPA rewrites.
  const routeMode = true;
  const els = Object.fromEntries([
    "menu", "home-link", "sidebar", "search", "file-list", "file-note", "breadcrumb",
    "stats", "main", "empty", "editor", "code", "doc-panel", "doc-resizer", "doc-state", "doc-content",
    "code-spacer", "code-window", "status", "position-status", "language-status"
  ].map(id => [id, document.getElementById(id)]));
  if (routeMode) {
    els["home-link"].href = appRoot.pathname;
  }

  let catalog;
  let manifest;
  let currentProject = "";
  let currentId = -1;
  let currentData;
  let filesByPath = new Map();
  let lines = [];
  let occurrencesByLine = [];
  let renderedStart = -1;
  let renderedEnd = -1;
  let pendingFrame = 0;
  let statusTimer = 0;
  let functionDocs = new Map();
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
    return {
      page: parts[0] ? "repository" : "repositories",
      slug: parts[0] || "",
      commit: parts[1] || "",
      filePath: parts.slice(2).join("/"),
      line: Math.max(0, Number(params.get("line") || 0)),
      character: Math.max(0, Number(params.get("char") || 0)),
      hasCharacter: params.has("char")
    };
  }

  function navigate(url, replace = false) {
    if (routeMode) {
      const hash = url.startsWith("#") ? url : `#${url}`;
      if (replace) {
        history.replaceState({}, "", hash);
        openRoute();
      } else {
        location.hash = hash.slice(1);
      }
      return;
    }
    history[replace ? "replaceState" : "pushState"]({}, "", url);
    openRoute();
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
    return Boolean(symbol) && !symbol.startsWith("local ") && /\([^)]*\)\.$/.test(symbol);
  }

  function functionMarkdown(item) {
    const stored = currentData.functionDocs?.[item.symbol] ?? currentData.docs?.[item.symbol];
    if (typeof stored === "string") return stored;
    if (stored && typeof stored.markdown === "string") return stored.markdown;
    return "### Source symbol\n\nUse the code view and symbol links to inspect this function.";
  }

  function renderFunctionDocs() {
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
        id: `function-doc-${functions.length}`
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
  }

  function scrollDocToSymbol(symbol) {
    const item = functionDocs.get(symbol);
    if (!item) return false;
    els["doc-content"].querySelectorAll(".markdown-doc.active").forEach(element => element.classList.remove("active"));
    const article = document.getElementById(item.id);
    article.classList.add("active");
    article.scrollIntoView({ behavior: "smooth", block: "start" });
    return true;
  }

  function showLanding() {
    els.menu.hidden = true;
    manifest = undefined;
    currentProject = "";
    currentId = -1;
    currentData = undefined;
    filesByPath = new Map();
    els.sidebar.hidden = true;
    els["doc-panel"].hidden = true;
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
    eyebrow.textContent = "Static source browser";
    const title = document.createElement("h1");
    title.textContent = "Browse indexed source.";
    const intro = document.createElement("p");
    intro.textContent = "Search files, read source, follow definitions, and inspect symbol documentation without a backend service.";
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
    sectionHint.textContent = "Open the latest or previous generated source index.";
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
      project.commits.forEach((revision, index) => {
        const link = document.createElement("a");
        link.className = "commit-link";
        link.href = `${routeMode ? "#/" : "/"}${encodeURIComponent(project.slug)}/${encodeURIComponent(revision.commit)}/`;
        const commitCopy = document.createElement("span");
        commitCopy.className = "commit-copy";
        const revisionTitle = document.createElement("strong");
        revisionTitle.textContent = index === 0 ? "Latest" : "Previous";
        const meta = document.createElement("span");
        meta.textContent = `${revision.commit.slice(0, 12)} · ${revision.fileCount.toLocaleString()} files · ${revision.occurrenceCount.toLocaleString()} symbols`;
        commitCopy.append(revisionTitle, meta);
        const arrow = document.createElement("span");
        arrow.className = "commit-arrow";
        arrow.textContent = "→";
        link.append(commitCopy, arrow);
        card.append(link);
      });
      projects.append(card);
    }
    els.empty.append(heading);
    els.breadcrumb.textContent = "Repositories";
    els.stats.textContent = `${catalog.projects.length.toLocaleString()} projects`;
    els["position-status"].textContent = "Ln 1, Col 1";
    els["language-status"].textContent = "SCIP";
    document.title = "Repositories · SCIP-CLI";
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

  async function loadPackedFile(file) {
    if (!("DecompressionStream" in globalThis)) {
      throw new Error("this browser does not support gzip streams");
    }
    const start = Number(file.offset);
    const length = Number(file.length);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(length) || start < 0 || length <= 0) {
      throw new Error("source pack index is invalid");
    }
    const response = await fetch(new URL(manifest.packUrl, appRoot), {
      headers: { Range: `bytes=${start}-${start + length - 1}` },
      cache: "force-cache",
      credentials: "same-origin"
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    let compressed = await response.arrayBuffer();
    if (response.status === 200 && compressed.byteLength !== length) {
      if (compressed.byteLength < start + length) throw new Error("source pack is truncated");
      compressed = compressed.slice(start, start + length);
    } else if (compressed.byteLength !== length) {
      throw new Error("source pack range has an unexpected length");
    }
    const stream = new Blob([compressed])
      .stream()
      .pipeThrough(new DecompressionStream("gzip"));
    return new Response(stream).json();
  }

  async function openRoute() {
    const target = parseRoute();
    if (target.invalid || target.page === "repositories") {
      showLanding();
      return;
    }
    els.menu.hidden = false;
    const project = catalog.projects.find(item => item.slug === target.slug);
    const revision = project?.commits.find(item => item.commit === target.commit);
    if (!project || !revision) {
      els.sidebar.hidden = true;
      els["doc-panel"].hidden = true;
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
        const response = await fetch(new URL(`${base}/manifest.json`, appRoot));
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        manifest = await response.json();
        filesByPath = new Map(manifest.files.map(file => [file.path, file]));
        currentProject = key;
        currentId = -1;
        els.sidebar.hidden = false;
        els["doc-panel"].hidden = false;
        els.main.parentElement.classList.remove("landing-mode");
        els.stats.textContent = `${manifest.fileCount.toLocaleString()} files · ${manifest.occurrenceCount.toLocaleString()} symbols`;
        renderFileList();
      }
      if (!target.filePath) {
        const firstFile = manifest.files[0];
        if (firstFile) {
          navigate(fileUrl(firstFile), true);
        } else {
          els.sidebar.hidden = false;
          els["doc-panel"].hidden = true;
          els.editor.hidden = true;
          els.empty.hidden = false;
          els.empty.textContent = "This index contains no source files.";
        }
        return;
      }
      els.sidebar.hidden = false;
      els["doc-panel"].hidden = false;
      els.main.parentElement.classList.remove("landing-mode");
      const file = filesByPath.get(target.filePath);
      if (!file) throw new Error("file is not present in this index");
      if (file.id !== currentId) {
        els.empty.hidden = false;
        els.empty.className = "empty";
        els.empty.textContent = "Loading source…";
        els.editor.hidden = true;
        currentData = await loadPackedFile(file);
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
      const response = await fetch(new URL("generated/catalog.json", appRoot));
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      catalog = await response.json();
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
  start();
})();
