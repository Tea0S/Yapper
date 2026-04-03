<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  type HudPhase = "hidden" | "idle" | "listening" | "transcribing";
  type MicLevel = { rms: number; peak: number };

  let phase = $state<HudPhase>("idle");
  let mic = $state<MicLevel>({ rms: 0, peak: 0 });
  let pttHint = $state("Push-to-talk");

  const dotCount = 9;

  const expanded = $derived(phase === "listening" || phase === "transcribing");

  function formatShortcut(raw: string): string {
    return raw
      .split("+")
      .map((t) => {
        const u = t.trim().toLowerCase();
        if (u === "control") return "Ctrl";
        if (u === "super") return "Win";
        if (u === "alt") return "Alt";
        if (u === "shift") return "Shift";
        if (u.startsWith("digit")) return u.slice(5);
        if (u.startsWith("key")) return u.slice(3).toUpperCase();
        return t.trim();
      })
      .join(" + ");
  }

  function dotLevel(i: number): number {
    const center = (dotCount - 1) / 2;
    const dist = Math.abs(i - center) / Math.max(center, 1);
    const rest = 0.12 + (1 - dist) * 0.28;
    const e = Math.min(1, mic.peak * 3.2 + mic.rms * 5.5);
    return Math.min(1, rest + e * (0.55 + (1 - dist) * 0.45));
  }

  async function openYapper() {
    try {
      await invoke("focus_main_window");
    } catch {
      /* ignore */
    }
  }

  onMount(() => {
    let dead = false;
    void invoke<{ action: string; shortcut: string }[]>("list_keybinds_cmd")
      .then((rows) => {
        if (dead) return;
        const row = rows.find((r) => r.action === "push_to_talk" && r.shortcut.trim());
        if (row) pttHint = formatShortcut(row.shortcut);
      })
      .catch(() => {});

    const tick = async () => {
      if (dead) return;
      try {
        const snap = await invoke<{ phase: HudPhase }>("hud_snapshot");
        phase = snap.phase;
      } catch {
        phase = "hidden";
      }
      if (phase === "listening") {
        try {
          mic = await invoke<MicLevel>("get_mic_input_level");
        } catch {
          mic = { rms: 0, peak: 0 };
        }
      }
    };
    const id = setInterval(tick, 72);
    void tick();
    return () => {
      dead = true;
      clearInterval(id);
    };
  });
</script>

<div class="hud-root">
  <div class="stack">
    <div class="tooltip" role="tooltip">
      <span class="tip-line"
        >Hold <strong class="accent">{pttHint}</strong> to dictate · release to transcribe</span
      >
      <span class="tip-sub">Click to open Yapper</span>
    </div>
    <button
      type="button"
      class="pill"
      class:expanded
      onclick={openYapper}
      aria-label="Open Yapper"
    >
      {#if expanded}
        <div class="dots" aria-hidden="true">
          {#each Array.from({ length: dotCount }, (_, i) => i) as i (i)}
            <span
              class="dot"
              class:busy={phase === "transcribing"}
              style="--lvl: {phase === 'listening' ? dotLevel(i) : 0.22}"
            ></span>
          {/each}
        </div>
      {:else}
        <span class="idle-cap" aria-hidden="true"></span>
      {/if}
    </button>
  </div>
</div>

<style>
  :global(html),
  :global(body) {
    background: transparent !important;
    margin: 0;
    min-height: 100%;
    overflow: hidden;
  }

  .hud-root {
    box-sizing: border-box;
    min-height: 100vh;
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    padding: 4px 6px 10px;
    font-family: "DM Sans", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
  }

  .stack {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
  }

  .stack:hover .tooltip {
    opacity: 1;
  }

  .tooltip {
    position: absolute;
    bottom: calc(100% + 10px);
    left: 50%;
    transform: translateX(-50%);
    max-width: min(280px, 85vw);
    padding: 9px 14px;
    border-radius: 10px;
    font-size: 12px;
    font-weight: 500;
    line-height: 1.4;
    text-align: center;
    color: rgba(248, 250, 252, 0.95);
    background: rgba(12, 14, 18, 0.92);
    border: 1px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.45);
    backdrop-filter: blur(12px);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.16s ease;
    z-index: 2;
  }

  .tip-line {
    display: block;
  }

  .tip-sub {
    display: block;
    margin-top: 4px;
    font-size: 11px;
    font-weight: 400;
    color: rgba(248, 250, 252, 0.65);
  }

  .accent {
    color: #e8b4d4;
    font-weight: 600;
  }

  .pill {
    margin: 0;
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
    border-radius: 999px;
    border: 1px solid rgba(255, 255, 255, 0.38);
    background: rgba(6, 8, 10, 0.45);
    backdrop-filter: blur(10px);
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      min-width 0.18s ease,
      min-height 0.18s ease,
      padding 0.18s ease,
      border-color 0.15s ease;
    min-width: 72px;
    min-height: 22px;
    padding: 5px 14px;
  }

  .pill:hover {
    border-color: rgba(255, 255, 255, 0.52);
    background: rgba(10, 12, 16, 0.55);
  }

  .pill.expanded {
    min-width: 236px;
    min-height: 36px;
    padding: 10px 20px;
  }

  .idle-cap {
    display: block;
    width: 44px;
    height: 3px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.2);
  }

  .dots {
    display: flex;
    align-items: flex-end;
    justify-content: center;
    gap: 5px;
    height: 22px;
  }

  .dot {
    width: 4px;
    height: 18px;
    border-radius: 2px;
    background: rgba(255, 255, 255, 0.88);
    transform: scaleY(var(--lvl));
    transform-origin: center bottom;
    transition: transform 0.06s ease-out, opacity 0.2s ease;
    opacity: 0.92;
  }

  .dot.busy {
    animation: breathe 0.9s ease-in-out infinite;
    animation-delay: calc(var(--i, 0) * 0.06s);
    opacity: 0.55;
  }

  .dot.busy:nth-child(1) {
    --i: 0;
  }
  .dot.busy:nth-child(2) {
    --i: 1;
  }
  .dot.busy:nth-child(3) {
    --i: 2;
  }
  .dot.busy:nth-child(4) {
    --i: 3;
  }
  .dot.busy:nth-child(5) {
    --i: 4;
  }
  .dot.busy:nth-child(6) {
    --i: 5;
  }
  .dot.busy:nth-child(7) {
    --i: 6;
  }
  .dot.busy:nth-child(8) {
    --i: 7;
  }
  .dot.busy:nth-child(9) {
    --i: 8;
  }

  @keyframes breathe {
    0%,
    100% {
      transform: scaleY(0.25);
      opacity: 0.45;
    }
    50% {
      transform: scaleY(0.85);
      opacity: 0.85;
    }
  }
</style>
