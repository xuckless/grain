<script lang="ts">
  import type { EngineHost } from "@workstation/engine-api";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import { masterId, selectedTakeId } from "./chain";
  import { services } from "./context";
  import { sungTakes } from "./listen";
  import { errorMessage, type Session } from "./state";

  /**
   * The toolbar's take picker (AGENTS.md §8.6), shared by Score, Karaoke, Auto-tune, Pitch and
   * Produce: which take the view works on (the ★ master unless another is picked), and the star
   * that makes the picked take the master. Starring is undoable.
   */
  let { host, session }: { host: EngineHost; session: Session } = $props();

  const { toast } = services();
  const project = $derived(session.project);
  const master = $derived(masterId(project));
  const take = $derived(selectedTakeId(project, session.take));
  const options = $derived(
    sungTakes(project)
      .slice()
      .reverse()
      .map((t) => ({ id: String(t.id), label: t.id === master ? `★ ${t.name}` : t.name })),
  );

  async function star() {
    if (take === null) return;
    const before = project.master_take;
    const after = take === master && before === take ? null : take;
    try {
      session.saved(await host.takes.setMaster(after));
      session.record({ kind: "Master", before, after });
    } catch (e) {
      toast(`Couldn't star the take: ${errorMessage(e)}`, "error");
    }
  }
</script>

{#if take !== null}
  <div class="picker">
    <Menu label="Take" value={String(take)} {options} onPick={(id) => (session.take = Number(id))} />
    <button
      class="press icon quiet star"
      class:on={take === master}
      aria-pressed={take === master}
      aria-label={take === master ? "This is the master take" : "Make this the master take"}
      title={take === master ? "The master take: exports and Produce use it" : "Make this the master take"}
      onclick={star}><Icon name={take === master ? "starFilled" : "star"} /></button
    >
  </div>
{/if}

<style>
  .picker {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .star.on {
    color: var(--c-near);
  }
</style>
