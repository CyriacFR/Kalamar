// Browser stand-in for "@tauri-apps/api/event" (vite --mode mock only): an in-page event bus the mock back end emits on.
type Handler = (e: { event: string; id: number; payload: unknown }) => void;
export type UnlistenFn = () => void;

const handlers = new Map<string, Set<Handler>>();

export async function listen<T>(event: string, cb: (e: { event: string; id: number; payload: T }) => void): Promise<UnlistenFn> {
  const set = handlers.get(event) ?? new Set<Handler>();
  handlers.set(event, set);
  set.add(cb as Handler);
  return () => set.delete(cb as Handler);
}

export function emit(event: string, payload: unknown): void {
  for (const h of handlers.get(event) ?? []) h({ event, id: 0, payload });
}
