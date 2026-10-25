import { useEffect, useRef, useState, type ReactNode } from "react";
import { X } from "lucide-react";

export function Modal({
  title,
  subtitle,
  children,
  onClose,
  wide = false,
  busy = false,
}: {
  title: string;
  subtitle?: string;
  children: ReactNode;
  onClose(): void;
  wide?: boolean;
  busy?: boolean;
}) {
  const box = useRef<HTMLDivElement>(null),
    latest = useRef({ onClose, busy });
  // Capture before child autoFocus runs during commit, rather than in an effect.
  const [previous] = useState(() => document.activeElement as HTMLElement | null);
  latest.current = { onClose, busy };
  useEffect(() => {
    const fields = () =>
      [
        ...(box.current?.querySelectorAll<HTMLElement>(
          "a[href],button,input,textarea,select,[tabindex]",
        ) ?? []),
      ].filter(
        (element) =>
          element.tabIndex >= 0 &&
          !element.matches(":disabled") &&
          !element.closest("[inert]") &&
          element.getClientRects().length > 0,
      );
    const timer = setTimeout(() => {
      if (box.current?.contains(document.activeElement)) return;
      const available = fields();
      (
        available.find((element) => element.matches("input,textarea,select")) ??
        available[0] ??
        box.current
      )?.focus();
    }, 0);
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        if (!latest.current.busy) latest.current.onClose();
      }
      if (event.key !== "Tab") return;
      const available = fields();
      if (!available.length) {
        event.preventDefault();
        box.current?.focus();
        return;
      }
      if (!available.includes(document.activeElement as HTMLElement)) {
        event.preventDefault();
        (event.shiftKey ? available.at(-1) : available[0])?.focus();
      } else if (event.shiftKey && document.activeElement === available[0]) {
        event.preventDefault();
        available.at(-1)?.focus();
      } else if (!event.shiftKey && document.activeElement === available.at(-1)) {
        event.preventDefault();
        available[0].focus();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("keydown", onKey, true);
      if (previous?.isConnected) previous.focus();
    };
  }, [previous]);
  return (
    <div
      className="modal-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onClose();
      }}
    >
      <div
        className={`modal ${wide ? "modal-wide" : ""}`}
        ref={box}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        tabIndex={-1}
      >
        <header className="modal-header">
          <div>
            <h2>{title}</h2>
            {subtitle && <p>{subtitle}</p>}
          </div>
          <button
            className="icon-button"
            aria-label="Close dialog"
            onClick={onClose}
            disabled={busy}
          >
            <X size={17} />
          </button>
        </header>
        {children}
      </div>
    </div>
  );
}

export function InlineError({ message }: { message: string | null }) {
  return message ? (
    <div className="inline-error" role="alert">
      {message}
    </div>
  ) : null;
}
