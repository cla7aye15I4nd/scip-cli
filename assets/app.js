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
    "menu", "explorer-home", "sidebar", "search", "file-list", "file-note", "breadcrumb",
    "stats", "main", "empty", "editor", "file-name", "file-meta", "code",
    "code-spacer", "code-window", "status", "position-status", "language-status",
    "view-back", "view-forward", "view-history", "history-list", "clear-history"
  ].map(id => [id, document.getElementById(id)]));
  if (routeMode) {
    const home = fileMode ? location.pathname : appRoot.pathname;
    els["explorer-home"].href = home;
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
      slug: parts[0] || "",
      commit: parts[1] || "",
      filePath: parts.slice(2).join("/"),
      line: Math.max(0, Number(params.get("line") || 0)),
      character: Math.max(0, Number(params.get("char") || 0))
    };
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
      location.textContent = `${view.path} · line ${view.line + 1}`;
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
    manifest = undefined;
    currentProject = "";
    currentId = -1;
    currentData = undefined;
    els.sidebar.hidden = true;
    els["view-history"].hidden = true;
    els.main.parentElement.classList.add("landing-mode");
    els.editor.hidden = true;
    els.empty.hidden = false;
    els.empty.className = "empty landing";
    els.empty.replaceChildren();
    const heading = document.createElement("div");
    heading.className = "landing-content";
    const title = document.createElement("h1");
    title.textContent = "Indexed repositories";
    const intro = document.createElement("p");
    intro.textContent = "Choose a repository and commit to browse its source code.";
    const projects = document.createElement("div");
    projects.className = "project-grid";
    heading.append(title, intro, projects);
    for (const project of catalog.projects) {
      const card = document.createElement("section");
      card.className = "project-card";
      const name = document.createElement("h2");
      name.textContent = project.repoUrl;
      card.append(name);
      for (const revision of project.commits) {
        const link = document.createElement("a");
        link.className = "commit-link";
        link.href = `${routeMode ? "#/" : "/"}${encodeURIComponent(project.slug)}/${encodeURIComponent(revision.commit)}/`;
        const commit = document.createElement("code");
        commit.textContent = revision.commit;
        const meta = document.createElement("span");
        meta.textContent = `${revision.fileCount.toLocaleString()} files · ${revision.occurrenceCount.toLocaleString()} symbols`;
        link.append(commit, meta);
        card.append(link);
      }
      projects.append(card);
    }
    els.empty.append(heading);
    els.breadcrumb.textContent = "Repositories";
    els.stats.textContent = `${catalog.projects.length.toLocaleString()} projects`;
    els["position-status"].textContent = "Ln 1, Col 1";
    els["language-status"].textContent = "SCIP";
    document.title = "SCIP source browser";
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
    els.editor.hidden = true;
    els.empty.hidden = false;
    els.empty.className = "empty";
    els.empty.textContent = "Choose a file to browse its source.";
    els.breadcrumb.textContent = `${manifest.repoUrl} · ${manifest.commit}`;
    document.title = `${manifest.title} · ${manifest.commit}`;
    els["position-status"].textContent = "Ln 1, Col 1";
    els["language-status"].textContent = "SCIP";
    renderFileList(els.search.value);
  }

  async function openRoute() {
    const target = parseRoute();
    if (target.invalid || !target.slug) {
      showLanding();
      return;
    }
    const project = catalog.projects.find(item => item.slug === target.slug);
    const revision = project?.commits.find(item => item.commit === target.commit);
    if (!project || !revision) {
      els.sidebar.hidden = true;
      els["view-history"].hidden = true;
      els.editor.hidden = true;
      els.empty.hidden = false;
      els.empty.className = "empty";
      els.empty.textContent = "This repository or commit is not in the generated catalog.";
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
        els.main.parentElement.classList.remove("landing-mode");
        els.stats.textContent = `${manifest.fileCount.toLocaleString()} files · ${manifest.occurrenceCount.toLocaleString()} symbols`;
        renderFileList();
      }
      if (!target.filePath) {
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
        els["file-name"].textContent = currentData.path.split("/").pop();
        els["file-meta"].textContent = `${currentData.language || "Text"} · ${lines.length.toLocaleString()} lines`;
        els["language-status"].textContent = currentData.language || "Plain Text";
        document.title = `${currentData.path} · ${manifest.title}`;
        els["code-spacer"].style.height = `${lines.length * LINE_HEIGHT}px`;
        els.empty.hidden = true;
        els.editor.hidden = false;
        renderedStart = renderedEnd = -1;
        renderFileList(els.search.value);
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
  window.addEventListener("resize", scheduleWindow, { passive: true });
  window.addEventListener("popstate", openRoute);
  window.addEventListener("hashchange", openRoute);
  renderViewHistory();
  start();
})();
