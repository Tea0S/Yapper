<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
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
  let dictIoMsg = $state("");
  let dictIoErr = $state("");
  /** Merge updates matching term+scope; replace clears the dictionary first. */
  let importMode = $state<"merge" | "replace">("merge");

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

  async function exportDictionary() {
    dictIoErr = "";
    dictIoMsg = "";
    const path = await save({
      title: "Export dictionary",
      defaultPath: "yapper-dictionary.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (path == null) return;
    try {
      await invoke("export_dictionary_to_path", { path });
      dictIoMsg = `Exported to ${path}`;
    } catch (e) {
      dictIoErr = String(e);
    }
  }

  async function importDictionary() {
    dictIoErr = "";
    dictIoMsg = "";
    const path = await open({
      title: "Import dictionary",
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (path == null || typeof path !== "string") return;
    try {
      const summary = await invoke<{ inserted: number; updated: number }>(
        "import_dictionary_from_path",
        { path, replace: importMode === "replace" },
      );
      if (importMode === "replace") {
        dictIoMsg = `Imported ${summary.inserted} entries (replaced all).`;
      } else {
        dictIoMsg = `Imported: ${summary.inserted} new, ${summary.updated} updated.`;
      }
      await load();
    } catch (e) {
      dictIoErr = String(e);
    }
  }
</script>

<section class="grid">
  <div>
    <h1>Dictionary</h1>
    <p class="muted">Word or phrase boosts (word-boundary aware for <code>word</code> scope).</p>
    <div class="dict-io">
      <button type="button" class="btn" onclick={exportDictionary}>Export…</button>
      <button type="button" class="btn" onclick={importDictionary}>Import…</button>
      <label class="import-mode">
        <span class="sr-only">Import mode</span>
        <select bind:value={importMode} title="Import mode">
          <option value="merge">Merge with existing</option>
          <option value="replace">Replace all</option>
        </select>
      </label>
    </div>
    {#if dictIoMsg}
      <p class="io-ok">{dictIoMsg}</p>
    {/if}
    {#if dictIoErr}
      <p class="io-err">{dictIoErr}</p>
    {/if}
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
  .dict-io {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 0.75rem;
    margin-bottom: 0.75rem;
  }
  .import-mode select {
    font-size: 0.88rem;
    padding: 0.35rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--panel);
    color: var(--text);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .io-ok {
    font-size: 0.88rem;
    color: var(--text-muted);
    margin: 0 0 0.75rem;
  }
  .io-err {
    font-size: 0.88rem;
    color: #c44;
    margin: 0 0 0.75rem;
  }
</style>
