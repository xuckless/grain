<script lang="ts">
  import type { EngineHost } from "@workstation/engine-api";
  import Confirm from "./Confirm.svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import { masterId, selectedTakeId } from "./chain";
  import { listener, services } from "./context";
  import { sungTakes } from "./listen";
  import { errorMessage, type Session } from "./state";
  import { deleteTake } from "./takeops";

  /**
   * The toolbar's take picker (AGENTS.md §8.6), shared by Score, Karaoke, Auto-tune, Pitch and
   * Produce: which take the view works on (the ★ master unless another is picked), and the star
   * that makes the picked take the master. Each take in the list can be deleted (asked first).
   * Starring and deleting are undoable.
   */
  let { host, session }: { host: EngineHost; session: Session } = $props();

  const { toast } = services();
  const listen = listener();
  const project = $derived(session.project);
  const master = $derived(masterId(project));
  const take = $derived(selectedTakeId(project, session.take));
  const options = $derived(
    sungTakes(project)
      .slice()
      .reverse()
      .map((t) => ({ id: String(t.id), label: t.id === master ? `★ ${t.name}` : t.name })),
  );

  type Deleting = { kind: "idle" } | { kind: "confirming"; id: number; name: string } | { kind: "deleting"; id: number };
  let deleting = $state<Deleting>({ kind: "idle" });

  async function remove(id: number) {
    deleting = { kind: "deleting", id };
    if (session.listening.kind !== "idle") listen({ kind: "stop" });
    try {
      await deleteTake(host.takes, session, id);
    } catch (e) {
      toast(`Couldn't delete the take: ${errorMessage(e)}`, "error");
    }
    deleting = { kind: "idle" };
  }

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
    <div class="anchor">
      <Menu
        label="Take"
        value={String(take)}
        {options}
        onPick={(id) => (session.take = Number(id))}
        remove={{
          label: (o) => `Delete ${o.label.replace(/^★ /, "")}`,
          run: (id) => {
            const t = project.takes.find((x) => x.id === Number(id));
            if (t && deleting.kind === "idle") deleting = { kind: "confirming", id: t.id, name: t.name };
          },
        }}
      />
      {#if deleting.kind === "confirming"}
        {@const d = deleting}
        <Confirm
          title={`Delete “${d.name}”?`}
          body={d.id === master ? "It's the ★ master: the newest remaining take becomes the master. ⌘Z brings it back." : "It leaves the song. ⌘Z brings it back until you close the song."}
          confirm="Delete"
          align="left"
          onCancel={() => (deleting = { kind: "idle" })}
          onConfirm={() => void remove(d.id)}
        />
      {/if}
    </div>
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
  .anchor {
    position: relative;
  }
  .star.on {
    color: var(--c-near);
  }
</style>
