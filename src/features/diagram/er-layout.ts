import type { DiagramForeignKey, DiagramTable, SchemaDiagram, TableRef } from "../../ipc/generated";

export const NODE_WIDTH = 232;
export const HEADER_HEIGHT = 30;
export const ROW_HEIGHT = 20;
export const MAX_VISIBLE_COLUMNS = 24;
const LAYER_GAP = 96;
const NODE_GAP = 28;
const MARGIN = 32;

export interface DiagramNode {
  key: string;
  table: TableRef;
  kind: string;
  /** Targets in another schema have no column metadata in this diagram. */
  external: boolean;
  columns: DiagramTable["columns"];
  /** Column names that participate in an outbound foreign key. */
  foreignKeyColumns: Set<string>;
  hiddenColumns: number;
  layer: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface DiagramEdge {
  key: string;
  foreignKey: DiagramForeignKey;
  source: string;
  target: string;
  path: string;
  selfReference: boolean;
}

export interface DiagramLayout {
  nodes: DiagramNode[];
  edges: DiagramEdge[];
  width: number;
  height: number;
  /** Node keys per layer, top to bottom, for keyboard traversal. */
  layers: string[][];
}

export const tableKey = (table: TableRef) => JSON.stringify([table.schema, table.name]);

const byName = (left: TableRef, right: TableRef) =>
  left.schema === right.schema
    ? left.name < right.name
      ? -1
      : left.name > right.name
        ? 1
        : 0
    : left.schema < right.schema
      ? -1
      : 1;

/**
 * Keeps tables whose qualified name contains `filter`, plus the tables they
 * reference or are referenced by. An empty filter keeps everything.
 */
export function filterDiagram(diagram: SchemaDiagram, filter: string): SchemaDiagram {
  const needle = filter.trim().toLocaleLowerCase();
  if (!needle) return diagram;
  const matches = new Set(
    diagram.tables
      .filter((t) => `${t.table.schema}.${t.table.name}`.toLocaleLowerCase().includes(needle))
      .map((t) => tableKey(t.table)),
  );
  const keep = new Set(matches);
  for (const fk of diagram.foreignKeys) {
    const source = tableKey(fk.source);
    const target = tableKey(fk.target);
    if (matches.has(source)) keep.add(target);
    if (matches.has(target)) keep.add(source);
  }
  return {
    ...diagram,
    tables: diagram.tables.filter((t) => keep.has(tableKey(t.table))),
    foreignKeys: diagram.foreignKeys.filter(
      (fk) => keep.has(tableKey(fk.source)) && keep.has(tableKey(fk.target)),
    ),
  };
}

/**
 * Deterministic layered layout. Referenced tables sit to the left of the
 * tables that reference them. Cycles are broken by name order, and
 * self-references do not affect layers.
 */
export function layoutDiagram(diagram: SchemaDiagram): DiagramLayout {
  const nodes = new Map<string, DiagramNode>();
  const tables = [...diagram.tables].sort((l, r) => byName(l.table, r.table));
  for (const table of tables) {
    const key = tableKey(table.table);
    nodes.set(key, node(key, table.table, table.kind, false, table.columns));
  }
  const keys = [...diagram.foreignKeys].sort(
    (l, r) => byName(l.source, r.source) || (l.name < r.name ? -1 : l.name > r.name ? 1 : 0),
  );
  for (const fk of keys) {
    const target = tableKey(fk.target);
    if (!nodes.has(target)) nodes.set(target, node(target, fk.target, "external", true, []));
    const source = nodes.get(tableKey(fk.source));
    if (source) for (const column of fk.columns) source.foreignKeyColumns.add(column);
  }

  // Outbound targets per source, excluding self-references.
  const targets = new Map<string, string[]>();
  for (const fk of keys) {
    const source = tableKey(fk.source);
    const target = tableKey(fk.target);
    if (source === target || !nodes.has(source)) continue;
    const list = targets.get(source) ?? [];
    if (!list.includes(target)) list.push(target);
    targets.set(source, list);
  }

  // Break cycles: DFS in name order, dropping edges back to the active stack.
  const ordered = [...nodes.keys()].sort((l, r) =>
    byName(nodes.get(l)!.table, nodes.get(r)!.table),
  );
  const state = new Map<string, 1 | 2>();
  const acyclic = new Map<string, string[]>();
  const visit = (start: string) => {
    const stack: { key: string; next: number }[] = [{ key: start, next: 0 }];
    state.set(start, 1);
    while (stack.length) {
      const frame = stack[stack.length - 1];
      const out = targets.get(frame.key) ?? [];
      if (frame.next >= out.length) {
        state.set(frame.key, 2);
        stack.pop();
        continue;
      }
      const target = out[frame.next++];
      if (state.get(target) === 1) continue;
      const kept = acyclic.get(frame.key) ?? [];
      kept.push(target);
      acyclic.set(frame.key, kept);
      if (!state.has(target)) {
        state.set(target, 1);
        stack.push({ key: target, next: 0 });
      }
    }
  };
  for (const key of ordered) if (!state.has(key)) visit(key);

  // Longest path to a sink: referenced tables get lower layers.
  const layerOf = new Map<string, number>();
  const depth = (start: string): number => {
    const stack: { key: string; next: number; best: number }[] = [{ key: start, next: 0, best: 0 }];
    while (stack.length) {
      const frame = stack[stack.length - 1];
      if (layerOf.has(frame.key)) {
        stack.pop();
        continue;
      }
      const out = acyclic.get(frame.key) ?? [];
      if (frame.next < out.length) {
        const target = out[frame.next++];
        if (layerOf.has(target)) frame.best = Math.max(frame.best, layerOf.get(target)! + 1);
        else {
          frame.next--;
          stack.push({ key: target, next: 0, best: 0 });
        }
        continue;
      }
      layerOf.set(frame.key, frame.best);
      stack.pop();
    }
    return layerOf.get(start)!;
  };
  for (const key of ordered) nodes.get(key)!.layer = depth(key);

  const layerCount = Math.max(0, ...[...nodes.values()].map((n) => n.layer + 1));
  const layers: string[][] = Array.from({ length: layerCount }, () => []);
  for (const key of ordered) layers[nodes.get(key)!.layer].push(key);

  // One barycenter pass against the previous layer keeps related tables close.
  const neighbors = new Map<string, string[]>();
  for (const [source, outs] of targets) {
    for (const target of outs) {
      neighbors.set(source, [...(neighbors.get(source) ?? []), target]);
      neighbors.set(target, [...(neighbors.get(target) ?? []), source]);
    }
  }
  for (let i = 1; i < layers.length; i++) {
    const position = new Map(layers[i - 1].map((key, index) => [key, index]));
    const score = (key: string) => {
      const near = (neighbors.get(key) ?? []).filter((n) => position.has(n));
      return near.length
        ? near.reduce((sum, n) => sum + position.get(n)!, 0) / near.length
        : Number.POSITIVE_INFINITY;
    };
    layers[i] = layers[i]
      .map((key, index) => ({ key, index, score: score(key) }))
      .sort((l, r) => l.score - r.score || l.index - r.index)
      .map((entry) => entry.key);
  }

  let height = 0;
  layers.forEach((layer, layerIndex) => {
    let y = MARGIN;
    for (const key of layer) {
      const n = nodes.get(key)!;
      n.x = MARGIN + layerIndex * (NODE_WIDTH + LAYER_GAP);
      n.y = y;
      y += n.height + NODE_GAP;
    }
    height = Math.max(height, y - NODE_GAP + MARGIN);
  });
  const width = MARGIN * 2 + Math.max(0, layerCount * (NODE_WIDTH + LAYER_GAP) - LAYER_GAP);

  const edges: DiagramEdge[] = [];
  for (const fk of keys) {
    const source = nodes.get(tableKey(fk.source));
    const target = nodes.get(tableKey(fk.target));
    if (!source || !target) continue;
    edges.push({
      key: `${source.key}\0${fk.name}`,
      foreignKey: fk,
      source: source.key,
      target: target.key,
      path: edgePath(source, fk.columns[0], target, fk.targetColumns[0]),
      selfReference: source.key === target.key,
    });
  }
  return {
    nodes: ordered.map((key) => nodes.get(key)!),
    edges,
    width: Math.max(width, MARGIN * 2),
    height: Math.max(height, MARGIN * 2),
    layers,
  };
}

function node(
  key: string,
  table: TableRef,
  kind: string,
  external: boolean,
  columns: DiagramTable["columns"],
): DiagramNode {
  const shown = columns.slice(0, MAX_VISIBLE_COLUMNS);
  const hidden = columns.length - shown.length;
  const rows = shown.length + (hidden > 0 ? 1 : 0);
  return {
    key,
    table,
    kind,
    external,
    columns: shown,
    foreignKeyColumns: new Set(),
    hiddenColumns: hidden,
    layer: 0,
    x: 0,
    y: 0,
    width: NODE_WIDTH,
    height: HEADER_HEIGHT + Math.max(1, rows) * ROW_HEIGHT + 6,
  };
}

/** Vertical anchor for a column row, or the header when the row is hidden. */
export function columnAnchor(node: DiagramNode, column: string | undefined): number {
  const index = column === undefined ? -1 : node.columns.findIndex((c) => c.name === column);
  return index < 0
    ? node.y + HEADER_HEIGHT / 2
    : node.y + HEADER_HEIGHT + index * ROW_HEIGHT + ROW_HEIGHT / 2;
}

function edgePath(
  source: DiagramNode,
  sourceColumn: string | undefined,
  target: DiagramNode,
  targetColumn: string | undefined,
): string {
  const sy = columnAnchor(source, sourceColumn);
  const ty = columnAnchor(target, targetColumn);
  if (source.key === target.key) {
    const x = source.x + source.width;
    return `M ${x} ${sy} C ${x + 48} ${sy}, ${x + 48} ${ty}, ${x} ${ty}`;
  }
  const leftward = target.x < source.x;
  const sx = leftward ? source.x : source.x + source.width;
  const tx = leftward ? target.x + target.width : target.x;
  if (source.x === target.x) {
    // Same layer after a broken cycle: loop around the right side.
    const x = source.x + source.width;
    const bend = x + 40;
    return `M ${x} ${sy} C ${bend} ${sy}, ${bend} ${ty}, ${x} ${ty}`;
  }
  const bend = Math.max(32, Math.abs(tx - sx) / 2);
  const c1 = leftward ? sx - bend : sx + bend;
  const c2 = leftward ? tx + bend : tx - bend;
  return `M ${sx} ${sy} C ${c1} ${sy}, ${c2} ${ty}, ${tx} ${ty}`;
}
