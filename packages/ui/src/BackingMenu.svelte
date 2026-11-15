<script lang="ts">
  import Menu from "./Menu.svelte";
  import { effectiveBacking } from "./recorder";
  import type { Session } from "./state";
  import { backingOptions } from "./takes";

  /** What the singer hears while recording (kept on the session), for Karaoke's toolbar. The
   * quiet guide vocal exists only here: it is never in takes, playback or exports. Latency and
   * calibration live in ⚙ Settings (AGENTS.md §8.6). */
  let { session, disabled = false }: { session: Session; disabled?: boolean } = $props();

  const backings = $derived(backingOptions(session.project).filter((b) => b.available));
  const backing = $derived(effectiveBacking(session));
</script>

<Menu label="Backing" value={backing} options={backings} onPick={(b) => !disabled && (session.backing = b)} />
