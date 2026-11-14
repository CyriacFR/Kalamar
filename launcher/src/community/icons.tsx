// The protocol's closed list of icon names (`activities.md` 5), drawn by the launcher itself: original line drawings,
// nothing from a game. An unknown name draws nothing.
import type { ReactNode, SVGProps } from "react";

type P = SVGProps<SVGSVGElement>;

const S = (p: P & { children: ReactNode }) => (
  <svg viewBox="0 0 24 24" width="1.25em" height="1.25em" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...p} />
);

const PATHS: Record<string, ReactNode> = {
  trophy: <><path d="M8 4h8v5a4 4 0 0 1-8 0V4Z" /><path d="M8 6H5a3 3 0 0 0 3 4M16 6h3a3 3 0 0 1-3 4M12 13v4M8.5 20h7M10 17h4" /></>,
  swords: <><path d="m4 4 9 9M4 4h3l9 9M4 4v3l9 9" /><path d="M20 4l-6.5 6.5M20 4h-3M20 4v3M14 17l3 3M17 14l3 3M15.5 18.5l3-3" /></>,
  shield: <path d="M12 3 19 6v5.5c0 4.2-3 7.6-7 9.5-4-1.9-7-5.3-7-9.5V6l7-3Z" />,
  star: <path d="m12 3.5 2.6 5.4 5.9.8-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.3-4.1 5.9-.8L12 3.5Z" />,
  crown: <path d="M4 8l4 3.5L12 5l4 6.5L20 8l-1.5 10h-13L4 8Z" />,
  flag: <><path d="M6 21V4" /><path d="M6 4h11l-2.5 4L17 12H6" /></>,
  clock: <><circle cx="12" cy="12" r="8.5" /><path d="M12 7.5V12l3 2" /></>,
  calendar: <><rect x="4" y="5.5" width="16" height="14" rx="2" /><path d="M4 10h16M8.5 3.5v4M15.5 3.5v4" /></>,
  users: <><circle cx="9" cy="8.5" r="3" /><path d="M3.5 19c.4-3 2.6-4.8 5.5-4.8s5.1 1.8 5.5 4.8" /><path d="M16 6a3 3 0 0 1 0 5.5M17.5 14.5c1.5.6 2.6 2 2.9 4.5" /></>,
  user: <><circle cx="12" cy="8.5" r="3.5" /><path d="M5 20c.5-3.8 3.2-6 7-6s6.5 2.2 7 6" /></>,
  target: <><circle cx="12" cy="12" r="8" /><circle cx="12" cy="12" r="4" /><circle cx="12" cy="12" r=".6" fill="currentColor" /></>,
  fire: <path d="M12 21c3.6 0 6-2.4 6-5.6 0-3.2-2.2-5.4-3.4-7.4-.4 1.6-1.2 2.6-2.2 3.2.2-3.4-1.2-6-3.4-7.7.2 3.2-1.4 5-2.8 6.8A7 7 0 0 0 6 15.4C6 18.6 8.4 21 12 21Z" />,
  bolt: <path d="M13 3 6 13.5h5.5L10.5 21 18 10.5h-5.5L13 3Z" />,
  medal: <><path d="M8 3h3l1.5 5M16 3h-3l-1.5 5" /><circle cx="12" cy="14.5" r="5.5" /><path d="m12 12 .9 1.8 2 .3-1.4 1.4.3 2-1.8-1-1.8 1 .3-2-1.4-1.4 2-.3L12 12Z" /></>,
  ribbon: <><circle cx="12" cy="9" r="5" /><path d="m9 13.5-2 7 5-2.5 5 2.5-2-7" /></>,
  map: <><path d="M3.5 6.5 9 4l6 2.5 5.5-2.5v13.5L15 20l-6-2.5-5.5 2.5V6.5Z" /><path d="M9 4v13.5M15 6.5V20" /></>,
  dice: <><rect x="4" y="4" width="16" height="16" rx="3" /><circle cx="8.5" cy="8.5" r="1" fill="currentColor" /><circle cx="15.5" cy="15.5" r="1" fill="currentColor" /><circle cx="12" cy="12" r="1" fill="currentColor" /></>,
  lock: <><rect x="5" y="10.5" width="14" height="9.5" rx="2" /><path d="M8.5 10.5V8a3.5 3.5 0 0 1 7 0v2.5" /></>,
  check: <path d="m5 12.5 4.5 4.5L19 7.5" />,
  info: <><circle cx="12" cy="12" r="8.5" /><path d="M12 11v5.5M12 7.8v.1" /></>,
  warning: <><path d="M12 4 21 19.5H3L12 4Z" /><path d="M12 10v4.5M12 17v.1" /></>,
  chat: <path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8a1.5 1.5 0 0 1-1.5 1.5h-7L7.5 20v-3.5H5A1.5 1.5 0 0 1 3.5 15V7A1.5 1.5 0 0 1 5 5.5Z" />,
  chart: <><path d="M4 20h16" /><path d="M7 16v-5M12 16V6M17 16v-8" /></>,
  heart: <path d="M12 19.5s-7.5-4.3-7.5-9.6A4.1 4.1 0 0 1 12 7.6a4.1 4.1 0 0 1 7.5 2.3c0 5.3-7.5 9.6-7.5 9.6Z" />,
};

export const ICON_NAMES = Object.keys(PATHS);

/** A protocol icon by name; nothing for a name outside the list. */
export function ProtoIcon({ name, ...p }: P & { name: unknown }) {
  if (typeof name !== "string" || !(name in PATHS)) return null;
  return <S {...p}>{PATHS[name]}</S>;
}

/** `#RRGGBB` from a server, or nothing (a hint: the page keeps its own contrast). */
export function safeColor(c: unknown): string | undefined {
  return typeof c === "string" && /^#[0-9a-fA-F]{6}$/.test(c) ? c : undefined;
}
