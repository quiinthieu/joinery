import { useEffect, useRef, type ReactNode } from "react";
import { X } from "lucide-react";

export function Modal({ title, subtitle, children, onClose, wide = false, busy = false }: { title: string; subtitle?: string; children: ReactNode; onClose(): void; wide?: boolean; busy?: boolean }) {
  const box = useRef<HTMLDivElement>(null), latest = useRef({ onClose, busy }); latest.current = { onClose, busy };
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const timer = setTimeout(() => (box.current?.querySelector("[autofocus], input, textarea, select, button") as HTMLElement | null)?.focus(), 0);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); if (!latest.current.busy) latest.current.onClose(); }
      if (event.key !== "Tab") return;
      const fields = [...(box.current?.querySelectorAll<HTMLElement>("button:not([disabled]),input:not([disabled]),textarea:not([disabled]),select:not([disabled]),[tabindex='0']") ?? [])].filter((element) => element.offsetParent !== null);
      if (!fields.length) return;
      if (event.shiftKey && document.activeElement === fields[0]) { event.preventDefault(); fields.at(-1)?.focus(); }
      if (!event.shiftKey && document.activeElement === fields.at(-1)) { event.preventDefault(); fields[0].focus(); }
    };
    document.addEventListener("keydown", onKey, true);
    return () => { clearTimeout(timer); document.removeEventListener("keydown", onKey, true); if (previous?.isConnected) previous.focus(); };
  }, []);
  return <div className="modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) onClose(); }}>
    <div className={`modal ${wide ? "modal-wide" : ""}`} ref={box} role="dialog" aria-modal="true" aria-label={title}>
      <header className="modal-header"><div><h2>{title}</h2>{subtitle && <p>{subtitle}</p>}</div><button className="icon-button" aria-label="Close dialog" onClick={onClose} disabled={busy}><X size={17} /></button></header>
      {children}
    </div>
  </div>;
}

export function InlineError({ message }: { message: string | null }) { return message ? <div className="inline-error" role="alert">{message}</div> : null; }
