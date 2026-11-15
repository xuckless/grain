// App-wide services handed to components through Svelte context.

import { getContext, setContext } from "svelte";
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
