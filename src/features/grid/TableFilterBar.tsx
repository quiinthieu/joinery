import { useEffect, useRef, useState } from "react";
import { ArrowUpDown, Filter, Plus, X } from "lucide-react";
import type { BrowseQuery, ColumnInfo, FilterOp } from "../../ipc/generated";
import {
  FILTER_OPERATORS,
  allowedOperators,
  describeFilter,
  describeSort,
  hasRawClauses,
  needsValue,
  normalizeClause,
  withFilter,
  withoutFilter,
} from "./table-query";

interface Props {
  query: BrowseQuery;
  columns: ColumnInfo[];
  /** Non-null while controls are locked (pending edits, loading, reconnect). */
  disabledReason: string | null;
  /** Engine-specific note for text search semantics. */
  searchNote: string;
  onChange(query: BrowseQuery): void;
}

/** Typed server-side filters and the active sort for a table tab. */
export function TableFilterBar({ query, columns, disabledReason, searchNote, onChange }: Props) {
  const [adding, setAdding] = useState(false);
  const [column, setColumn] = useState(columns[0]?.name ?? "");
  const [op, setOp] = useState<FilterOp>("eq");
  const [value, setValue] = useState("");
  const [whereDraft, setWhereDraft] = useState(query.whereClause ?? "");
  const [orderDraft, setOrderDraft] = useState(query.orderClause ?? "");
  const first = useRef<HTMLSelectElement>(null);
  const selected = columns.find((c) => c.name === column) ?? columns[0];
  const operators = selected ? allowedOperators(selected) : [];
  const disabled = disabledReason !== null;
  const clausesDirty =
    normalizeClause(whereDraft) !== normalizeClause(query.whereClause) ||
    normalizeClause(orderDraft) !== normalizeClause(query.orderClause);

  useEffect(() => {
    setWhereDraft(query.whereClause ?? "");
    setOrderDraft(query.orderClause ?? "");
  }, [query.whereClause, query.orderClause]);

  useEffect(() => {
    if (adding) first.current?.focus();
  }, [adding]);
  useEffect(() => {
    if (selected && !operators.includes(op)) setOp(operators[0]);
  }, [column]);

  function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!selected || disabled) return;
    onChange(
      withFilter(query, {
        column: selected.name,
        op,
        value: needsValue(op) ? value : null,
      }),
    );
    setAdding(false);
    setValue("");
  }

  function submitClauses(event: React.FormEvent) {
    event.preventDefault();
    if (disabled) return;
    const whereClause = normalizeClause(whereDraft);
    const orderClause = normalizeClause(orderDraft);
    onChange({
      ...query,
      sort: orderClause ? [] : query.sort,
      whereClause,
      orderClause,
    });
  }

  return (
    <div className="table-filter-bar" role="toolbar" aria-label="Table filters">
      <Filter size={12} aria-hidden="true" />
      {query.filters.map((filter, index) => (
        <span className="filter-chip" key={`${index}:${describeFilter(filter)}`}>
          <span className="mono">{describeFilter(filter)}</span>
          <button
            className="icon-button"
            aria-label={`Remove filter ${describeFilter(filter)}`}
            disabled={disabled}
            onClick={() => onChange(withoutFilter(query, index))}
          >
            <X size={11} />
          </button>
        </span>
      ))}
      {adding ? (
        <form
          className="filter-form"
          onSubmit={submit}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.stopPropagation();
              setAdding(false);
            }
          }}
        >
          <select
            ref={first}
            aria-label="Filter column"
            value={selected?.name ?? ""}
            onChange={(event) => setColumn(event.target.value)}
          >
            {columns.map((c) => (
              <option key={c.id} value={c.name}>
                {c.name}
              </option>
            ))}
          </select>
          <select
            aria-label="Filter operator"
            value={op}
            onChange={(event) => setOp(event.target.value as FilterOp)}
          >
            {FILTER_OPERATORS.filter((o) => operators.includes(o.op)).map((o) => (
              <option key={o.op} value={o.op}>
                {o.label}
              </option>
            ))}
          </select>
          {needsValue(op) && (
            <input
              aria-label="Filter value"
              value={value}
              placeholder={op === "contains" || op === "startsWith" ? searchNote : "value"}
              onChange={(event) => setValue(event.target.value)}
            />
          )}
          <button className="button primary small" type="submit" disabled={disabled}>
            Apply
          </button>
          <button className="button ghost small" type="button" onClick={() => setAdding(false)}>
            Cancel
          </button>
        </form>
      ) : (
        <button
          className="button ghost small"
          disabled={disabled || !columns.length}
          onClick={() => setAdding(true)}
        >
          <Plus size={12} />
          Add filter
        </button>
      )}
      {query.filters.length > 0 && !adding && (
        <button
          className="button ghost small"
          disabled={disabled}
          onClick={() => onChange({ ...query, filters: [] })}
        >
          Clear filters
        </button>
      )}
      <form
        className="sql-clause-form"
        onSubmit={submitClauses}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            setWhereDraft(query.whereClause ?? "");
            setOrderDraft(query.orderClause ?? "");
          }
        }}
      >
        <label className="sql-clause-field">
          <span className="mono">WHERE</span>
          <input
            aria-label="WHERE clause"
            className="mono"
            value={whereDraft}
            disabled={disabled}
            placeholder="SQL predicate"
            onChange={(event) => setWhereDraft(event.target.value)}
          />
        </label>
        <label className="sql-clause-field">
          <span className="mono">ORDER BY</span>
          <input
            aria-label="ORDER BY clause"
            className="mono"
            value={orderDraft}
            disabled={disabled}
            placeholder="SQL order"
            onChange={(event) => setOrderDraft(event.target.value)}
          />
        </label>
        <button className="button ghost small" type="submit" disabled={disabled || !clausesDirty}>
          Run SQL clauses
        </button>
        {hasRawClauses(query) && (
          <button
            className="button ghost small"
            type="button"
            disabled={disabled}
            onClick={() => {
              setWhereDraft("");
              setOrderDraft("");
              onChange({ ...query, whereClause: null, orderClause: null });
            }}
          >
            Reset SQL clauses
          </button>
        )}
      </form>
      <span className="filter-spacer" />
      {normalizeClause(query.orderClause) && (
        <span className="filter-note">First 500 rows · custom ORDER BY</span>
      )}
      {query.sort.length > 0 && (
        <span className="sort-summary">
          <ArrowUpDown size={11} aria-hidden="true" />
          <span>Sorted by {describeSort(query)}</span>
          <button
            className="icon-button"
            aria-label="Clear sort"
            disabled={disabled}
            onClick={() => onChange({ ...query, sort: [] })}
          >
            <X size={11} />
          </button>
        </span>
      )}
      {disabledReason && <span className="filter-note">{disabledReason}</span>}
    </div>
  );
}
