<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  type ClipItem = {
    id: number;
    kind?: "text" | "image";
    text: string;
    thumb?: string;
    width?: number;
    height?: number;
    lastCopiedAtMs: number;
  };

  let query = $state("");
  let items: ClipItem[] = $state([]);
  let total = $state(0);
  let offset = $state(0);
  let pageSize = $state(50);
  let loading = $state(true);
  let copying = $state(false);
  let error = $state("");
  let loadedQuery = $state<string | null>(null);
  let loadedOffset = $state(-1);
  let requestNumber = 0;
  let selectedId: number | null = $state(null);
  let confirmingClear = $state(false);
  let pop = $state(true);
  let searchInput: HTMLInputElement | undefined;

  async function refresh(q: string, pageOffset: number) {
    const request = ++requestNumber;
    loading = true;
    try {
      const result = await invoke<{ items: ClipItem[]; total: number; pageSize: number }>("history", { query: q, offset: pageOffset });
      if (request !== requestNumber || q !== query || pageOffset !== offset) return;
      items = result.items;
      total = result.total;
      pageSize = result.pageSize;
      loadedQuery = q;
      loadedOffset = pageOffset;
      if (offset >= total && offset > 0) {
        offset = Math.max(0, Math.floor((total - 1) / pageSize) * pageSize);
      }
      if (selectedId !== null && !items.some((item) => item.id === selectedId)) selectedId = null;
    } catch (cause) {
      if (request === requestNumber) {
        error = `Cannot load history: ${cause}`;
        loadedQuery = null;
      }
    } finally {
      if (request === requestNumber) loading = false;
    }
  }

  async function yoinkItem(id: number) {
    if (loading || copying || loadedQuery !== query || loadedOffset !== offset) return;
    copying = true;
    error = "";
    try {
      await invoke("yoink", { id });
      query = "";
      offset = 0;
    } catch (cause) {
      error = `Cannot copy item: ${cause}`;
    } finally {
      copying = false;
    }
  }

  async function deleteItem(id: number) {
    error = "";
    try {
      await invoke("delete_item", { id });
      await refresh(query, offset);
    } catch (cause) {
      error = `Cannot remove item: ${cause}`;
    }
  }

  async function clearHistory() {
    if (!confirmingClear) {
      confirmingClear = true;
      setTimeout(() => (confirmingClear = false), 3000);
      return;
    }
    confirmingClear = false;
    error = "";
    try {
      await invoke("clear_history");
      await refresh(query, offset);
    } catch (cause) {
      error = `Cannot clear history: ${cause}`;
    }
  }

  function replayPop() {
    pop = false;
    requestAnimationFrame(() => requestAnimationFrame(() => (pop = true)));
  }

  function onSearchKeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && items.length > 0) {
      event.preventDefault();
      void yoinkItem(items[0].id);
    }
  }

  function onGlobalKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      void getCurrentWindow().hide().catch((cause) => (error = `Cannot hide window: ${cause}`));
    } else if (
      event.key === "c" &&
      (event.ctrlKey || event.metaKey) &&
      selectedId !== null &&
      !(event.target instanceof HTMLElement && event.target.closest("input, textarea, [contenteditable]")) &&
      !window.getSelection()?.toString()
    ) {
      event.preventDefault();
      void yoinkItem(selectedId);
    }
  }

  function timeAgo(ms: number): string {
    const seconds = Math.floor((Date.now() - ms) / 1000);
    if (seconds < 60) return "just now";
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
    if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
    return new Date(ms).toLocaleDateString();
  }

  $effect(() => {
    void refresh(query, offset);
  });

  // Visibility avoids refreshes on every pointer entry with focus-follows-mouse.
  function onVisibilityChange() {
    if (document.visibilityState === "visible") {
      replayPop();
      void refresh(query, offset);
      searchInput?.focus();
      searchInput?.select();
    }
  }

  onMount(() => {
    searchInput?.focus();
    const unlisten = listen("history-changed", () => refresh(query, offset));
    void unlisten.catch((cause) => (error = `Cannot watch history: ${cause}`));
    document.addEventListener("visibilitychange", onVisibilityChange);
    window.addEventListener("keydown", onGlobalKeydown);
    return () => {
      requestNumber++;
      void unlisten.then((fn) => fn()).catch(() => {});
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.removeEventListener("keydown", onGlobalKeydown);
    };
  });
</script>

<main class="panel" class:pop>
  <header>
    <div class="search">
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
      >
        <circle cx="11" cy="11" r="7" />
        <path d="m21 21-4.3-4.3" />
      </svg>
      <input
        bind:this={searchInput}
        bind:value={query}
        oninput={() => { offset = 0; selectedId = null; }}
        onkeydown={onSearchKeydown}
        type="search"
        placeholder="Search yoink"
        aria-label="Search clipboard history"
        spellcheck="false"
      />
    </div>
  </header>

  {#if error}
    <div class="error" role="alert">
      <span>{error}</span>
      <button onclick={() => { error = ""; void refresh(query, offset); }}>Retry</button>
    </div>
  {/if}

  <ul aria-busy={loading}>
    {#each items as item (item.id)}
      <li>
        <button
          class="item"
          class:selected={selectedId === item.id}
          disabled={loading || copying}
          onclick={() => (selectedId = item.id)}
          ondblclick={() => yoinkItem(item.id)}
          title="Double-click to copy, or select and press Ctrl+C"
        >
          {#if item.kind === "image" && item.thumb}
            <img class="shot" src={item.thumb} alt="clipboard capture" />
            <span class="meta">
              {item.width}×{item.height} · {timeAgo(item.lastCopiedAtMs)}
            </span>
          {:else}
            <span class="text">{item.text}</span>
            <span class="meta">{timeAgo(item.lastCopiedAtMs)}</span>
          {/if}
        </button>
        <button
          class="delete"
          onclick={() => deleteItem(item.id)}
          title="Remove from history"
          aria-label="Remove item from history"
          disabled={loading || copying}
        >
          ✕
        </button>
      </li>
    {:else}
      <li class="empty">
        {loading ? "Loading…" : query ? "No matches." : "Nothing yoinked yet. Copy something!"}
      </li>
    {/each}
  </ul>

  {#if total > pageSize}
    <nav aria-label="History pages">
      <button disabled={offset === 0 || loading || copying} onclick={() => { offset -= pageSize; selectedId = null; }}>Newer</button>
      <span>{offset + 1}–{Math.min(offset + items.length, total)} of {total}</span>
      <button disabled={offset + items.length >= total || loading || copying} onclick={() => { offset += pageSize; selectedId = null; }}>Older</button>
    </nav>
  {/if}

  <footer>
    <span class="hints">
      <kbd>↵</kbd> top
      <kbd>Ctrl</kbd><kbd>C</kbd> copy
      <kbd>Esc</kbd> hide
    </span>
    <span class="footer-right">
      {total || "no"} {query ? "match" : "item"}{total === 1 ? "" : query ? "es" : "s"}
      {#if total > 0 && !query}
        <button class="clear" onclick={clearHistory} disabled={loading || copying}>
          {confirmingClear ? "Sure?" : "Clear all"}
        </button>
      {/if}
    </span>
  </footer>
</main>

<style>
  :root {
    --bg: #f7f7f8;
    --surface: #ffffff;
    --text: #1b1b1f;
    --muted: #74747c;
    --border: #e3e3e8;
    --accent: #f90;
    --danger: #d3453c;
  }

  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #212125;
      --surface: #2b2b30;
      --text: #ececf0;
      --muted: #8f8f98;
      --border: #3a3a41;
    }
  }

  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    background: var(--bg);
    color: var(--text);
    font-family:
      system-ui,
      -apple-system,
      "Segoe UI",
      Roboto,
      sans-serif;
    font-size: 14px;
  }

  .panel {
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    background: var(--bg);
  }

  .panel.pop {
    animation: pop 140ms ease-out;
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: scale(0.985) translateY(-4px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }

  header {
    padding: 12px;
    border-bottom: 1px solid var(--border);
  }

  .search {
    position: relative;
    color: var(--muted);
  }

  .search svg {
    position: absolute;
    left: 12px;
    top: 50%;
    transform: translateY(-50%);
    pointer-events: none;
  }

  input {
    width: 100%;
    box-sizing: border-box;
    padding: 10px 12px 10px 36px;
    border: 1.5px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
    color: var(--text);
    font-size: 15px;
    outline: none;
    transition:
      border-color 120ms ease,
      box-shadow 120ms ease;
  }

  input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 25%, transparent);
  }

  ul {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    margin: 0;
    padding: 6px;
    list-style: none;
  }

  /* Non-overlay scrollbar: WebKitGTK's overlay bar hit-tests a wide strip over the list edge and swallows hovers on the delete badge. */
  ul::-webkit-scrollbar {
    width: 8px;
  }

  ul::-webkit-scrollbar-thumb {
    background: var(--border);
    border-radius: 4px;
  }

  ul::-webkit-scrollbar-thumb:hover {
    background: var(--muted);
  }

  li {
    position: relative;
    display: flex;
    margin-bottom: 4px;
  }

  li.empty {
    justify-content: center;
    padding: 24px 0;
    color: var(--muted);
  }

  .item {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    transition:
      border-color 120ms ease,
      background 120ms ease;
  }

  .item:hover {
    border-color: var(--accent);
  }

  .item.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent);
  }

  .text {
    display: -webkit-box;
    line-clamp: 3;
    -webkit-line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .meta {
    color: var(--muted);
    font-size: 11px;
  }

  .shot {
    max-width: 100%;
    max-height: 140px;
    object-fit: contain;
    align-self: flex-start;
    border-radius: 4px;
  }

  .delete {
    position: absolute;
    top: -6px;
    right: -6px;
    width: 20px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: var(--surface);
    color: var(--muted);
    font-size: 11px;
    line-height: 1;
    cursor: pointer;
    visibility: hidden;
    transition:
      color 120ms ease,
      border-color 120ms ease;
  }

  li:hover .delete,
  li:focus-within .delete {
    visibility: visible;
  }

  .delete:hover {
    color: var(--danger);
    border-color: var(--danger);
  }

  footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
  }

  .hints {
    display: flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
    overflow: hidden;
  }

  kbd {
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: 4px;
    background: var(--surface);
    color: var(--muted);
    font-family: inherit;
    font-size: 10px;
  }

  kbd + kbd {
    margin-left: -2px;
  }

  .footer-right {
    display: flex;
    align-items: center;
    gap: 10px;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .clear {
    border: none;
    background: none;
    color: var(--muted);
    font-size: 12px;
    cursor: pointer;
    transition: color 120ms ease;
  }

  .clear:hover {
    color: var(--danger);
  }

  nav, .error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }

  nav button, .error button {
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 4px 8px;
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }

  .error { color: var(--danger); }
  .error span { overflow-wrap: anywhere; }
  button:disabled { opacity: 0.5; cursor: default; }
</style>
