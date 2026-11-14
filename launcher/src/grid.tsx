// Virtualised, keyboard-navigable grid of fixed-size cells: only the rows near the viewport are in the DOM, so
// thousands of entries stay fast. Arrow keys / Home / End / PageUp / PageDown move the active cell, Enter or Space
// picks it; the grid is a single tab stop (aria-activedescendant).
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { iconUrl } from "./gamedata";

interface GridProps<T> {
  items: T[];
  keyOf: (item: T) => string | number;
  render: (item: T, state: { selected: boolean; active: boolean }) => ReactNode;
  isSelected: (item: T) => boolean;
  onPick: (item: T) => void;
  label: string;
  cellWidth: number;
  cellHeight: number;
  gap?: number;
  empty?: ReactNode;
  className?: string;
  /** One column whose cells span the full width (lists). */
  fill?: boolean;
}

export function VirtualGrid<T>({ items, keyOf, render, isSelected, onPick, label, cellWidth: cellWidthProp, cellHeight, gap = 8, empty, className, fill }: GridProps<T>) {
  const box = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: 600, h: 400 });
  const [scrollTop, setScrollTop] = useState(0);
  const [active, setActive] = useState(0);
  const uid = useRef(`g${Math.random().toString(36).slice(2, 8)}`).current;

  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setSize({ w: el.clientWidth, h: el.clientHeight }));
    ro.observe(el);
    setSize({ w: el.clientWidth, h: el.clientHeight });
    return () => ro.disconnect();
  }, []);

  const cellWidth = fill ? Math.max(120, size.w - 2) : cellWidthProp;
  const cols = fill ? 1 : Math.max(1, Math.floor((size.w + gap) / (cellWidth + gap)));
  const rowH = cellHeight + gap;
  const rows = Math.ceil(items.length / cols);
  const total = rows * rowH;

  // Keep the active index valid when the list changes (search, filters).
  useEffect(() => {
    setActive((a) => Math.min(a, Math.max(0, items.length - 1)));
  }, [items.length]);
  useEffect(() => {
    if (box.current) box.current.scrollTop = 0;
    setScrollTop(0);
    setActive(0);
  }, [items]);

  const ensureVisible = useCallback((index: number) => {
    const el = box.current;
    if (!el) return;
    const top = Math.floor(index / cols) * rowH;
    if (top < el.scrollTop) el.scrollTop = top;
    else if (top + cellHeight > el.scrollTop + el.clientHeight) el.scrollTop = top + cellHeight - el.clientHeight;
  }, [cols, rowH, cellHeight]);

  const move = (to: number) => {
    const next = Math.max(0, Math.min(items.length - 1, to));
    setActive(next);
    ensureVisible(next);
  };

  const onKey = (e: KeyboardEvent) => {
    const page = Math.max(1, Math.floor(size.h / rowH)) * cols;
    switch (e.key) {
      case "ArrowRight": move(active + 1); break;
      case "ArrowLeft": move(active - 1); break;
      case "ArrowDown": move(active + cols); break;
      case "ArrowUp": move(active - cols); break;
      case "PageDown": move(active + page); break;
      case "PageUp": move(active - page); break;
      case "Home": move(0); break;
      case "End": move(items.length - 1); break;
      case "Enter":
      case " ":
        if (items[active]) onPick(items[active]);
        break;
      default: return;
    }
    e.preventDefault();
  };

  const first = Math.max(0, Math.floor(scrollTop / rowH) - 2);
  const last = Math.min(rows, Math.ceil((scrollTop + size.h) / rowH) + 2);
  const cells = useMemo(() => {
    const out: ReactNode[] = [];
    for (let r = first; r < last; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c;
        if (i >= items.length) break;
        const it = items[i];
        const selected = isSelected(it);
        out.push(
          <div
            key={keyOf(it)}
            id={`${uid}-${i}`}
            role="option"
            aria-selected={selected}
            className={`vg-cell ${selected ? "vg-selected" : ""} ${i === active ? "vg-active" : ""}`}
            style={{ transform: `translate(${c * (cellWidth + gap)}px, ${r * rowH}px)`, width: cellWidth, height: cellHeight }}
            onClick={() => { setActive(i); onPick(it); box.current?.focus(); }}
          >
            {render(it, { selected, active: i === active })}
          </div>,
        );
      }
    }
    return out;
  }, [first, last, cols, items, active, isSelected, render, keyOf, onPick, uid, cellWidth, cellHeight, gap, rowH]);

  return (
    <div
      ref={box}
      className={`vg ${className ?? ""}`}
      role="listbox"
      aria-label={label}
      aria-activedescendant={items.length ? `${uid}-${active}` : undefined}
      tabIndex={0}
      onKeyDown={onKey}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
    >
      {items.length === 0 ? <div className="vg-empty">{empty}</div> : <div className="vg-spacer" style={{ height: total }}>{cells}</div>}
    </div>
  );
}

/** An icon from the game data cache; hidden when the file cannot be decoded. */
export function GameIcon({ icon, alt = "", className }: { icon: string | null | undefined; alt?: string; className?: string }) {
  const [bad, setBad] = useState(false);
  useEffect(() => setBad(false), [icon]);
  if (!icon || bad) return <span className={`gicon gicon-none ${className ?? ""}`} aria-hidden="true" />;
  return <img className={`gicon ${className ?? ""}`} src={iconUrl(icon)} alt={alt} loading="lazy" decoding="async" draggable={false} onError={() => setBad(true)} />;
}
