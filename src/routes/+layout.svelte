<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import { unregisterAll } from "@tauri-apps/plugin-global-shortcut";
  import { afterNavigate } from "$app/navigation";
  import { page } from "$app/stores";
  import { invoke } from "@tauri-apps/api/core";
  import { bindYapperShortcuts } from "$lib/shortcuts";
  import { applyUiTheme, loadUiTheme } from "$lib/theme";

  interface Props {
    children?: import("svelte").Snippet;
  }
  let { children }: Props = $props();

  let status = $state("");
  /** Sidebar hint: how this install is primarily used */
  let instanceTag = $state("this device");

  async function loadInstanceTag() {
    try {
      const role =
        (await invoke<string | null>("get_setting_cmd", { key: "instance_role" })) ?? "dictation";
      instanceTag = role === "network_server" ? "network host" : "this device";
    } catch {
      instanceTag = "this device";
    }
  }

  const nav = [
    { href: "/", label: "Home" },
    { href: "/transcribe", label: "Transcribe" },
    { href: "/dictionary", label: "Dictionary" },
    { href: "/settings", label: "Settings" },
  ];

  /** Windows + devUrl: taskbar can stay on WebView2's blue placeholder until icon is set after load. */
  async function reapplyWindowIconFromBundle() {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const { defaultWindowIcon } = await import("@tauri-apps/api/app");
      const icon = await defaultWindowIcon();
      if (icon) await getCurrentWindow().setIcon(icon);
    } catch {
      /* browser / non-Tauri */
    }
  }

  onMount(() => {
    let cancelled = false;
    void reapplyWindowIconFromBundle();
    void loadInstanceTag();
    void (async () => {
      const mode = await loadUiTheme();
      if (!cancelled) applyUiTheme(mode);
    })();

    if (typeof window !== "undefined" && window.location.pathname === "/hud") {
      return () => {
        cancelled = true;
      };
    }
    (async () => {
      try {
        await bindYapperShortcuts((s) => {
          if (!cancelled) status = s;
        });
      } catch (e) {
        if (!cancelled) status = String(e);
      }
    })();
    return () => {
      cancelled = true;
      void unregisterAll();
    };
  });

  afterNavigate(({ from }) => {
    if (from?.url.pathname === "/settings") void loadInstanceTag();
  });
</script>

{#if $page.url.pathname === "/hud"}
  {@render children?.()}
{:else}
<div class="shell">
  <aside class="sidebar">
    <div class="brand">
      <img class="mark" src="/yapper-mouth.png" alt="" width="40" height="40" />
      <div>
        <div class="name">Yapper</div>
        <div class="tag">{instanceTag}</div>
      </div>
    </div>
    <nav>
      {#each nav as item}
        <a
          href={item.href}
          class:active={$page.url.pathname === item.href}
          data-sveltekit-preload-data="tap">{item.label}</a>
      {/each}
    </nav>
    {#if status}
      <p class="hint">{status}</p>
    {/if}
  </aside>
  <main class="main">
    {@render children?.()}
  </main>
</div>
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: 220px 1fr;
    min-height: 100vh;
  }
  .sidebar {
    padding: 1.5rem 1.25rem;
    border-right: 1px solid var(--border);
    background: linear-gradient(180deg, var(--bg-elevated) 0%, var(--bg) 100%);
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
  }
  .brand {
    display: flex;
    gap: 0.75rem;
    align-items: center;
  }
  .mark {
    width: 2.25rem;
    height: 2.25rem;
    object-fit: contain;
    flex-shrink: 0;
    display: block;
  }
  .name {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: 1.35rem;
    letter-spacing: -0.03em;
  }
  .tag {
    font-size: 0.72rem;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.12em;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  nav a {
    padding: 0.5rem 0.65rem;
    border-radius: 8px;
    color: var(--text-muted);
    font-weight: 500;
    text-decoration: none;
    transition: background 0.12s, color 0.12s;
  }
  nav a:hover {
    background: var(--bg);
    color: var(--text);
  }
  nav a.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--text);
  }
  .hint {
    margin-top: auto;
    font-size: 0.78rem;
    color: var(--text-muted);
    line-height: 1.35;
  }
  .main {
    padding: 2rem 2.25rem;
    overflow: auto;
  }
</style>
