import { useEffect, useMemo, useRef, useState } from "react";
import { KeyRound, Link2, Minus, Network, Plus, RefreshCw, Search } from "lucide-react";
import type { SchemaDiagram, TableRef } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { useStudio, type StudioTab } from "../../state/studio";
import {
  HEADER_HEIGHT,
  ROW_HEIGHT,
  filterDiagram,
  layoutDiagram,
  type DiagramLayout,
} from "./er-layout";

const MIN_SCALE = 0.25;
const MAX_SCALE = 2;

/** Moves keyboard focus between diagram tables. */
export function neighborNode(
  layout: DiagramLayout,
  current: string,
  key: "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Home" | "End",
): string {
  const ordered = layout.layers.flat();
  if (key === "Home") return ordered[0] ?? current;
  if (key === "End") return ordered[ordered.length - 1] ?? current;
  const layer = layout.layers.findIndex((keys) => keys.includes(current));
  if (layer < 0) return ordered[0] ?? current;
  const index = layout.layers[layer].indexOf(current);
  if (key === "ArrowUp") return layout.layers[layer][Math.max(0, index - 1)];
  if (key === "ArrowDown")
    return layout.layers[layer][Math.min(layout.layers[layer].length - 1, index + 1)];
  const step = key === "ArrowLeft" ? -1 : 1;
  for (let next = layer + step; next >= 0 && next < layout.layers.length; next += step) {
    const keys = layout.layers[next];
    if (keys.length) return keys[Math.min(index, keys.length - 1)];
  }
  return current;
}

export function DiagramView({
  tab,
  onOpenTable,
}: {
  tab: StudioTab;
  onOpenTable(table: TableRef): void;
}) {
  const session = useStudio((s) => (tab.connectionId ? s.sessions[tab.connectionId] : null));
  const schema = tab.diagramSchema ?? "";
  const [diagram, setDiagram] = useState<SchemaDiagram | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState("");
  const [scale, setScale] = useState(1);
  const [focused, setFocused] = useState<string | null>(null);
  const request = useRef(0);
  const canvas = useRef<HTMLDivElement>(null);

  async function refresh() {
    if (!session || !schema) return;
    const id = ++request.current;
    setLoading(true);
    try {
      const next = await client.call<SchemaDiagram>("schema_diagram", {
        sessionId: session.id,
        schema,
      });
      if (id !== request.current) return;
      setDiagram(next);
      setError(null);
    } catch (failure) {
      if (id !== request.current) return;
      setDiagram(null);
      setError(messageOf(failure));
    } finally {
      if (id === request.current) setLoading(false);
    }
  }
  useEffect(() => {
    setDiagram(null);
    void refresh();
    return () => {
      request.current++;
    };
  }, [session?.id, schema]);

  const layout = useMemo(
    () => (diagram ? layoutDiagram(filterDiagram(diagram, filter)) : null),
    [diagram, filter],
  );
  const nodeByKey = useMemo(
    () => new Map(layout?.nodes.map((node) => [node.key, node]) ?? []),
    [layout],
  );
  const active = focused && nodeByKey.has(focused) ? focused : (layout?.nodes[0]?.key ?? null);

  function focusNode(key: string) {
    setFocused(key);
    const nodes = canvas.current?.querySelectorAll<SVGGElement>("[data-diagram-node]") ?? [];
    for (const element of nodes) {
      if (element.getAttribute("data-diagram-node") === key) {
        element.focus();
        break;
      }
    }
  }

  const zoom = (factor: number) =>
    setScale((value) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, +(value * factor).toFixed(2))));

  return (
    <div className="utility-view diagram-view">
      <header className="utility-header">
        <div>
          <span className="eyebrow">SCHEMA DIAGRAM</span>
          <h1>{schema || "Schema"}</h1>
          <p>
            {diagram
              ? `${diagram.tables.length} relations · ${diagram.foreignKeys.length} foreign keys`
              : "Tables and their foreign-key relationships."}
          </p>
        </div>
        <div className="diagram-toolbar">
          <label className="diagram-filter">
            <Search size={13} />
            <input
              aria-label="Filter diagram tables"
              placeholder="Filter tables"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
            />
          </label>
          <button className="icon-button" aria-label="Zoom out" onClick={() => zoom(1 / 1.25)}>
            <Minus size={14} />
          </button>
          <span className="diagram-scale" aria-live="polite">
            {Math.round(scale * 100)}%
          </span>
          <button className="icon-button" aria-label="Zoom in" onClick={() => zoom(1.25)}>
            <Plus size={14} />
          </button>
          <button
            className="button ghost"
            disabled={loading || !session}
            onClick={() => void refresh()}
          >
            <RefreshCw size={14} className={loading ? "spin" : ""} />
            Refresh
          </button>
        </div>
      </header>
      {error && (
        <div className="inline-error" role="alert">
          {error}
        </div>
      )}
      {diagram?.truncated && (
        <div className="diagram-notice" role="status">
          This schema exceeds the diagram limits. Some tables, columns or relationships are not
          shown.
        </div>
      )}
      {diagram?.restricted && (
        <div className="diagram-notice" role="status">
          Some relationships are hidden because not all of their columns are readable.
        </div>
      )}
      {!session ? (
        <div className="utility-empty">
          <Network size={28} />
          <h3>This connection is offline</h3>
          <p>Reconnect to load the schema diagram.</p>
        </div>
      ) : layout && layout.nodes.length === 0 ? (
        <div className="utility-empty">
          <Network size={28} />
          <h3>{filter ? "No matching tables" : "No readable relations"}</h3>
          <p>{filter ? "Clear the filter to see every table." : "This schema has no tables."}</p>
        </div>
      ) : layout ? (
        <div className="diagram-canvas" ref={canvas}>
          <svg
            role="group"
            aria-label={`Relationship diagram for ${schema}`}
            width={layout.width * scale}
            height={layout.height * scale}
            viewBox={`0 0 ${layout.width} ${layout.height}`}
          >
            <defs>
              <marker
                id="diagram-arrow"
                viewBox="0 0 10 10"
                refX="9"
                refY="5"
                markerWidth="7"
                markerHeight="7"
                orient="auto-start-reverse"
              >
                <path d="M 0 0 L 10 5 L 0 10 z" className="diagram-arrowhead" />
              </marker>
            </defs>
            {layout.edges.map((edge) => {
              const fk = edge.foreignKey;
              const related = active === edge.source || active === edge.target;
              return (
                <path
                  key={edge.key}
                  d={edge.path}
                  className={`diagram-edge ${related ? "related" : ""}`}
                  markerEnd="url(#diagram-arrow)"
                  data-diagram-edge={fk.name}
                >
                  <title>
                    {`${fk.name}: ${fk.source.name}(${fk.columns.join(", ")}) → ${fk.target.schema}.${fk.target.name}(${fk.targetColumns.join(", ")})`}
                  </title>
                </path>
              );
            })}
            {layout.nodes.map((node) => {
              const label = node.external
                ? `${node.table.schema}.${node.table.name}, referenced table in another schema`
                : `${node.table.name}, ${node.columns.length + node.hiddenColumns} columns`;
              return (
                <g
                  key={node.key}
                  data-diagram-node={node.key}
                  role="button"
                  tabIndex={node.key === active ? 0 : -1}
                  aria-label={label}
                  className={`diagram-node ${node.external ? "external" : ""} ${node.key === active ? "active" : ""}`}
                  transform={`translate(${node.x} ${node.y})`}
                  onFocus={() => setFocused(node.key)}
                  onClick={() => {
                    setFocused(node.key);
                    if (!node.external) onOpenTable(node.table);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      if (!node.external) onOpenTable(node.table);
                      return;
                    }
                    if (
                      ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(
                        event.key,
                      )
                    ) {
                      event.preventDefault();
                      focusNode(neighborNode(layout, node.key, event.key as never));
                    }
                  }}
                >
                  <rect
                    className="diagram-node-box"
                    width={node.width}
                    height={node.height}
                    rx={6}
                  />
                  <rect
                    className="diagram-node-header"
                    width={node.width}
                    height={HEADER_HEIGHT}
                    rx={6}
                  />
                  <text className="diagram-node-title" x={10} y={HEADER_HEIGHT / 2 + 4}>
                    {node.external ? `${node.table.schema}.${node.table.name}` : node.table.name}
                  </text>
                  {node.external && (
                    <text className="diagram-node-muted" x={10} y={HEADER_HEIGHT + 14}>
                      Other schema
                    </text>
                  )}
                  {node.columns.map((column, index) => {
                    const y = HEADER_HEIGHT + index * ROW_HEIGHT + 14;
                    return (
                      <g key={column.name} className="diagram-column">
                        {column.primaryKey ? (
                          <KeyRound x={8} y={y - 10} size={11} className="diagram-pk" />
                        ) : node.foreignKeyColumns.has(column.name) ? (
                          <Link2 x={8} y={y - 10} size={11} className="diagram-fk" />
                        ) : null}
                        <text x={24} y={y} className={column.nullable ? "" : "required"}>
                          {column.name}
                        </text>
                        <text
                          x={node.width - 8}
                          y={y}
                          textAnchor="end"
                          className="diagram-node-muted"
                        >
                          {column.databaseType.length > 18
                            ? `${column.databaseType.slice(0, 17)}…`
                            : column.databaseType}
                        </text>
                      </g>
                    );
                  })}
                  {node.hiddenColumns > 0 && (
                    <text
                      className="diagram-node-muted"
                      x={24}
                      y={HEADER_HEIGHT + node.columns.length * ROW_HEIGHT + 14}
                    >
                      {`+${node.hiddenColumns} more`}
                    </text>
                  )}
                </g>
              );
            })}
          </svg>
        </div>
      ) : (
        <div className="utility-empty">
          <Network size={28} />
          <h3>{loading ? "Loading diagram…" : "Diagram unavailable"}</h3>
        </div>
      )}
    </div>
  );
}
