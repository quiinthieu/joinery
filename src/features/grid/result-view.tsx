import { useCallback, useEffect, useRef, useState } from "react";
import { RotateCcw, Search, X } from "lucide-react";
import type {
  ColumnInfo,
  ResultInfo,
  ResultSortSpec,
  ResultViewResponse,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";

export const MAX_RESULT_SORT_KEYS = 4;
export const MAX_PINNED_RESULTS_PER_TAB = 8;

export function cycleResultSort(
  current: readonly ResultSortSpec[],
  columnId: string,
  additive: boolean,
  maxKeys: number = MAX_RESULT_SORT_KEYS,
): ResultSortSpec[] {
  const existingIdx = current.findIndex((s) => s.columnId === columnId);
  if (!additive) {
    if (existingIdx < 0 || current.length > 1) {
      return [{ columnId, descending: false }];
    }
    if (!current[0].descending) {
      return [{ columnId, descending: true }];
    }
    return [];
  }

  if (existingIdx < 0) {
    if (current.length >= maxKeys) {
      return [...current.slice(0, maxKeys - 1), { columnId, descending: false }];
    }
    return [...current, { columnId, descending: false }];
  }
  const existing = current[existingIdx];
  if (!existing.descending) {
    return current.map((entry, idx) =>
      idx === existingIdx ? { ...entry, descending: true } : entry,
    );
  }
  return current.filter((_, idx) => idx !== existingIdx);
}

export function resultSortIndicator(
  sort: readonly ResultSortSpec[] | undefined,
  columnId: string,
): string {
  if (!sort || sort.length === 0) return "";
  const idx = sort.findIndex((s) => s.columnId === columnId);
  if (idx < 0) return "";
  const arrow = sort[idx].descending ? "▼" : "▲";
  return sort.length > 1 ? `${arrow}${idx + 1}` : arrow;
}

export interface PerResultViewConfig {
  filterText: string;
  filterColumnId: string;
  caseSensitive: boolean;
  sort: ResultSortSpec[];
}

const DEFAULT_RESULT_VIEW_CONFIG: PerResultViewConfig = {
  filterText: "",
  filterColumnId: "",
  caseSensitive: false,
  sort: [],
};

export function isResultViewActive(config: PerResultViewConfig): boolean {
  return config.filterText.trim().length > 0 || config.sort.length > 0;
}

export interface UseResultViewOutcome {
  config: PerResultViewConfig;
  active: boolean;
  displayRowIndices: readonly number[] | null;
  response: ResultViewResponse | null;
  loading: boolean;
  error: string | null;
  setFilterText(value: string): void;
  setFilterColumnId(columnId: string): void;
  setCaseSensitive(value: boolean): void;
  toggleSort(column: ColumnInfo, additive: boolean): void;
  resetView(): void;
}

export function useResultViewState(
  result: ResultInfo | undefined,
  options?: {
    disabled?: boolean;
    lockedReason?: string | null;
    onNotify?: (message: string) => void;
  },
): UseResultViewOutcome {
  const [configs, setConfigs] = useState<Record<string, PerResultViewConfig>>({});
  const [responses, setResponses] = useState<Record<string, ResultViewResponse>>({});
  const [loadingId, setLoadingId] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const seqRef = useRef(0);

  const resultId = result?.id ?? "";
  const config = (resultId && configs[resultId]) || DEFAULT_RESULT_VIEW_CONFIG;
  const active = Boolean(resultId && !options?.disabled && isResultViewActive(config));

  const updateConfig = useCallback(
    (updater: (prev: PerResultViewConfig) => PerResultViewConfig) => {
      if (!resultId) return;
      if (options?.lockedReason) {
        options.onNotify?.(options.lockedReason);
        return;
      }
      setConfigs((prev) => {
        const current = prev[resultId] ?? DEFAULT_RESULT_VIEW_CONFIG;
        const next = updater(current);
        if (!isResultViewActive(next)) {
          const copy = { ...prev };
          delete copy[resultId];
          return copy;
        }
        return { ...prev, [resultId]: next };
      });
    },
    [options, resultId],
  );

  useEffect(() => {
    if (!result || !resultId || options?.disabled || result.status === "streaming") {
      return;
    }
    if (!isResultViewActive(config)) {
      setLoadingId((cur) => (cur === resultId ? null : cur));
      setErrors((prev) => {
        if (!(resultId in prev)) return prev;
        const copy = { ...prev };
        delete copy[resultId];
        return copy;
      });
      return;
    }

    const seq = ++seqRef.current;
    setLoadingId(resultId);
    setErrors((prev) => {
      if (!(resultId in prev)) return prev;
      const copy = { ...prev };
      delete copy[resultId];
      return copy;
    });

    void client
      .call("query_result_view", {
        request: {
          resultId,
          filterText: config.filterText.trim() || null,
          filterColumnId: config.filterColumnId || null,
          caseSensitive: config.caseSensitive,
          sort: config.sort,
        },
      })
      .then((res) => {
        if (seq !== seqRef.current) return;
        setResponses((prev) => ({ ...prev, [resultId]: res }));
        setLoadingId((cur) => (cur === resultId ? null : cur));
      })
      .catch((err) => {
        if (seq !== seqRef.current) return;
        setErrors((prev) => ({ ...prev, [resultId]: messageOf(err) }));
        setLoadingId((cur) => (cur === resultId ? null : cur));
      });
  }, [config, options?.disabled, result, result?.retainedRows, result?.status, resultId]);

  const response = active ? (responses[resultId] ?? null) : null;
  const displayRowIndices = active && response ? response.rowIndices : null;

  return {
    config,
    active,
    displayRowIndices,
    response,
    loading: active && loadingId === resultId,
    error: active ? (errors[resultId] ?? null) : null,
    setFilterText: (value) => updateConfig((prev) => ({ ...prev, filterText: value })),
    setFilterColumnId: (columnId) =>
      updateConfig((prev) => ({ ...prev, filterColumnId: columnId })),
    setCaseSensitive: (value) => updateConfig((prev) => ({ ...prev, caseSensitive: value })),
    toggleSort: (column, additive) => {
      if (column.kind === "binary") return;
      updateConfig((prev) => ({
        ...prev,
        sort: cycleResultSort(prev.sort, column.id, additive),
      }));
    },
    resetView: () => {
      if (!resultId) return;
      if (options?.lockedReason) {
        options.onNotify?.(options.lockedReason);
        return;
      }
      setConfigs((prev) => {
        if (!(resultId in prev)) return prev;
        const copy = { ...prev };
        delete copy[resultId];
        return copy;
      });
      setErrors((prev) => {
        if (!(resultId in prev)) return prev;
        const copy = { ...prev };
        delete copy[resultId];
        return copy;
      });
    },
  };
}

export function ResultQuickFilterBar({
  columns,
  view,
  disabled,
}: {
  columns: readonly ColumnInfo[];
  view: UseResultViewOutcome;
  disabled?: boolean;
}) {
  const { config, active, response, error } = view;
  const nonBinaryCols = columns.filter((c) => c.kind !== "binary");
  return (
    <div
      className="result-quick-filter"
      role="search"
      aria-label="Result row filter controls"
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: 6,
      }}
    >
      <label
        className="diagram-filter"
        style={{
          height: 26,
          minWidth: 165,
          maxWidth: 220,
          padding: "0 8px",
          display: "inline-flex",
          alignItems: "center",
          gap: 5,
        }}
      >
        <Search size={12} />
        <input
          type="search"
          aria-label="Filter result rows"
          placeholder="Filter rows…"
          disabled={disabled}
          value={config.filterText}
          onChange={(e) => view.setFilterText(e.target.value)}
          style={{ fontSize: 12, width: "100%" }}
        />
        {config.filterText && (
          <button
            type="button"
            className="icon-button"
            aria-label="Clear row filter"
            disabled={disabled}
            onClick={() => view.setFilterText("")}
            style={{ width: 16, height: 16, padding: 0 }}
          >
            <X size={11} />
          </button>
        )}
      </label>
      {nonBinaryCols.length > 1 && config.filterText.trim().length > 0 && (
        <select
          aria-label="Result filter column"
          disabled={disabled}
          value={config.filterColumnId}
          onChange={(e) => view.setFilterColumnId(e.target.value)}
          style={{ height: 26, fontSize: 11, maxWidth: 130 }}
        >
          <option value="">All columns</option>
          {nonBinaryCols.map((col) => (
            <option key={col.id} value={col.id}>
              {col.name}
            </option>
          ))}
        </select>
      )}
      {config.filterText.trim().length > 0 && (
        <button
          type="button"
          className={`button ghost small ${config.caseSensitive ? "active" : ""}`}
          aria-label="Match case in result filter"
          aria-pressed={config.caseSensitive}
          disabled={disabled}
          title="Match case (Aa)"
          onClick={() => view.setCaseSensitive(!config.caseSensitive)}
          style={{ padding: "0 6px", minWidth: 26 }}
        >
          Aa
        </button>
      )}
      {active && response && !error && (
        <span
          className="badge result-quick-filter-count"
          aria-live="polite"
          title={
            response.truncated
              ? `Showing first ${response.rowIndices.length.toLocaleString()} of ${response.matchedRows.toLocaleString()} matching rows (${response.scannedRows.toLocaleString()} scanned)`
              : `Showing ${response.rowIndices.length.toLocaleString()} of ${response.scannedRows.toLocaleString()} rows`
          }
        >
          {response.rowIndices.length.toLocaleString()} / {response.scannedRows.toLocaleString()}
          {response.truncated ? "+" : ""}
        </span>
      )}
      {error && (
        <span
          className="badge warning result-quick-filter-error"
          role="status"
          title={error}
          style={{
            maxWidth: 260,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {error}
        </span>
      )}
      {(active || Boolean(error)) && (
        <button
          type="button"
          className="button ghost small"
          disabled={disabled}
          title={
            error
              ? error
              : response
                ? `Showing ${response.rowIndices.length.toLocaleString()} of ${response.scannedRows.toLocaleString()} rows — click to reset`
                : "Reset result filter and sort"
          }
          onClick={view.resetView}
        >
          <RotateCcw size={11} />
          Reset view
        </button>
      )}
    </div>
  );
}
