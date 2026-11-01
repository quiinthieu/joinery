import { useEffect, useRef } from "react";
import { ArrowUpRight, BookOpen, GitBranch, X } from "lucide-react";
import type { DatabaseObjectKind, DatabaseObjectRef, ObjectDetails } from "../../ipc/generated";
import { objectTabTitle } from "../connections/explorer-tree";
import { ObjectKindGlyph } from "./ObjectInspectorView";
import type { ResolvedSqlSymbol } from "./sql-navigation";

const OBJECT_KIND_BADGES: Record<DatabaseObjectKind, string> = {
  table: "TABLE",
  partitionedTable: "PARTITIONED TABLE",
  view: "VIEW",
  materializedView: "MATERIALIZED VIEW",
  function: "FUNCTION",
  procedure: "PROCEDURE",
  sequence: "SEQUENCE",
  trigger: "TRIGGER",
  index: "INDEX",
};

interface Props {
  mode: "docs" | "usages";
  symbol: ResolvedSqlSymbol;
  objectDetails: ObjectDetails | null;
  objectDetailsLoading: boolean;
  objectDetailsError: string | null;
  objectExplorerSupported: boolean;
  onJumpInEditor(from: number, to: number): void;
  onOpenDefinition(): void;
  onOpenObject?(object: DatabaseObjectRef, highlightedColumn?: string): void;
  onSwitchMode(mode: "docs" | "usages"): void;
  onClose(): void;
}

function symbolTitle(symbol: ResolvedSqlSymbol): { badge: string; title: string } {
  switch (symbol.kind) {
    case "cte":
      return { badge: "CTE", title: symbol.name };
    case "alias":
      return {
        badge: "ALIAS",
        title: symbol.table
          ? `${symbol.name} → ${symbol.table.schema}.${symbol.table.name}`
          : symbol.cteName
            ? `${symbol.name} → ${symbol.cteName}`
            : symbol.name,
      };
    case "column":
      return {
        badge: "COLUMN",
        title: `${symbol.table.schema}.${symbol.table.name}.${symbol.name}`,
      };
    case "object":
      return {
        badge: OBJECT_KIND_BADGES[symbol.object.kind],
        title: `${symbol.object.schema}.${objectTabTitle(symbol.object)}`,
      };
  }
}

export function SqlNavigationOverlay({
  mode,
  symbol,
  objectDetails,
  objectDetailsLoading,
  objectDetailsError,
  objectExplorerSupported,
  onJumpInEditor,
  onOpenDefinition,
  onOpenObject,
  onSwitchMode,
  onClose,
}: Props) {
  const region = useRef<HTMLDivElement>(null);
  useEffect(() => {
    region.current?.focus();
  }, [mode]);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [onClose]);

  const { badge, title } = symbolTitle(symbol);
  const catalogTarget: DatabaseObjectRef | null =
    symbol.kind === "object"
      ? symbol.object
      : symbol.kind === "column"
        ? symbol.object
        : symbol.kind === "alias"
          ? symbol.object
          : null;

  const fkRefs =
    symbol.kind === "column" && symbol.schema
      ? symbol.schema.foreignKeys.flatMap((fk) => {
          const idx = fk.columns.indexOf(symbol.name);
          return idx >= 0
            ? [`${fk.target.schema}.${fk.target.name}.${fk.targetColumns[idx]} (${fk.name})`]
            : [];
        })
      : [];

  const declarationOcc = symbol.occurrences.find((o) => o.isDeclaration);

  return (
    <div
      className="sql-nav-overlay"
      ref={region}
      tabIndex={-1}
      role="region"
      aria-label={mode === "docs" ? "Quick SQL documentation" : "SQL symbol usages"}
    >
      <div className="sql-nav-header">
        <div className="sql-nav-heading">
          <span className="sql-nav-badge">{badge}</span>
          <strong className="mono">{title}</strong>
        </div>
        <div className="sql-nav-actions">
          <button
            type="button"
            className={`button ghost small ${mode === "docs" ? "active" : ""}`}
            aria-pressed={mode === "docs"}
            onClick={() => onSwitchMode("docs")}
          >
            <BookOpen size={11} />
            Docs
          </button>
          <button
            type="button"
            className={`button ghost small ${mode === "usages" ? "active" : ""}`}
            aria-pressed={mode === "usages"}
            onClick={() => onSwitchMode("usages")}
          >
            <GitBranch size={11} />
            Usages ({symbol.occurrences.length})
          </button>
          <button type="button" className="button ghost small" onClick={onOpenDefinition}>
            <ArrowUpRight size={11} />
            Definition
          </button>
          <button
            type="button"
            className="icon-button"
            aria-label="Close SQL symbol inspector"
            onClick={onClose}
          >
            <X size={12} />
          </button>
        </div>
      </div>

      {mode === "docs" ? (
        <div className="sql-nav-body">
          {objectDetailsError && (
            <p role="status" className="sql-nav-empty">
              Catalog metadata unavailable: {objectDetailsError}
            </p>
          )}
          {objectDetails?.comment && <p className="sql-nav-comment">{objectDetails.comment}</p>}

          {symbol.kind === "cte" && (
            <dl className="sql-nav-dl">
              <dt>Construct</dt>
              <dd>Common Table Expression (WITH)</dd>
              {declarationOcc && (
                <>
                  <dt>Declared at</dt>
                  <dd>
                    Line {declarationOcc.line}, Col {declarationOcc.column}
                  </dd>
                </>
              )}
              <dt>Columns</dt>
              <dd className="mono">
                {symbol.columns.length > 0 ? symbol.columns.join(", ") : "Not declared explicitly"}
              </dd>
              <dt>Local usages</dt>
              <dd>{symbol.occurrences.length} in current statement</dd>
            </dl>
          )}

          {symbol.kind === "alias" && (
            <>
              <dl className="sql-nav-dl">
                <dt>Construct</dt>
                <dd>Query Table Alias</dd>
                {declarationOcc && (
                  <>
                    <dt>Declared at</dt>
                    <dd>
                      Line {declarationOcc.line}, Col {declarationOcc.column}
                    </dd>
                  </>
                )}
                <dt>Target</dt>
                <dd className="mono">
                  {symbol.table
                    ? `${symbol.table.schema}.${symbol.table.name}`
                    : symbol.cteName
                      ? `CTE ${symbol.cteName}`
                      : "Derived subquery"}
                </dd>
                {symbol.schema && (
                  <>
                    <dt>Columns</dt>
                    <dd>
                      {symbol.schema.columns.length} columns
                      {symbol.schema.primaryKey.length > 0
                        ? ` · PK (${symbol.schema.primaryKey.join(", ")})`
                        : ""}
                    </dd>
                  </>
                )}
              </dl>
              {symbol.object && onOpenObject && (
                <div className="sql-nav-footer-row">
                  <button
                    type="button"
                    className="button ghost small"
                    onClick={() => onOpenObject(symbol.object!)}
                  >
                    <ArrowUpRight size={11} />
                    Open {symbol.object.schema}.{symbol.object.name} structure
                  </button>
                </div>
              )}
            </>
          )}

          {symbol.kind === "column" && (
            <dl className="sql-nav-dl">
              <dt>Relation</dt>
              <dd className="mono">
                {symbol.table.schema}.{symbol.table.name}
              </dd>
              <dt>Type</dt>
              <dd>
                <code>{symbol.column?.databaseType ?? "Loading…"}</code>
              </dd>
              {symbol.column && (
                <>
                  <dt>Nullable</dt>
                  <dd>{symbol.column.nullable ? "Yes (NULL)" : "No (NOT NULL)"}</dd>
                  <dt>Attributes</dt>
                  <dd>
                    {[
                      symbol.column.primaryKey ? "PRIMARY KEY" : null,
                      symbol.column.foreignKey ? "FOREIGN KEY" : null,
                      symbol.column.hasDefault ? "DEFAULT" : null,
                    ]
                      .filter(Boolean)
                      .join(" · ") || "—"}
                  </dd>
                </>
              )}
              {fkRefs.length > 0 && (
                <>
                  <dt>References</dt>
                  <dd className="mono">{fkRefs.join(", ")}</dd>
                </>
              )}
            </dl>
          )}

          {symbol.kind === "object" && (
            <>
              {(objectDetails?.routine || symbol.summary?.signature) && (
                <dl className="sql-nav-dl">
                  <dt>Arguments</dt>
                  <dd className="mono">
                    {objectDetails?.routine?.arguments ?? symbol.summary?.signature ?? "()"}
                  </dd>
                  {objectDetails?.routine?.returnType && (
                    <>
                      <dt>Returns</dt>
                      <dd>
                        <code>{objectDetails.routine.returnType}</code>
                      </dd>
                    </>
                  )}
                  {objectDetails?.routine && (
                    <>
                      <dt>Language</dt>
                      <dd>
                        {objectDetails.routine.language}
                        {objectDetails.routine.volatility
                          ? ` · ${objectDetails.routine.volatility}`
                          : ""}
                        {objectDetails.routine.securityDefiner ? " · SECURITY DEFINER" : ""}
                      </dd>
                    </>
                  )}
                </dl>
              )}

              {objectDetails?.sequence && (
                <dl className="sql-nav-dl">
                  <dt>Data type</dt>
                  <dd>
                    <code>{objectDetails.sequence.dataType}</code>
                  </dd>
                  <dt>Progression</dt>
                  <dd className="mono">
                    START {objectDetails.sequence.startValue} · INCREMENT{" "}
                    {objectDetails.sequence.incrementBy} ·{" "}
                    {objectDetails.sequence.cycle ? "CYCLE" : "NO CYCLE"}
                  </dd>
                  <dt>Bounds</dt>
                  <dd className="mono">
                    {objectDetails.sequence.minValue} .. {objectDetails.sequence.maxValue}
                  </dd>
                </dl>
              )}

              {symbol.schema && (
                <div className="sql-nav-columns">
                  <div className="sql-nav-section-head">
                    <span>Columns ({symbol.schema.columns.length})</span>
                    {symbol.estimatedRows !== null && (
                      <span>~{symbol.estimatedRows.toLocaleString()} rows</span>
                    )}
                  </div>
                  <div className="sql-nav-column-list">
                    {symbol.schema.columns.slice(0, 10).map((col) => (
                      <div key={col.id} className="sql-nav-column-row">
                        <span className="mono">{col.name}</span>
                        <code>{col.databaseType}</code>
                        <span className="sql-nav-col-flags">
                          {col.primaryKey
                            ? "PK"
                            : col.foreignKey
                              ? "FK"
                              : !col.nullable
                                ? "NOT NULL"
                                : ""}
                        </span>
                      </div>
                    ))}
                    {symbol.schema.columns.length > 10 && (
                      <div className="sql-nav-more">
                        +{symbol.schema.columns.length - 10} more columns
                      </div>
                    )}
                  </div>
                </div>
              )}

              {!symbol.schema &&
                !objectDetails?.routine &&
                !objectDetails?.sequence &&
                !symbol.summary?.signature && (
                  <div className="sql-nav-empty">
                    {objectDetailsLoading
                      ? "Loading catalog metadata…"
                      : "Press F12 or Definition to inspect full structure."}
                  </div>
                )}
            </>
          )}
        </div>
      ) : (
        <div className="sql-nav-body">
          {symbol.usagesLimited && (
            <p role="status" className="sql-nav-empty">
              Showing up to 500 references from the first 10,000 SQL name tokens. Narrow the editor
              text to inspect more.
            </p>
          )}
          <div className="sql-nav-section">
            <div className="sql-nav-section-head">
              <span>In this editor</span>
              <span>{symbol.occurrences.length}</span>
            </div>
            <div className="sql-nav-usage-list">
              {symbol.occurrences.map((occ) => (
                <button
                  type="button"
                  key={`${occ.from}:${occ.to}`}
                  className="sql-nav-usage-row"
                  onClick={() => onJumpInEditor(occ.from, occ.to)}
                >
                  <span className="sql-nav-loc">
                    Ln {occ.line}, Col {occ.column}
                  </span>
                  <span className={`sql-nav-pill ${occ.isDeclaration ? "decl" : ""}`}>
                    {occ.isDeclaration ? "Declaration" : "Reference"}
                  </span>
                  <code className="mono sql-nav-snippet">{occ.lineText}</code>
                </button>
              ))}
            </div>
          </div>

          {catalogTarget && objectExplorerSupported && (
            <div className="sql-nav-section">
              <div className="sql-nav-section-head">
                <span>
                  Database dependents of {catalogTarget.schema}.{catalogTarget.name} (object level)
                </span>
                {objectDetails && <span>{objectDetails.usedBy.length}</span>}
              </div>
              {objectDetailsLoading && !objectDetails ? (
                <div className="sql-nav-empty">Loading database dependencies…</div>
              ) : objectDetailsError ? (
                <div className="sql-nav-empty">{objectDetailsError}</div>
              ) : !objectDetails ? (
                <div className="sql-nav-empty">Database dependencies unavailable.</div>
              ) : objectDetails.usedBy.length === 0 ? (
                <div className="sql-nav-empty">No database objects depend on this object.</div>
              ) : (
                <div className="sql-nav-usage-list">
                  {objectDetails.usedBy.map((edge) => {
                    const depLabel = `${edge.object.schema}.${objectTabTitle(edge.object)}`;
                    return (
                      <button
                        type="button"
                        key={`${edge.object.schema}:${edge.object.kind}:${edge.object.name}:${edge.object.identity ?? ""}:${edge.relation}`}
                        className="sql-nav-usage-row"
                        disabled={!onOpenObject}
                        onClick={() => onOpenObject?.(edge.object)}
                      >
                        <ObjectKindGlyph kind={edge.object.kind} size={12} />
                        <span className="mono sql-nav-snippet">{depLabel}</span>
                        <span className="sql-nav-pill">{edge.relation}</span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
