// App-wide services handed to components through Svelte context.

import { getContext, setContext, type Snippet } from "svelte";
import type { Burst } from "./grain";
import type { ListenEvent } from "./listen";

export type ToastTone = "info" | "error";

export type Services = {
  /** The full-window hit-burst overlay. */
  burst: Burst;
  /** Shows a short message that dissolves after a few seconds. */
  toast(text: string, tone?: ToastTone): void;
};

const KEY = Symbol("services");
const LISTEN = Symbol("listen");

export const provideServices = (s: Services): void => void setContext(KEY, s);
export const services = (): Services => getContext<Services>(KEY);

/** The loaded song's listen machine (listen.ts), driven by SessionScreen. */
export const provideListen = (listen: (e: ListenEvent) => void): void => void setContext(LISTEN, listen);
export const listener = (): ((e: ListenEvent) => void) => getContext(LISTEN);

/**
 * The session's bottom action bar (AGENTS.md §8.6). A view's `BottomAction` hands its buttons
 * here, and SessionScreen draws them in a full-width footer outside the scrolling content, so the
 * content always ends above the bar instead of passing behind it.
 */
export type Footer = { kind: "none" } | { kind: "shown"; owner: symbol; main: Snippet; side?: Snippet };
export type FooterSlot = { show(owner: symbol, main: Snippet, side?: Snippet): void; hide(owner: symbol): void };
const FOOTER = Symbol("footer");
export const provideFooter = (slot: FooterSlot): void => void setContext(FOOTER, slot);
export const footerSlot = (): FooterSlot => getContext<FooterSlot>(FOOTER);
