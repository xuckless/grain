import { mount } from "svelte";
import { App } from "@workstation/ui";
import { createDesktopHost } from "./host";

const target = document.getElementById("app")!;
createDesktopHost().then(
  (host) => mount(App, { target, props: { host } }),
  (e) => {
    target.textContent = `Engine failed to start: ${e instanceof Error ? e.message : String(e)}`;
  },
);
