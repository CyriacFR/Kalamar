// Browser stand-in for "@tauri-apps/api/core" (vite --mode mock only): commands answer from the made-up back end.
import { handle } from "./backend";
import { iconDataUrl } from "./catalogue";

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (await handle(cmd, args ?? {})) as T;
}

export function convertFileSrc(path: string, _protocol = "asset"): string {
  return iconDataUrl(path);
}

// `?gallery=stages`: the stage cards in every state, over the app (see ./gallery.tsx).
if (typeof location !== "undefined" && new URLSearchParams(location.search).get("gallery") === "stages") {
  void import("./gallery").then((g) => g.mount());
}
