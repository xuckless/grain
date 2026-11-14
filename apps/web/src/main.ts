import { mount } from "svelte";
import { App } from "@workstation/ui";
import { createWebHost } from "./host";

const target = document.getElementById("app")!;
createWebHost().then(
  (host) => mount(App, { target, props: { host } }),
  (e) => {
    target.textContent = `Engine failed to start: ${e instanceof Error ? e.message : String(e)}`;
  },
);
