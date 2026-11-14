// Original icon set and logo mark, drawn for this launcher (inline SVG, 24x24 grid, 1.9 stroke, round caps).
import type { ReactNode, SVGProps } from "react";

type P = SVGProps<SVGSVGElement> & { size?: number | string };

function Svg({ size, children, ...rest }: P & { children: ReactNode }) {
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth={1.9} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

/** Logo mark: an ink drop cut by a slash, with a satellite drop. Colors come from CSS variables. */
export function Mark({ size = 40, className }: { size?: number | string; className?: string }) {
  return (
    <svg className={className} viewBox="0 0 64 64" width={size} height={size} aria-hidden="true" focusable="false">
      <defs>
        <mask id="mk-cut" maskUnits="userSpaceOnUse" x="0" y="0" width="64" height="64">
          <rect width="64" height="64" fill="#fff" />
          <path d="M6 47 L58 21 L58 29 L6 55 Z" fill="#000" />
        </mask>
      </defs>
      <path
        mask="url(#mk-cut)"
        fill="var(--mint)"
        d="M30 3c7 11 21 21 21 36 0 11.600-9.400 20-21 20S9 50.600 9 39C9 24 23 14 30 3Z"
      />
      <path fill="var(--magenta)" d="M47 33c3.200 4.400 7.500 7.500 7.500 12.200 0 4.200-3.400 7.300-7.500 7.300s-7.500-3.100-7.500-7.300C39.500 40.500 43.800 37.400 47 33Z" />
      <circle cx="55.500" cy="21" r="2.600" fill="var(--mint)" />
      <circle cx="12" cy="13" r="1.800" fill="var(--magenta)" />
    </svg>
  );
}

export const IconPlay = (p: P) => <Svg {...p}><path d="M7 4.500v15l12-7.500L7 4.500Z" fill="currentColor" stroke="none" /></Svg>;
export const IconLocker = (p: P) => <Svg {...p}><rect x="4" y="3.500" width="16" height="17" rx="2.500" /><path d="M12 3.500v17M8 8.500v2.500M16 8.500v2.500" /></Svg>;
export const IconFriends = (p: P) => <Svg {...p}><circle cx="9" cy="8.500" r="3.200" /><path d="M3 19.500c.4-3.300 2.800-5.200 6-5.200s5.600 1.900 6 5.200" /><path d="M16 5.800a3.200 3.200 0 0 1 0 5.800M18.200 14.700c1.600.7 2.600 2.300 2.800 4.800" /></Svg>;
export const IconRank = (p: P) => <Svg {...p}><path d="M12 3.500 19.500 8v8L12 20.500 4.500 16V8L12 3.500Z" /><path d="m8.500 12.500 3.500 2.500 3.500-2.500M8.500 9.500 12 12l3.500-2.500" /></Svg>;
export const IconSettings = (p: P) => <Svg {...p}><path d="M4 7h9M17 7h3M4 17h3M11 17h9" /><circle cx="15" cy="7" r="2.200" /><circle cx="9" cy="17" r="2.200" /></Svg>;
export const IconInfo = (p: P) => <Svg {...p}><circle cx="12" cy="12" r="9" /><path d="M12 11v5.500M12 7.800v.1" /></Svg>;
export const IconTarget = (p: P) => <Svg {...p}><circle cx="12" cy="12" r="8.500" /><circle cx="12" cy="12" r="3.800" /><path d="M12 1.500v4M12 18.500v4M1.500 12h4M18.500 12h4" /></Svg>;
export const IconCompass = (p: P) => <Svg {...p}><circle cx="12" cy="12" r="9" /><path d="m15.800 8.200-2 5.600-5.600 2 2-5.600 5.600-2Z" /></Svg>;
export const IconLock = (p: P) => <Svg {...p}><rect x="5" y="10.500" width="14" height="9.500" rx="2.200" /><path d="M8.200 10.500V8a3.800 3.800 0 0 1 7.600 0v2.500M12 14.200v2.300" /></Svg>;
export const IconBolt = (p: P) => <Svg {...p}><path d="M13 2.500 5 13.500h6l-1 8 8-11h-6l1-8Z" /></Svg>;
export const IconVersus = (p: P) => <Svg {...p}><path d="m4 4 7 7M20 4l-7 7M4 20l6-6M20 20l-6-6" /><path d="M10.500 13.500 13.500 10.500" /></Svg>;
export const IconSquad = (p: P) => <Svg {...p}><circle cx="7" cy="8" r="2.300" /><circle cx="17" cy="8" r="2.300" /><circle cx="7" cy="17" r="2.300" /><circle cx="17" cy="17" r="2.300" /></Svg>;
export const IconPlus = (p: P) => <Svg {...p}><path d="M12 5v14M5 12h14" /></Svg>;
export const IconJoin = (p: P) => <Svg {...p}><path d="M14 4.500h4.500a1.500 1.500 0 0 1 1.500 1.500v12a1.500 1.500 0 0 1-1.500 1.500H14M4 12h10M10.500 8l3.500 4-3.500 4" /></Svg>;
export const IconCopy = (p: P) => <Svg {...p}><rect x="8.500" y="8.500" width="11" height="11" rx="2" /><path d="M15.500 8.500V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7.500a2 2 0 0 0 2 2h2.500" /></Svg>;
export const IconCheck = (p: P) => <Svg {...p}><path d="m5 12.500 4.500 4.500L19 7.500" /></Svg>;
export const IconAlert = (p: P) => <Svg {...p}><path d="M12 3.500 21.500 20h-19L12 3.500Z" /><path d="M12 10v4.500M12 17.300v.1" /></Svg>;
export const IconClose = (p: P) => <Svg {...p}><path d="m6 6 12 12M18 6 6 18" /></Svg>;
export const IconBack = (p: P) => <Svg {...p}><path d="M15 5.500 8.500 12l6.500 6.500" /></Svg>;
export const IconChevron = (p: P) => <Svg {...p}><path d="m9 5.500 6.500 6.500L9 18.500" /></Svg>;
export const IconStop = (p: P) => <Svg {...p}><rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" stroke="none" /></Svg>;
export const IconRefresh = (p: P) => <Svg {...p}><path d="M19.500 12a7.500 7.500 0 1 1-2.200-5.300M19.500 4.500v4h-4" /></Svg>;
export const IconRelay = (p: P) => <Svg {...p}><circle cx="12" cy="12" r="2.200" /><path d="M7.200 7.200a6.800 6.800 0 0 0 0 9.600M16.800 7.200a6.800 6.800 0 0 1 0 9.600M4.200 4.200a11 11 0 0 0 0 15.600M19.800 4.200a11 11 0 0 1 0 15.600" /></Svg>;
export const IconCrown = (p: P) => <Svg {...p}><path d="m3.500 8 4.500 4 4-7 4 7 4.500-4-1.500 11h-14L3.500 8Z" /></Svg>;
export const IconFolder = (p: P) => <Svg {...p}><path d="M3.500 6.500a2 2 0 0 1 2-2h4l2 2.500h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2v-11.500Z" /></Svg>;
export const IconShuffle = (p: P) => <Svg {...p}><path d="M3.500 7h3.200c2.500 0 3.800 1.300 5.300 5s2.800 5 5.300 5h3.200M3.500 17h3.200c1.400 0 2.400-.4 3.200-1.300M13.800 8.300c.8-.9 1.800-1.300 3.200-1.300h3.500M18 4.500 20.500 7 18 9.500M18 14.500l2.500 2.500-2.500 2.500" /></Svg>;
export const IconBan = (p: P) => <Svg {...p}><circle cx="12" cy="12" r="8.500" /><path d="m6 6 12 12" /></Svg>;
export const IconMouse = (p: P) => <Svg {...p}><rect x="6.500" y="3" width="11" height="18" rx="5.500" /><path d="M12 3v6.500M6.500 9.500h11" /></Svg>;
export const IconScreen = (p: P) => <Svg {...p}><rect x="3" y="4.500" width="18" height="12" rx="2" /><path d="M9 20.500h6M12 16.500v4" /></Svg>;
export const IconInstall = (p: P) => <Svg {...p}><path d="M12 3.500v11M7.500 10.500l4.500 4.500 4.500-4.500M4.500 19.500h15" /></Svg>;
