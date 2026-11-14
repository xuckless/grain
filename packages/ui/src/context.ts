// App-wide services handed to components through Svelte context.

import { getContext, setContext } from "svelte";
import type { Burst } from "./grain";

export type ToastTone = "info" | "error";

export type Services = {
  /** The full-window hit-burst overlay. */
  burst: Burst;
  /** Shows a short message that dissolves after a few seconds. */
  toast(text: string, tone?: ToastTone): void;
};

const KEY = Symbol("services");

export const provideServices = (s: Services): void => void setContext(KEY, s);
export const services = (): Services => getContext<Services>(KEY);
