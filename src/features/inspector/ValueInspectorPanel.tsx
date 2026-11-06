import { useState, useMemo, useEffect, useCallback } from "react";
import { X, Maximize2, ArrowUpRight, ArrowDownLeft, Copy, Check, Database } from "lucide-react";
import type { CellTarget } from "../grid/DatabaseGrid";
import { JsonTreeViewer } from "./JsonTreeViewer";
import { BinaryHexViewer } from "./BinaryHexViewer";
import { DateTimeInspector } from "./DateTimeInspector";
import { ArrayViewer } from "./ArrayViewer";
import { SpatialViewer } from "./SpatialViewer";
import { useStudio } from "../../state/studio";

export type InspectorMode = "raw" | "json" | "binary" | "datetime" | "array" | "spatial";

export interface ValueInspectorPanelProps {
  target: CellTarget | null;
  onClose(): void;
  onOpenValue?(target: CellTarget): void;
  onGoToReferenced?(): void;
  onGoToReferencing?(): void;
  hasReferencedKeys?: boolean;
  hasReferencingKeys?: boolean;
  sessionTimezone?: string;
}

export function detectCompatibleModes(target: CellTarget | null): InspectorMode[] {
  const modes: InspectorMode[] = ["raw"];
  if (!target || target.value === null) return modes;

  const raw = target.value.trim();
  const dbType = (target.column.databaseType || "").toLowerCase();
  const kind = target.column.kind;

  // JSON
  if (
    kind === "json" ||
    dbType.includes("variant") ||
    dbType.includes("json") ||
    (raw.startsWith("{") && raw.endsWith("}")) ||
    (raw.startsWith("[") && raw.endsWith("]"))
  ) {
    if (!modes.includes("json")) modes.push("json");
  }

  // Binary
  if (
    kind === "binary" ||
    dbType.includes("bytea") ||
    dbType.includes("blob") ||
    dbType.includes("binary") ||
    raw.startsWith("\\x") ||
    raw.startsWith("0x")
  ) {
    if (!modes.includes("binary")) modes.push("binary");
  }

  // DateTime
  if (
    kind === "temporal" ||
    dbType.includes("timestamp") ||
    dbType.includes("date") ||
    dbType.includes("time") ||
    /^\d{4}-\d{2}-\d{2}/.test(raw)
  ) {
    if (!modes.includes("datetime")) modes.push("datetime");
  }

  // Array
  if (
    dbType.endsWith("[]") ||
    dbType.includes("array") ||
    (raw.startsWith("{") && raw.endsWith("}")) ||
    (raw.startsWith("[") && raw.endsWith("]"))
  ) {
    if (!modes.includes("array")) modes.push("array");
  }

  // Spatial
  if (
    dbType.includes("geography") ||
    dbType.includes("geometry") ||
    /^(POINT|LINESTRING|POLYGON|MULTIPOINT|MULTILINESTRING|MULTIPOLYGON|GEOMETRYCOLLECTION)\s*\(/i.test(
      raw,
    )
  ) {
    if (!modes.includes("spatial")) modes.push("spatial");
  }

  return modes;
}

export function detectDefaultMode(target: CellTarget | null): InspectorMode {
  if (!target || target.value === null) return "raw";

  const raw = target.value.trim();
  const dbType = (target.column.databaseType || "").toLowerCase();
  const kind = target.column.kind;

  // 1. Spatial
  if (
    dbType.includes("geography") ||
    dbType.includes("geometry") ||
    /^(POINT|LINESTRING|POLYGON|MULTIPOINT|MULTILINESTRING|MULTIPOLYGON|GEOMETRYCOLLECTION)\s*\(/i.test(
      raw,
    )
  ) {
    return "spatial";
  }

  // 2. Binary
  if (
    kind === "binary" ||
    dbType.includes("bytea") ||
    dbType.includes("blob") ||
    dbType.includes("binary") ||
    raw.startsWith("\\x") ||
    raw.startsWith("0x")
  ) {
    return "binary";
  }

  // 3. Temporal
  if (
    kind === "temporal" ||
    dbType.includes("timestamp") ||
    dbType.includes("date") ||
    dbType.includes("time")
  ) {
    return "datetime";
  }

  // 4. Array (explicit array dbType or array literal)
  if (dbType.endsWith("[]") || dbType.includes("array")) {
    return "array";
  }

  // 5. JSON / Variant
  if (kind === "json" || dbType.includes("variant") || dbType.includes("json")) {
    // If it's a JSON array, prefer array mode if array-like, or json if complex
    if (raw.startsWith("[") && raw.endsWith("]")) {
      return "array";
    }
    return "json";
  }

  // 6. Content sniffing
  if (raw.startsWith("{") && raw.endsWith("}")) {
    return "json";
  }
  if (raw.startsWith("[") && raw.endsWith("]")) {
    return "array";
  }

  return "raw";
}

function calculateByteSize(value: string | null): number {
  if (value === null) return 0;
  if (value.startsWith("\\x") || value.startsWith("0x")) {
    const hex = value.slice(2);
    if (/^[0-9a-fA-F]*$/.test(hex)) return Math.floor(hex.length / 2);
  }
  return new TextEncoder().encode(value).length;
}

function formatByteSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export function ValueInspectorPanel({
  target,
  onClose,
  onOpenValue,
  onGoToReferenced,
  onGoToReferencing,
  hasReferencedKeys = false,
  hasReferencingKeys = false,
  sessionTimezone = "America/Los_Angeles",
}: ValueInspectorPanelProps) {
  const [activeMode, setActiveMode] = useState<InspectorMode>("raw");
  const [copied, setCopied] = useState(false);

  // Auto-switch mode when target changes
  useEffect(() => {
    setActiveMode(detectDefaultMode(target));
  }, [target?.resultId, target?.row, target?.column.id]);

  const compatibleModes = useMemo(() => detectCompatibleModes(target), [target]);
  const byteSize = useMemo(() => calculateByteSize(target?.value ?? null), [target?.value]);

  const handleCopyValue = useCallback(async () => {
    if (!target) return;
    const text = target.insert?.usingDefault
      ? "<default>"
      : target.value === null
        ? "NULL"
        : target.value;
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
    useStudio.getState().notify("Copied cell value");
  }, [target]);

  return (
    <aside
      className="inspector"
      aria-label="Selected cell inspector"
      aria-live="polite"
      aria-atomic="true"
    >
      {/* Title & Close Header */}
      <div className="inspector-title">
        <span>INSPECTOR</span>
        <button className="icon-button" aria-label="Close inspector" onClick={onClose}>
          <X size={13} />
        </button>
      </div>

      {target ? (
        <>
          {/* Metadata Section */}
          <div className="inspector-header">
            <h3>{target.column.name}</h3>
            <dl>
              <dt>Type</dt>
              <dd>
                <code>{target.column.databaseType}</code>
              </dd>
              <dt>Size</dt>
              <dd>
                {formatByteSize(byteSize)} ({byteSize.toLocaleString()} bytes)
              </dd>
              <dt>Nullable</dt>
              <dd>{target.column.nullable ? "Yes" : "No"}</dd>
              <dt>Key</dt>
              <dd>
                {target.column.primaryKey
                  ? "Primary key"
                  : target.column.foreignKey
                    ? "Foreign key"
                    : "—"}
              </dd>
              {target.deleted && (
                <>
                  <dt>Row</dt>
                  <dd>Marked for deletion</dd>
                </>
              )}
              {target.insert && (
                <>
                  <dt>Row</dt>
                  <dd>Staged insert</dd>
                </>
              )}
            </dl>
          </div>

          {/* Mode Switcher Tabs */}
          {compatibleModes.length > 1 && (
            <div className="inspector-mode-tabs" role="tablist" aria-label="Inspector mode">
              {compatibleModes.map((mode) => (
                <button
                  key={mode}
                  type="button"
                  role="tab"
                  aria-selected={activeMode === mode}
                  className={`inspector-mode-tab ${activeMode === mode ? "active" : ""}`}
                  onClick={() => setActiveMode(mode)}
                >
                  {mode.toUpperCase()}
                </button>
              ))}
            </div>
          )}

          {/* Viewer Content Area */}
          <div className="inspector-value">
            {target.insert?.usingDefault ? (
              <em>&lt;default&gt;</em>
            ) : target.insert?.unfilledRequired ? (
              <em>&lt;required&gt;</em>
            ) : target.value === null ? (
              <em>NULL</em>
            ) : activeMode === "json" ? (
              <JsonTreeViewer
                value={target.value}
                columnName={target.column.name}
                isVariant={
                  target.column.kind === "json" ||
                  (target.column.databaseType || "").toLowerCase().includes("variant")
                }
              />
            ) : activeMode === "binary" ? (
              <BinaryHexViewer value={target.value} column={target.column} />
            ) : activeMode === "datetime" ? (
              <DateTimeInspector
                value={target.value}
                column={target.column}
                sessionTimezone={sessionTimezone}
              />
            ) : activeMode === "array" ? (
              <ArrayViewer value={target.value} column={target.column} />
            ) : activeMode === "spatial" ? (
              <SpatialViewer value={target.value} column={target.column} />
            ) : (
              <pre>{target.value}</pre>
            )}
          </div>

          {/* Footer Actions */}
          <div className="inspector-actions">
            <button type="button" className="button ghost small" onClick={handleCopyValue}>
              {copied ? <Check size={12} /> : <Copy size={12} />}
              Copy value
            </button>
            {onOpenValue && (
              <button
                type="button"
                className="button ghost small"
                onClick={() => onOpenValue(target)}
              >
                <Maximize2 size={12} />
                {target.column.editable ? "Edit value" : "View full value"}
              </button>
            )}
            {hasReferencedKeys && onGoToReferenced && (
              <button type="button" className="button ghost small" onClick={onGoToReferenced}>
                <ArrowUpRight size={12} />
                Go to referenced row
              </button>
            )}
            {hasReferencingKeys && onGoToReferencing && (
              <button type="button" className="button ghost small" onClick={onGoToReferencing}>
                <ArrowDownLeft size={12} />
                Go to referencing rows
              </button>
            )}
          </div>
        </>
      ) : (
        <div className="inspector-empty">
          <Database size={23} />
          <p>Select a cell to see its value and type.</p>
        </div>
      )}
    </aside>
  );
}
