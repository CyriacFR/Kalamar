// Start-up timeline for the measuring script (read through the WebView's performance timeline). Marks cost
// microseconds and nothing leaves the page.
export function mark(name: string): void {
  try {
    if (!performance.getEntriesByName(name, "mark").length) performance.mark(name);
  } catch {
    /* no performance timeline: nothing to record */
  }
}

/** Times one IPC call as a `cmd:<name>` measure (start and end on the page's clock). */
export async function timedCall<T>(cmd: string, f: () => Promise<T>): Promise<T> {
  const start = performance.now();
  try {
    return await f();
  } finally {
    try {
      performance.measure(`cmd:${cmd}`, { start, end: performance.now() });
    } catch {
      /* ignore */
    }
  }
}

// Evaluated first (main.tsx imports this module first): the start of the bundle's execution.
mark("js:eval-start");
