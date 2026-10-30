import { useMemo, useState, type FormEvent } from "react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { tabNavigation } from "../../components/ui/tabs";
import { dirtyStore } from "../../data/dirty-store";
import { resultCache } from "../../data/result-cache";
import type { CellTarget } from "./DatabaseGrid";
import {
  MAX_HEX_PREVIEW_BYTES,
  decodeBinaryPreview,
  formatHexDump,
  formatJsonLossless,
  isBinaryColumn,
  isJsonColumn,
  isPostgresArrayColumn,
  parsePostgresArray,
} from "./value-format";

export function ValueDialog({ target, onClose }: { target: CellTarget; onClose(): void }) {
  const canUseDefault = !!target.insert && target.column.hasDefault && target.column.editable;
  const viewerKind = isBinaryColumn(target.column)
    ? "binary"
    : isPostgresArrayColumn(target.column)
      ? "pgArray"
      : isJsonColumn(target.column)
        ? "json"
        : null;

  const [usingDefault, setUsingDefault] = useState(canUseDefault && !!target.insert?.usingDefault),
    [value, setValue] = useState(target.value ?? ""),
    [isNull, setNull] = useState(
      target.insert ? target.insert.explicitNull : target.value === null,
    ),
    [viewMode, setViewMode] = useState<"formatted" | "raw">(() => {
      if (viewerKind === "binary" || viewerKind === "pgArray") return "formatted";
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
    const preview = decodeBinaryPreview(value);
    return { ...preview, dump: formatHexDump(preview.bytes) };
  }, [viewerKind, value, isNull, usingDefault]);

  const arrayPreview = useMemo(() => {
    if (viewerKind !== "pgArray" || isNull || usingDefault) return null;
    return parsePostgresArray(value);
  }, [viewerKind, value, isNull, usingDefault]);

  const jsonPreview = useMemo(() => {
    if (viewerKind !== "json" || viewMode !== "formatted" || isNull || usingDefault) return null;
    return formatJsonLossless(value);
  }, [viewerKind, viewMode, value, isNull, usingDefault]);

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
    viewerKind === "binary" ? "Hex / ASCII" : viewerKind === "pgArray" ? "Elements" : "Formatted";

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
                {viewerKind === "json" && target.column.editable && (
                  <button type="button" className="button ghost small" onClick={handleFormatJson}>
                    Format JSON
                  </button>
                )}
              </div>
            </div>
          )}
          {showViewerTabs && viewMode === "formatted" ? (
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
