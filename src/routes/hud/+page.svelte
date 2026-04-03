<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  type HudPhase = "hidden" | "listening" | "transcribing";
  type MicLevel = { rms: number; peak: number };

  let phase = $state<HudPhase>("listening");
  let mic = $state<MicLevel>({ rms: 0, peak: 0 });
  let pttHint = $state("Push-to-talk");

  const dotCount = 9;

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
  <div class="hint-pill">
    <span class="hint-muted">Hold</span>
    <span class="hint-key">{pttHint}</span>
    <span class="hint-muted">to dictate · release to transcribe</span>
  </div>
  <div class="meter-pill" aria-hidden="true">
    <div class="dots">
      {#each Array.from({ length: dotCount }, (_, i) => i) as i (i)}
        <span
          class="dot"
          class:busy={phase === "transcribing"}
          style="--lvl: {phase === 'listening' ? dotLevel(i) : 0.22}"
        ></span>
      {/each}
    </div>
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
    padding: 10px 14px 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    font-family: "DM Sans", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
    user-select: none;
  }

  .hint-pill,
  .meter-pill {
    border-radius: 999px;
    border: 1px solid rgba(255, 255, 255, 0.38);
    background: rgba(6, 8, 10, 0.45);
    backdrop-filter: blur(10px);
  }

  .hint-pill {
    padding: 8px 18px;
    font-size: 12.5px;
    font-weight: 500;
    letter-spacing: 0.01em;
    color: rgba(248, 250, 252, 0.92);
    text-align: center;
    max-width: 340px;
    line-height: 1.35;
  }

  .hint-muted {
    color: rgba(248, 250, 252, 0.78);
  }

  .hint-key {
    color: #e8b4d4;
    font-weight: 600;
    margin: 0 0.2em;
  }

  .meter-pill {
    padding: 10px 22px 11px;
    min-width: 200px;
    display: flex;
    align-items: center;
    justify-content: center;
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
