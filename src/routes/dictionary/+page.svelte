<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  type DictRow = {
    id?: number;
    term: string;
    replacement: string;
    priority: number;
    scope: string;
  };
  type CorRow = {
    id?: number;
    mishear: string;
    intended: string;
    priority: number;
  };

  let dict = $state<DictRow[]>([]);
  let corr = $state<CorRow[]>([]);
  let dTerm = $state("");
  let dRep = $state("");
  let dScope = $state("word");
  let cFrom = $state("");
  let cTo = $state("");

  async function load() {
    dict = await invoke("list_dictionary_cmd");
    corr = await invoke("list_corrections_cmd");
  }

  onMount(load);

  async function addDict() {
    if (!dTerm.trim()) return;
    await invoke("upsert_dictionary_cmd", {
      entry: {
        id: null,
        term: dTerm.trim(),
        replacement: dRep.trim() || dTerm.trim(),
        priority: 10,
        scope: dScope,
      },
    });
    dTerm = "";
    dRep = "";
    await load();
  }

  async function addCorr() {
    if (!cFrom.trim()) return;
    await invoke("upsert_correction_cmd", {
      entry: {
        id: null,
        mishear: cFrom.trim(),
        intended: cTo.trim() || cFrom.trim(),
        priority: 20,
      },
    });
    cFrom = "";
    cTo = "";
    await load();
  }

  async function delDict(id: number) {
    await invoke("delete_dictionary_cmd", { id });
    await load();
  }

  async function delCorr(id: number) {
    await invoke("delete_correction_cmd", { id });
    await load();
  }
</script>

<section class="grid">
  <div>
    <h1>Dictionary</h1>
    <p class="muted">Word or phrase boosts (word-boundary aware for <code>word</code> scope).</p>
    <div class="panel">
      <div class="field">
        <label for="t">Term</label>
        <input id="t" bind:value={dTerm} placeholder="Yapper" />
      </div>
      <div class="field">
        <label for="r">Replacement</label>
        <input id="r" bind:value={dRep} placeholder="Yapper" />
      </div>
      <div class="field">
        <label for="s">Scope</label>
        <select id="s" bind:value={dScope}>
          <option value="word">word</option>
          <option value="phrase">phrase</option>
        </select>
      </div>
      <button type="button" class="btn btn-primary" onclick={addDict}>Add / update</button>
      <ul class="list">
        {#each dict as row}
          <li>
            <span
              ><strong>{row.term}</strong> → {row.replacement}
              <small>({row.scope})</small></span>
            {#if row.id != null}
              <button type="button" class="btn mini" onclick={() => delDict(row.id!)}>Remove</button>
            {/if}
          </li>
        {/each}
      </ul>
    </div>
  </div>

  <div>
    <h1>Corrections</h1>
    <p class="muted">Simple mishear → intended replacements (applied before dictionary).</p>
    <div class="panel">
      <div class="field">
        <label for="mf">Mishear</label>
        <input id="mf" bind:value={cFrom} placeholder="wrapper flow" />
      </div>
      <div class="field">
        <label for="int">Intended</label>
        <input id="int" bind:value={cTo} placeholder="WisprFlow" />
      </div>
      <button type="button" class="btn btn-primary" onclick={addCorr}>Add / update</button>
      <ul class="list">
        {#each corr as row}
          <li>
            <span><strong>{row.mishear}</strong> → {row.intended}</span>
            {#if row.id != null}
              <button type="button" class="btn mini" onclick={() => delCorr(row.id!)}>Remove</button>
            {/if}
          </li>
        {/each}
      </ul>
    </div>
  </div>
</section>

<style>
  h1 {
    margin-top: 0;
    font-size: 1.35rem;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2rem;
  }
  @media (max-width: 880px) {
    .grid {
      grid-template-columns: 1fr;
    }
  }
  .muted {
    color: var(--text-muted);
    font-size: 0.92rem;
  }
  .list {
    list-style: none;
    margin: 1rem 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .list li {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.5rem;
    padding: 0.45rem 0;
    border-bottom: 1px solid var(--border);
    font-size: 0.92rem;
  }
  .mini {
    padding: 0.25rem 0.5rem;
    font-size: 0.8rem;
  }
  small {
    color: var(--text-muted);
    font-weight: 400;
  }
</style>
