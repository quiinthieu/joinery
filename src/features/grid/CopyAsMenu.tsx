import { useEffect, useRef, useState, type RefObject } from "react";
import { ChevronDown, Copy } from "lucide-react";
import { COPY_FORMATS, type CopyFormat } from "./copy-format";
import type { CopyScope, CopySelectionSummary, DatabaseGridHandle } from "./DatabaseGrid";

interface Props {
  gridRef: RefObject<DatabaseGridHandle>;
  disabled?: boolean;
}

export function CopyAsMenu({ gridRef, disabled }: Props) {
  const [open, setOpen] = useState(false);
  const [summary, setSummary] = useState<CopySelectionSummary | null>(null);
  const [scope, setScope] = useState<Exclude<CopyScope, "auto">>("loaded");
  const [includeHeaders, setIncludeHeaders] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  function toggleMenu() {
    if (open) {
      setOpen(false);
      return;
    }
    const nextSummary = gridRef.current?.selectionSummary() ?? null;
    setSummary(nextSummary);
    setScope(nextSummary?.hasRangeSelection ? "selection" : "loaded");
    setOpen(true);
  }

  async function chooseFormat(format: CopyFormat) {
    setOpen(false);
    await gridRef.current?.copyAs(format, { scope, includeHeaders });
  }

  return (
    <div className="copy-as-menu" ref={containerRef}>
      <button
        type="button"
        className="button ghost small"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={disabled}
        onClick={toggleMenu}
      >
        <Copy size={12} />
        Copy as
        <ChevronDown size={11} />
      </button>
      {open && (
        <div className="copy-as-popover" role="menu" aria-label="Copy grid data as">
          {summary && (summary.hasRangeSelection || summary.hasSingleCell) && (
            <div className="copy-as-scopes" role="group" aria-label="Copy scope">
              {summary.hasRangeSelection && (
                <button
                  type="button"
                  className={scope === "selection" ? "active" : ""}
                  onClick={() => setScope("selection")}
                >
                  Selection ({summary.selectedCellCount.toLocaleString()})
                </button>
              )}
              <button
                type="button"
                className={scope === "loaded" ? "active" : ""}
                onClick={() => setScope("loaded")}
              >
                Loaded ({summary.loadedRowCount.toLocaleString()})
              </button>
              {summary.hasSingleCell && (
                <>
                  <button
                    type="button"
                    className={scope === "row" ? "active" : ""}
                    onClick={() => setScope("row")}
                  >
                    Row
                  </button>
                  <button
                    type="button"
                    className={scope === "cell" ? "active" : ""}
                    onClick={() => setScope("cell")}
                  >
                    Cell
                  </button>
                </>
              )}
            </div>
          )}
          <label className="copy-as-headers">
            <input
              type="checkbox"
              checked={includeHeaders}
              onChange={(event) => setIncludeHeaders(event.target.checked)}
            />
            <span>Include CSV/TSV headers</span>
          </label>
          <div className="copy-as-items">
            {COPY_FORMATS.map((item) => (
              <button
                key={item.id}
                type="button"
                role="menuitem"
                onClick={() => void chooseFormat(item.id)}
              >
                <span>Copy as {item.label}</span>
                <kbd>{item.hint}</kbd>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
