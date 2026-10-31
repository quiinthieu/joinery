import type {
  DatabaseObjectKind,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  TableInfo,
} from "../../ipc/generated";

export type ExplorerCategoryId =
  "tables" | "views" | "materializedViews" | "routines" | "sequences" | "triggers" | "indexes";

export type ExplorerFilterChip = "all" | "tables" | "views" | "routines" | "other";

export const EXPLORER_FILTER_CHIPS: readonly {
  id: ExplorerFilterChip;
  label: string;
}[] = [
  { id: "all", label: "All" },
  { id: "tables", label: "Tables" },
  { id: "views", label: "Views" },
  { id: "routines", label: "Routines" },
  { id: "other", label: "Other" },
];

export const EXPLORER_CATEGORY_ORDER: readonly ExplorerCategoryId[] = [
  "tables",
  "views",
  "materializedViews",
  "routines",
  "sequences",
  "triggers",
  "indexes",
];

const CATEGORY_LABELS: Record<ExplorerCategoryId, string> = {
  tables: "Tables",
  views: "Views",
  materializedViews: "Materialized views",
  routines: "Routines",
  sequences: "Sequences",
  triggers: "Triggers",
  indexes: "Indexes",
};

export interface ExplorerItem {
  key: string;
  object: DatabaseObjectRef;
  category: ExplorerCategoryId;
  primaryLabel: string;
  secondaryLabel: string | null;
  tooltip: string;
  ownerTable: string | null;
  signature: string | null;
  estimatedRows: number | null;
  totalBytes: number | null;
}

export interface ExplorerCategoryGroup {
  id: ExplorerCategoryId;
  key: string;
  label: string;
  items: ExplorerItem[];
}

export interface ExplorerSchemaGroup {
  schema: string;
  totalCount: number;
  categories: ExplorerCategoryGroup[];
}

export type FlatExplorerNode =
  | {
      id: string;
      kind: "schema";
      schema: string;
      expanded: boolean;
      parentId: null;
    }
  | {
      id: string;
      kind: "category";
      schema: string;
      category: ExplorerCategoryId;
      categoryKey: string;
      expanded: boolean;
      parentId: string;
    }
  | {
      id: string;
      kind: "item";
      schema: string;
      category: ExplorerCategoryId;
      item: ExplorerItem;
      parentId: string;
    };

export type TreeKeyAction =
  | { type: "focus"; nodeId: string }
  | { type: "expandSchema"; schema: string }
  | { type: "collapseSchema"; schema: string }
  | { type: "expandCategory"; categoryKey: string }
  | { type: "collapseCategory"; categoryKey: string };

export function categoryLabel(category: ExplorerCategoryId): string {
  return CATEGORY_LABELS[category];
}

export function isTabularObjectKind(kind: DatabaseObjectKind): boolean {
  return (
    kind === "table" ||
    kind === "partitionedTable" ||
    kind === "view" ||
    kind === "materializedView"
  );
}

export function sameDatabaseObject(
  left: DatabaseObjectRef | null | undefined,
  right: DatabaseObjectRef | null | undefined,
): boolean {
  if (!left || !right) return false;
  return (
    left.schema === right.schema &&
    left.name === right.name &&
    left.kind === right.kind &&
    (left.identity ?? null) === (right.identity ?? null)
  );
}

export function databaseObjectKey(object: DatabaseObjectRef): string {
  return `${object.schema}:${object.kind}:${object.name}:${object.identity ?? ""}`;
}

export function objectTabTitle(object: DatabaseObjectRef): string {
  if (
    (object.kind === "function" || object.kind === "procedure") &&
    object.identity !== null &&
    object.identity !== undefined
  ) {
    return `${object.name}(${object.identity})`;
  }
  return object.name;
}

export function tableInfoKindToObjectKind(rawKind: string): DatabaseObjectKind {
  const normalized = rawKind.trim().toUpperCase();
  if (normalized === "P" || normalized === "PARTITIONED TABLE") return "partitionedTable";
  if (normalized === "V" || normalized === "VIEW") return "view";
  if (normalized === "M" || normalized === "MATERIALIZED VIEW") return "materializedView";
  if (normalized === "S" || normalized === "SEQUENCE") return "sequence";
  return "table";
}

export function objectKindCategory(kind: DatabaseObjectKind): ExplorerCategoryId {
  switch (kind) {
    case "table":
    case "partitionedTable":
      return "tables";
    case "view":
      return "views";
    case "materializedView":
      return "materializedViews";
    case "function":
    case "procedure":
      return "routines";
    case "sequence":
      return "sequences";
    case "trigger":
      return "triggers";
    case "index":
      return "indexes";
  }
}

export function categoryMatchesFilter(
  category: ExplorerCategoryId,
  filter: ExplorerFilterChip,
): boolean {
  switch (filter) {
    case "all":
      return true;
    case "tables":
      return category === "tables";
    case "views":
      return category === "views" || category === "materializedViews";
    case "routines":
      return category === "routines";
    case "other":
      return category === "sequences" || category === "triggers" || category === "indexes";
  }
}

function extractSignatureArgs(signature: string | null): string | null {
  if (!signature) return null;
  const trimmed = signature.trim();
  if (!trimmed.startsWith("(")) return null;
  let depth = 0;
  for (let i = 0; i < trimmed.length; i += 1) {
    if (trimmed[i] === "(") depth += 1;
    else if (trimmed[i] === ")") {
      depth -= 1;
      if (depth === 0) return trimmed.slice(0, i + 1);
    }
  }
  return null;
}

export function formatRoutinePrimaryLabel(
  object: DatabaseObjectRef,
  signature: string | null,
): string {
  if (object.identity !== null && object.identity !== undefined) {
    return `${object.name}(${object.identity})`;
  }
  const args = extractSignatureArgs(signature);
  return args ? `${object.name}${args}` : object.name;
}

export function buildExplorerSchemas(
  tables: readonly TableInfo[],
  objects: readonly DatabaseObjectSummary[],
  filter: ExplorerFilterChip,
  search: string,
): ExplorerSchemaGroup[] {
  const seenKeys = new Set<string>();
  const rawItems: Omit<ExplorerItem, "primaryLabel" | "secondaryLabel" | "tooltip">[] = [];

  for (const entry of tables) {
    const kind = tableInfoKindToObjectKind(entry.kind);
    const object: DatabaseObjectRef = {
      schema: entry.table.schema,
      name: entry.table.name,
      kind,
      identity: null,
    };
    const key = databaseObjectKey(object);
    if (seenKeys.has(key)) continue;
    seenKeys.add(key);
    rawItems.push({
      key,
      object,
      category: objectKindCategory(kind),
      ownerTable: null,
      signature: null,
      estimatedRows: entry.estimatedRows,
      totalBytes: entry.totalBytes,
    });
  }

  for (const summary of objects) {
    const object: DatabaseObjectRef = {
      schema: summary.object.schema,
      name: summary.object.name,
      kind: summary.object.kind,
      identity: summary.object.identity ?? null,
    };
    const key = databaseObjectKey(object);
    if (seenKeys.has(key)) continue;
    seenKeys.add(key);
    rawItems.push({
      key,
      object,
      category: objectKindCategory(object.kind),
      ownerTable: summary.ownerTable,
      signature: summary.signature,
      estimatedRows: summary.estimatedRows,
      totalBytes: summary.totalBytes,
    });
  }

  // Count duplicate names within (schema, category) for table-scoped triggers/indexes
  const nameCounts = new Map<string, number>();
  for (const item of rawItems) {
    if (item.category === "triggers" || item.category === "indexes") {
      const bucket = `${item.object.schema}\0${item.category}\0${item.object.name}`;
      nameCounts.set(bucket, (nameCounts.get(bucket) ?? 0) + 1);
    }
  }

  const needle = search.trim().toLowerCase();
  const bySchema = new Map<string, Map<ExplorerCategoryId, ExplorerItem[]>>();

  for (const raw of rawItems) {
    if (!categoryMatchesFilter(raw.category, filter)) continue;

    let primaryLabel = raw.object.name;
    let secondaryLabel: string | null = null;
    if (raw.object.kind === "function" || raw.object.kind === "procedure") {
      primaryLabel = formatRoutinePrimaryLabel(raw.object, raw.signature);
      secondaryLabel = raw.object.kind === "procedure" ? "proc" : null;
    } else if (raw.object.kind === "partitionedTable") {
      secondaryLabel = "part";
    } else if (raw.category === "triggers" || raw.category === "indexes") {
      const bucket = `${raw.object.schema}\0${raw.category}\0${raw.object.name}`;
      const duplicated = (nameCounts.get(bucket) ?? 0) > 1;
      if (duplicated && raw.ownerTable) {
        primaryLabel = `${raw.object.name} (${raw.ownerTable})`;
      } else {
        secondaryLabel = raw.ownerTable;
      }
    }

    const tooltipParts = [`${raw.object.schema}.${primaryLabel}`];
    if (raw.ownerTable && !primaryLabel.includes(`(${raw.ownerTable})`)) {
      tooltipParts.push(`on ${raw.ownerTable}`);
    }
    if (raw.signature) tooltipParts.push(raw.signature);
    const tooltip = tooltipParts.join(" · ");

    if (needle) {
      const haystack = [
        `${raw.object.schema}.${raw.object.name}`,
        primaryLabel,
        raw.ownerTable ?? "",
        raw.signature ?? "",
      ]
        .join(" ")
        .toLowerCase();
      if (!haystack.includes(needle)) continue;
    }

    let catMap = bySchema.get(raw.object.schema);
    if (!catMap) {
      catMap = new Map();
      bySchema.set(raw.object.schema, catMap);
    }
    const list = catMap.get(raw.category) ?? [];
    list.push({
      ...raw,
      primaryLabel,
      secondaryLabel,
      tooltip,
    });
    catMap.set(raw.category, list);
  }

  const schemas: ExplorerSchemaGroup[] = [];
  for (const [schema, catMap] of bySchema) {
    const categories: ExplorerCategoryGroup[] = [];
    let totalCount = 0;
    for (const catId of EXPLORER_CATEGORY_ORDER) {
      const items = catMap.get(catId);
      if (!items?.length) continue;
      totalCount += items.length;
      categories.push({
        id: catId,
        key: `${schema}:${catId}`,
        label: CATEGORY_LABELS[catId],
        items,
      });
    }
    if (categories.length > 0) {
      schemas.push({ schema, totalCount, categories });
    }
  }
  return schemas;
}

export function flattenVisibleExplorerNodes(
  schemas: readonly ExplorerSchemaGroup[],
  isSchemaExpanded: (schema: string) => boolean,
  isCategoryExpanded: (categoryKey: string) => boolean,
  itemLimit = 200,
): FlatExplorerNode[] {
  const nodes: FlatExplorerNode[] = [];
  for (const group of schemas) {
    const schemaId = `schema:${group.schema}`;
    const schemaOpen = isSchemaExpanded(group.schema);
    nodes.push({
      id: schemaId,
      kind: "schema",
      schema: group.schema,
      expanded: schemaOpen,
      parentId: null,
    });
    if (!schemaOpen) continue;
    for (const cat of group.categories) {
      const catId = `cat:${cat.key}`;
      const catOpen = isCategoryExpanded(cat.key);
      nodes.push({
        id: catId,
        kind: "category",
        schema: group.schema,
        category: cat.id,
        categoryKey: cat.key,
        expanded: catOpen,
        parentId: schemaId,
      });
      if (!catOpen) continue;
      for (const item of cat.items.slice(0, itemLimit)) {
        nodes.push({
          id: `item:${item.key}`,
          kind: "item",
          schema: group.schema,
          category: cat.id,
          item,
          parentId: catId,
        });
      }
    }
  }
  return nodes;
}

export function resolveTreeKeyAction(
  nodes: readonly FlatExplorerNode[],
  currentNodeId: string,
  key: string,
): TreeKeyAction | null {
  if (!nodes.length) return null;
  const index = nodes.findIndex((n) => n.id === currentNodeId);
  if (index < 0) return null;
  const current = nodes[index];

  switch (key) {
    case "ArrowDown": {
      const next = nodes[Math.min(nodes.length - 1, index + 1)];
      return next && next.id !== current.id ? { type: "focus", nodeId: next.id } : null;
    }
    case "ArrowUp": {
      const prev = nodes[Math.max(0, index - 1)];
      return prev && prev.id !== current.id ? { type: "focus", nodeId: prev.id } : null;
    }
    case "Home": {
      const first = nodes[0];
      return first.id !== current.id ? { type: "focus", nodeId: first.id } : null;
    }
    case "End": {
      const last = nodes[nodes.length - 1];
      return last.id !== current.id ? { type: "focus", nodeId: last.id } : null;
    }
    case "ArrowRight": {
      if (current.kind === "schema") {
        if (!current.expanded) return { type: "expandSchema", schema: current.schema };
        const child = nodes[index + 1];
        return child?.parentId === current.id ? { type: "focus", nodeId: child.id } : null;
      }
      if (current.kind === "category") {
        if (!current.expanded) {
          return { type: "expandCategory", categoryKey: current.categoryKey };
        }
        const child = nodes[index + 1];
        return child?.parentId === current.id ? { type: "focus", nodeId: child.id } : null;
      }
      return null;
    }
    case "ArrowLeft": {
      if (current.kind === "item") {
        return { type: "focus", nodeId: current.parentId };
      }
      if (current.kind === "category") {
        if (current.expanded) {
          return { type: "collapseCategory", categoryKey: current.categoryKey };
        }
        return { type: "focus", nodeId: current.parentId };
      }
      if (current.kind === "schema" && current.expanded) {
        return { type: "collapseSchema", schema: current.schema };
      }
      return null;
    }
    default:
      return null;
  }
}
