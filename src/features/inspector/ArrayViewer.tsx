import { useState, useMemo, useCallback } from "react";
import { Search, X, Copy, Check, Download, Layers } from "lucide-react";
import type { ColumnInfo } from "../../ipc/generated";
import {
  parseUniversalArray,
  filterArrayElements,
  exportArrayAsJson,
  exportArrayAsCsv,
  type ArrayElementRow,
} from "./array-format";
import { VariantPathHelper } from "./VariantPathHelper";
import { useStudio } from "../../state/studio";

export interface ArrayViewerProps {
  value: string | null;
  column?: Pick<ColumnInfo, "name" | "databaseType" | "kind">;
  compact?: boolean;
}

export function ArrayViewer({ value, column, compact = false }: ArrayViewerProps) {
  const [searchQuery, setSearchQuery] = useState("");
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [selectedRow, setSelectedRow] = useState<ArrayElementRow | null>(null);

  const arrayResult = useMemo(() => parseUniversalArray(value), [value]);

  const filteredElements = useMemo(() => {
    if (!arrayResult.ok) return [];
    return filterArrayElements(arrayResult.elements, searchQuery);
  }, [arrayResult, searchQuery]);

  // Initial selected element
  useMemo(() => {
    if (arrayResult.ok && arrayResult.elements.length > 0 && !selectedRow) {
      setSelectedRow(arrayResult.elements[0]);
    }
  }, [arrayResult, selectedRow]);

  const handleCopy = useCallback(async (text: string, label: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedKey(label);
    setTimeout(() => setCopiedKey(null), 1500);
    useStudio.getState().notify(`Copied ${label}`);
  }, []);

  if (value === null || !arrayResult.ok) {
    return (
      <div className="inspector-placeholder" data-testid="array-viewer">
        {value === null ? "NULL" : arrayResult.error || "Invalid array format"}
      </div>
    );
  }

  const isSnowflakeOrJson =
    arrayResult.format === "json" ||
    (column?.databaseType || "").toLowerCase().includes("variant") ||
    (column?.databaseType || "").toLowerCase().includes("array");

  return (
    <div
      className={`array-viewer-container ${compact ? "compact" : ""}`}
      data-testid="array-viewer"
    >
      {/* Top Controls: Search + Summary + Export Actions */}
      <div className="array-toolbar">
        <div className="array-search-wrapper">
          <Search size={12} className="array-search-icon" />
          <input
            type="text"
            className="array-search-input"
            placeholder="Filter elements..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            aria-label="Filter array elements"
          />
          {searchQuery && (
            <button
              type="button"
              className="icon-button micro array-search-clear"
              aria-label="Clear filter"
              onClick={() => setSearchQuery("")}
            >
              <X size={10} />
            </button>
          )}
        </div>

        <div className="array-meta-pill">
          <Layers size={11} />
          <span>
            {arrayResult.dimensions}D · {arrayResult.totalElements} item
            {arrayResult.totalElements === 1 ? "" : "s"}
          </span>
        </div>
      </div>

      {/* Action Bar: Export JSON & CSV */}
      <div className="array-actions-bar">
        <span className="array-filter-count">
          Showing {filteredElements.length} of {arrayResult.totalElements}
        </span>
        <div className="array-export-buttons">
          <button
            type="button"
            className="button ghost small"
            onClick={() => handleCopy(exportArrayAsJson(arrayResult.elements), "JSON")}
          >
            {copiedKey === "JSON" ? <Check size={11} /> : <Copy size={11} />}
            Copy JSON
          </button>
          <button
            type="button"
            className="button ghost small"
            onClick={() => handleCopy(exportArrayAsCsv(arrayResult.elements), "CSV")}
          >
            {copiedKey === "CSV" ? <Check size={11} /> : <Download size={11} />}
            Copy CSV
          </button>
        </div>
      </div>

      {/* Elements Table */}
      <div
        className="array-table-wrapper"
        role="region"
        aria-label="Array elements table"
        tabIndex={0}
      >
        <table className="array-elements-table" role="table">
          <thead>
            <tr>
              <th scope="col" className="col-idx">
                INDEX
              </th>
              <th scope="col" className="col-type">
                TYPE
              </th>
              <th scope="col" className="col-val">
                VALUE
              </th>
              <th scope="col" className="col-size">
                SIZE
              </th>
            </tr>
          </thead>
          <tbody>
            {filteredElements.map((row) => {
              const isSelected = selectedRow?.index === row.index;
              return (
                <tr
                  key={row.index}
                  className={`array-row ${isSelected ? "selected" : ""}`}
                  onClick={() => setSelectedRow(row)}
                  tabIndex={0}
                >
                  <td className="col-idx">
                    <code>{row.indexLabel}</code>
                  </td>
                  <td className="col-type">
                    <span className={`array-type-badge badge-${row.kind}`}>{row.kind}</span>
                  </td>
                  <td className="col-val">
                    {row.isNull ? (
                      <em className="array-null-val">NULL</em>
                    ) : (
                      <span className="array-cell-text">{row.displayValue}</span>
                    )}
                  </td>
                  <td className="col-size">
                    <span className="array-size-text">{row.lengthOrSize}</span>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      {/* Snowflake Semi-Structured Variant Path Helper */}
      {isSnowflakeOrJson && selectedRow && (
        <VariantPathHelper
          columnName={column?.name || "array_col"}
          selectedPath={selectedRow.path}
          valueType={selectedRow.kind}
          sampleValue={selectedRow.raw}
        />
      )}
    </div>
  );
}
