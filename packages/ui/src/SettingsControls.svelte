<script lang="ts">
  import type { Difficulty } from "@workstation/engine-api";
  import Menu from "./Menu.svelte";
  import Toggle from "./Toggle.svelte";
  import type { Session } from "./state";

  /** Scoring difficulty and octave folding, kept on the session so Score and Karaoke agree.
   * `disabled`: a take is being recorded, so the rules can't change under it. */
  let { session, onChange, disabled = false }: { session: Session; onChange?: () => void; disabled?: boolean } = $props();

  const difficulties = [
    { id: "Easy", label: "Easy" },
    { id: "Medium", label: "Medium" },
    { id: "Hard", label: "Hard" },
  ] as const satisfies readonly { id: Difficulty; label: string }[];

  function set(next: () => void) {
    if (disabled) return;
    next();
    onChange?.();
  }
</script>

<Menu label="Difficulty" value={session.settings.difficulty} options={difficulties} onPick={(d) => set(() => (session.settings.difficulty = d))} />
<Toggle
  checked={session.settings.octave === "Fold"}
  label="Octave folding"
  onToggle={(on) => set(() => (session.settings.octave = on ? "Fold" : "Strict"))}
/>
