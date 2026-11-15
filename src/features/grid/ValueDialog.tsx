import { useMemo, useState, type FormEvent } from "react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { tabNavigation } from "../../components/ui/tabs";
import { dirtyStore } from "../../data/dirty-store";
import { resultCache } from "../../data/result-cache";
import { useStudio } from "../../state/studio";
import type { CellTarget } from "./DatabaseGrid";
import {
  MAX_HEX_PREVIEW_BYTES,
  buildJsonTree,
  decodeBinaryPreview,
  formatHexDump,
  formatJsonLossless,
  isBinaryColumn,
  isJsonColumn,
  isPostgresArrayColumn,
  isRawHexBinaryType,
  isSpatialColumn,
  parsePostgresArray,
  parseSpatialPreview,
  type JsonTreeResult,
  type SpatialPreviewResult,
} from "./value-format";

export function JsonTreeViewer({
  tree,
  onCopyPath,
}: {
  tree: JsonTreeResult;
  onCopyPath?: (path: string) => void;
}) {
  if (!tree.ok) {
    return (
      <div className="value-viewer array-viewer-empty" data-testid="json-tree-viewer">
        Current text is not valid JSON ({tree.error}). Switch to Raw view to inspect or edit.
      </div>
    );
  }
  return (
    <div className="value-viewer array-viewer" data-testid="json-tree-viewer" tabIndex={0}>
      <table className="array-elements-table" aria-label="Semi-structured path tree">
        <thead>
          <tr>
            <th scope="col">Path</th>
            <th scope="col">Type</th>
            <th scope="col">Value</th>
            <th scope="col">Action</th>
          </tr>
        </thead>
        <tbody>
          {tree.nodes.map((node) => (
            <tr key={node.path}>
              <td
                className="array-index-cell"
                style={{ paddingLeft: `${Math.min(node.depth, 8) * 12 + 8}px` }}
              >
                <code>{node.path}</code>
              </td>
              <td>
                <span className="array-type-tag">{node.kind}</span>
              </td>
              <td className="array-value-cell">
                {node.kind === "null" ? (
                  <span className="array-null-badge">null</span>
                ) : (
                  <code className="array-element-code">{node.summary}</code>
                )}
              </td>
              <td>
                <button
                  type="button"
                  className="button ghost small"
                  aria-label={`Copy path ${node.path}`}
                  onClick={() => {
                    void navigator.clipboard?.writeText(node.path).catch(() => {});
                    onCopyPath?.(node.path);
                  }}
                >
                  Copy :path
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function SpatialViewer({ spatial }: { spatial: SpatialPreviewResult }) {
  if (!spatial.ok) {
    return (
      <div className="value-viewer array-viewer-empty" data-testid="spatial-viewer">
        Could not parse spatial value ({spatial.error}). Switch to Raw view to inspect the literal.
      </div>
    );
  }
  return (
    <div className="value-viewer" data-testid="spatial-viewer" tabIndex={0}>
      <div className="query-actions" style={{ marginBottom: 8, flexWrap: "wrap" }}>
        <span className="sql-nav-badge">{spatial.format}</span>
        <span className="sql-nav-badge">{spatial.geometryType}</span>
        {spatial.srid !== null && <span className="sql-nav-badge">SRID {spatial.srid}</span>}
        <span className="sql-nav-badge">
          {spatial.coordinateCount.toLocaleString()}{" "}
          {spatial.coordinateCount === 1 ? "vertex" : "vertices"}
        </span>
        {spatial.bbox && (
          <span className="mono" style={{ fontSize: 12 }}>
            bbox [{spatial.bbox.map((n) => Number(n.toFixed(6))).join(", ")}]
          </span>
        )}
      </div>
      {spatial.shapes.length > 0 ? (
        <svg
          viewBox="0 0 240 160"
          width="100%"
          style={{
            maxHeight: 180,
            display: "block",
            marginBottom: 8,
            background: "rgba(15, 23, 42, 0.35)",
            borderRadius: 6,
            border: "1px solid rgba(148, 163, 184, 0.25)",
          }}
          role="img"
          aria-label={`Spatial ${spatial.geometryType} preview`}
          data-testid="spatial-svg-preview"
        >
          {spatial.shapes.map((shape, idx) =>
            shape.kind === "polygon" ? (
              <path
                key={idx}
                d={shape.d}
                fill="rgba(56, 189, 248, 0.22)"
                stroke="#38bdf8"
                strokeWidth={1.75}
                fillRule="evenodd"
              />
            ) : shape.kind === "polyline" ? (
              <path
                key={idx}
                d={shape.d}
                fill="none"
                stroke="#38bdf8"
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            ) : (
              <circle
                key={idx}
                cx={shape.cx}
                cy={shape.cy}
                r={4.5}
                fill="#38bdf8"
                stroke="#0f172a"
                strokeWidth={1.5}
              />
            ),
          )}
        </svg>
      ) : (
        <p role="note">Geometry is empty (0 coordinates).</p>
      )}
      <pre className="json-viewer" style={{ margin: 0, maxHeight: 180, overflow: "auto" }}>
        {spatial.formatted}
      </pre>
    </div>
  );
}

export function ValueDialog({ target, onClose }: { target: CellTarget; onClose(): void }) {
  const notify = useStudio((s) => s.notify);
  const canUseDefault = !!target.insert && target.column.hasDefault && target.column.editable;
  const viewerKind = isBinaryColumn(target.column)
    ? "binary"
    : isPostgresArrayColumn(target.column)
      ? "pgArray"
      : isSpatialColumn(target.column)
        ? "spatial"
        : isJsonColumn(target.column)
          ? "json"
          : null;

  const [usingDefault, setUsingDefault] = useState(canUseDefault && !!target.insert?.usingDefault),
    [value, setValue] = useState(target.value ?? ""),
    [isNull, setNull] = useState(
      target.insert ? target.insert.explicitNull : target.value === null,
    ),
    [viewMode, setViewMode] = useState<"formatted" | "tree" | "raw">(() => {
      if (viewerKind === "binary" || viewerKind === "pgArray" || viewerKind === "spatial")
        return "formatted";
      if (viewerKind === "json") {
        const initial = (target.value ?? "").trim();
        return initial.length > 0 && formatJsonLossless(initial).ok ? "formatted" : "raw";
      }
      return "raw";
    }),
    [error, setError] = useState<string | null>(null);

  const showViewerTabs = viewerKind !== null && !isNull && !usingDefault;

  const binaryPreview = useMemo(() => {
    if (viewerKind !== "binary" || isNull || usingDefault) return null;
    const preview = decodeBinaryPreview(
      value,
      MAX_HEX_PREVIEW_BYTES,
      isRawHexBinaryType(target.column.databaseType),
    );
    return { ...preview, dump: formatHexDump(preview.bytes) };
  }, [viewerKind, value, isNull, usingDefault, target.column.databaseType]);

  const arrayPreview = useMemo(() => {
    if (viewerKind !== "pgArray" || isNull || usingDefault) return null;
    return parsePostgresArray(value);
  }, [viewerKind, value, isNull, usingDefault]);

  const spatialPreview = useMemo(() => {
    if (viewerKind !== "spatial" || viewMode !== "formatted" || isNull || usingDefault) return null;
    return parseSpatialPreview(value);
  }, [viewerKind, viewMode, value, isNull, usingDefault]);

  const jsonPreview = useMemo(() => {
    if (viewerKind !== "json" || viewMode !== "formatted" || isNull || usingDefault) return null;
    return formatJsonLossless(value);
  }, [viewerKind, viewMode, value, isNull, usingDefault]);

  const jsonTree = useMemo(() => {
    if (viewerKind !== "json" || viewMode !== "tree" || isNull || usingDefault) return null;
    return buildJsonTree(value, target.column.name);
  }, [viewerKind, viewMode, value, isNull, usingDefault, target.column.name]);

  function handleFormatJson() {
    const res = formatJsonLossless(value);
    if (!res.ok) {
      setError(res.error);
      setViewMode("raw");
      return;
    }
    setError(null);
    setValue(res.formatted);
    setViewMode("raw");
  }

  function save(event: FormEvent) {
    event.preventDefault();
    try {
      if (target.insert) {
        if (usingDefault) {
          dirtyStore.setInsertCell(
            target.insert.tabId,
            target.insert.insertId,
            target.column.id,
            undefined,
          );
          onClose();
          return;
        }
        if (!isNull && target.column.kind === "json") JSON.parse(value);
        if (!isNull && target.column.kind === "integer" && !/^[+-]?\d+$/.test(value))
          throw new Error("Enter a whole number without losing precision");
        dirtyStore.setInsertCell(
          target.insert.tabId,
          target.insert.insertId,
          target.column.id,
          isNull ? null : value,
        );
        onClose();
        return;
      }
      if (!isNull && target.column.kind === "json") JSON.parse(value);
      if (!isNull && target.column.kind === "integer" && !/^[+-]?\d+$/.test(value))
        throw new Error("Enter a whole number without losing precision");
      dirtyStore.set({
        resultId: target.resultId,
        row: target.row,
        column: target.column.id,
        original:
          resultCache.read(target.resultId, target.column.id, target.row, true)?.value ?? null,
        value: isNull ? null : value,
      });
      onClose();
    } catch (error) {
      setError(error instanceof Error ? error.message : "The value is invalid");
    }
  }

  const formattedTabLabel =
    viewerKind === "binary"
      ? "Hex / ASCII"
      : viewerKind === "pgArray"
        ? "Elements"
        : viewerKind === "spatial"
          ? "Spatial"
          : "Formatted";

  return (
    <Modal
      title={target.column.editable ? `Edit ${target.column.name}` : target.column.name}
      subtitle={`${target.column.databaseType}${target.column.primaryKey ? " · primary key" : ""}`}
      onClose={onClose}
      wide
    >
      <form onSubmit={save}>
        <div className="modal-body">
          {canUseDefault && (
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={usingDefault}
                onChange={(e) => {
                  setUsingDefault(e.target.checked);
                  if (e.target.checked) setNull(false);
                }}
              />
              Use database default
            </label>
          )}
          {target.column.editable && target.column.nullable && (
            <label className="checkbox-label">
              <input
                type="checkbox"
                checked={!usingDefault && isNull}
                disabled={usingDefault}
                onChange={(e) => {
                  setNull(e.target.checked);
                  if (e.target.checked) setUsingDefault(false);
                }}
              />
              Set to NULL
            </label>
          )}
          {showViewerTabs && (
            <div className="value-viewer-toolbar">
              <div
                className="value-viewer-tabs"
                role="tablist"
                aria-label="Value view"
                onKeyDown={tabNavigation}
              >
                <button
                  type="button"
                  role="tab"
                  aria-selected={viewMode === "formatted"}
                  tabIndex={viewMode === "formatted" ? 0 : -1}
                  className={`value-viewer-tab ${viewMode === "formatted" ? "active" : ""}`}
                  onClick={() => setViewMode("formatted")}
                >
                  {formattedTabLabel}
                </button>
                {viewerKind === "json" && (
                  <button
                    type="button"
                    role="tab"
                    aria-selected={viewMode === "tree"}
                    tabIndex={viewMode === "tree" ? 0 : -1}
                    className={`value-viewer-tab ${viewMode === "tree" ? "active" : ""}`}
                    onClick={() => setViewMode("tree")}
                  >
                    Tree (:path)
                  </button>
                )}
                <button
                  type="button"
                  role="tab"
                  aria-selected={viewMode === "raw"}
                  tabIndex={viewMode === "raw" ? 0 : -1}
                  className={`value-viewer-tab ${viewMode === "raw" ? "active" : ""}`}
                  onClick={() => setViewMode("raw")}
                >
                  Raw
                </button>
              </div>
              <div className="value-viewer-meta">
                {viewerKind === "binary" && binaryPreview && (
                  <span>
                    {binaryPreview.totalBytes.toLocaleString()}{" "}
                    {binaryPreview.totalBytes === 1 ? "byte" : "bytes"}
                    {binaryPreview.truncated
                      ? ` · showing first ${MAX_HEX_PREVIEW_BYTES.toLocaleString()} bytes`
                      : ""}
                  </span>
                )}
                {viewerKind === "pgArray" && arrayPreview && (
                  <span>
                    {arrayPreview.ok
                      ? `${arrayPreview.totalElements.toLocaleString()} ${arrayPreview.totalElements === 1 ? "element" : "elements"}${arrayPreview.dimensions > 1 ? ` · ${arrayPreview.dimensions}D` : ""}${arrayPreview.truncated ? ` · showing first ${arrayPreview.elements.length.toLocaleString()}` : ""}`
                      : "Unparsed array literal"}
                  </span>
                )}
                {viewerKind === "json" && viewMode === "tree" && jsonTree?.ok && (
                  <span>
                    {jsonTree.totalNodes.toLocaleString()}{" "}
                    {jsonTree.totalNodes === 1 ? "node" : "nodes"}
                    {jsonTree.truncated
                      ? ` · showing first ${jsonTree.nodes.length.toLocaleString()}`
                      : ""}
                  </span>
                )}
                {viewerKind === "json" && target.column.editable && (
                  <button type="button" className="button ghost small" onClick={handleFormatJson}>
                    Format JSON
                  </button>
                )}
              </div>
            </div>
          )}
          {showViewerTabs && viewMode === "tree" && viewerKind === "json" && jsonTree ? (
            <JsonTreeViewer tree={jsonTree} onCopyPath={(p) => notify(`Copied path ${p}`)} />
          ) : showViewerTabs && viewMode === "formatted" ? (
            viewerKind === "binary" && binaryPreview ? (
              <pre
                className="value-viewer hex-dump"
                data-testid="binary-hex-viewer"
                aria-label="Binary hex and ASCII view"
                tabIndex={0}
              >
                {binaryPreview.totalBytes === 0
                  ? "0 bytes (empty binary value)"
                  : binaryPreview.dump}
              </pre>
            ) : viewerKind === "spatial" && spatialPreview ? (
              <SpatialViewer spatial={spatialPreview} />
            ) : viewerKind === "pgArray" && arrayPreview ? (
              !arrayPreview.ok ? (
                <div className="value-viewer array-viewer-empty" data-testid="pg-array-viewer">
                  {arrayPreview.error} Switch to Raw view to inspect the literal.
                </div>
              ) : arrayPreview.totalElements === 0 ? (
                <div className="value-viewer array-viewer-empty" data-testid="pg-array-viewer">
                  {"Empty array ({})"}
                </div>
              ) : (
                <div
                  className="value-viewer array-viewer"
                  data-testid="pg-array-viewer"
                  tabIndex={0}
                >
                  <table className="array-elements-table" aria-label="Array elements">
                    <thead>
                      <tr>
                        <th scope="col">Index</th>
                        <th scope="col">Element</th>
                      </tr>
                    </thead>
                    <tbody>
                      {arrayPreview.elements.map((item) => {
                        const ambiguousNullString =
                          !item.isNull && item.value !== null && /^null$/i.test(item.value);
                        const emptyString = !item.isNull && item.value === "";
                        return (
                          <tr key={item.indexLabel}>
                            <td className="array-index-cell">
                              <code>{item.indexLabel}</code>
                            </td>
                            <td className="array-value-cell">
                              {item.isNull ? (
                                <>
                                  <span className="array-null-badge">NULL</span>
                                  <span className="array-type-tag">SQL NULL</span>
                                </>
                              ) : ambiguousNullString ? (
                                <>
                                  <code className="array-element-code">
                                    {JSON.stringify(item.value)}
                                  </code>
                                  <span className="array-type-tag">string</span>
                                </>
                              ) : emptyString ? (
                                <>
                                  <code className="array-element-code">{'""'}</code>
                                  <span className="array-type-tag">empty string</span>
                                </>
                              ) : (
                                <code className="array-element-code">{item.value}</code>
                              )}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )
            ) : viewerKind === "json" && jsonPreview ? (
              jsonPreview.ok ? (
                <pre
                  className="value-viewer json-viewer"
                  data-testid="json-formatted-viewer"
                  aria-label="Formatted JSON view"
                  tabIndex={0}
                >
                  {jsonPreview.formatted}
                </pre>
              ) : (
                <div
                  className="value-viewer array-viewer-empty"
                  data-testid="json-formatted-viewer"
                >
                  Current text is not valid JSON ({jsonPreview.error}). Switch to Raw view to edit.
                </div>
              )
            ) : null
          ) : target.column.editable &&
            target.column.databaseType === "date" &&
            !isNull &&
            !usingDefault ? (
            <input type="date" value={value} onChange={(e) => setValue(e.target.value)} />
          ) : (
            <textarea
              className="value-editor"
              aria-label="Cell value"
              autoFocus
              value={usingDefault ? "<default>" : isNull ? "NULL" : value}
              onChange={(e) => setValue(e.target.value)}
              readOnly={!target.column.editable || isNull || usingDefault}
              spellCheck={false}
            />
          )}
          <InlineError message={error} />
        </div>
        <footer className="modal-footer">
          <button className="button ghost" type="button" onClick={onClose}>
            {target.column.editable ? "Cancel" : "Close"}
          </button>
          {target.column.editable && (
            <button type="submit" className="button primary">
              Keep edit
            </button>
          )}
        </footer>
      </form>
    </Modal>
  );
}
