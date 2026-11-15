<script lang="ts">
  import { popover } from "./motion";

  /** A small confirm popover under its trigger (the trigger's wrapper is `position: relative`);
   * it grows from and shrinks back to the trigger (rules 11–12). */
  let {
    title,
    body,
    confirm,
    align = "right",
    onCancel,
    onConfirm,
  }: { title: string; body: string; confirm: string; align?: "left" | "right"; onCancel: () => void; onConfirm: () => void } = $props();
</script>

<div class="confirm" class:left={align === "left"} role="dialog" aria-label={title} transition:popover>
  <strong>{title}</strong>
  <span class="muted">{body}</span>
  <div class="row">
    <button class="press quiet" onclick={onCancel}>Cancel</button>
    <button class="press primary" onclick={onConfirm}>{confirm}</button>
  </div>
</div>

<style>
  .confirm {
    position: absolute;
    right: 0;
    top: 52px;
    width: 300px;
    box-sizing: border-box;
    padding: 16px 18px;
    background: var(--c-raised);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 14px;
    line-height: 21px;
    transform-origin: top right;
    z-index: 40;
  }
  .left {
    right: auto;
    left: 0;
    transform-origin: top left;
  }
  strong {
    font-weight: 500;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  .row button {
    height: 40px;
    padding: 0 14px;
    border-radius: 10px;
  }
</style>
