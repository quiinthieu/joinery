import { useState, useMemo, useCallback } from "react";
import {
  ChevronRight,
  ChevronDown,
  Copy,
  Check,
  Search,
  X,
  AlertCircle,
  Filter,
} from "lucide-react";
import { evaluateJsonPath } from "./jsonpath";
import { VariantPathHelper } from "./VariantPathHelper";
import { useStudio } from "../../state/studio";

export interface JsonTreeViewerProps {
  value: string | null;
  columnName?: string;
  isVariant?: boolean;
}

export function JsonTreeViewer({
  value,
  columnName = "value",
  isVariant = true,
}: JsonTreeViewerProps) {
  const [query, setQuery] = useState("");
  const [filterMode, setFilterMode] = useState(false);
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => new Set(["$"]));
  const [copiedKey, setCopiedKey] = useState<string | null>(null);
  const [selectedNode, setSelectedNode] = useState<{
    path: (string | number)[];
    value: unknown;
    valueType: string;
  } | null>(null);

  // Parse root value
  const parseResult = useMemo(() => {
    if (value === null || value === undefined) {
      return { ok: false, error: "Value is NULL", data: null };
    }
    try {
      return { ok: true, data: JSON.parse(value.trim()), error: null };
    } catch (err) {
      return {
        ok: false,
        error: err instanceof Error ? err.message : "Malformed JSON",
        data: null,
      };
    }
  }, [value]);

  // Initial selected node inference (finds first deep leaf node if available)
  useMemo(() => {
    if (!parseResult.ok || !parseResult.data) return;
    // Auto-select first deep leaf node so VariantPathHelper is immediately useful
    function findFirstLeaf(
      val: unknown,
      p: (string | number)[],
    ): { path: (string | number)[]; value: unknown; valueType: string } | null {
      if (val === null) return { path: p, value: null, valueType: "null" };
      if (typeof val !== "object") return { path: p, value: val, valueType: typeof val };
      if (Array.isArray(val)) {
        if (val.length === 0) return { path: p, value: val, valueType: "array" };
        return findFirstLeaf(val[0], [...p, 0]);
      }
      const keys = Object.keys(val as Record<string, unknown>);
      if (keys.length === 0) return { path: p, value: val, valueType: "object" };
      // Check if "store" or "book" or first key
      return findFirstLeaf((val as Record<string, unknown>)[keys[0]], [...p, keys[0]]);
    }

    const firstLeaf = findFirstLeaf(parseResult.data, []);
    if (firstLeaf && firstLeaf.path.length > 0) {
      setSelectedNode(firstLeaf);
    }
  }, [parseResult.data, parseResult.ok]);

  // Evaluate live JSONPath
  const jsonPathResult = useMemo(() => {
    if (!parseResult.ok || !query.trim()) return null;
    return evaluateJsonPath(query, parseResult.data);
  }, [parseResult, query]);

  // Set of exact matching normalized paths and ancestor paths
  const { exactMatchPaths, ancestorMatchPaths } = useMemo(() => {
    const exact = new Set<string>();
    const ancestors = new Set<string>();

    if (jsonPathResult && jsonPathResult.ok) {
      for (const m of jsonPathResult.matches) {
        exact.add(m.path);
        // Add all ancestor prefixes
        let p = "$";
        ancestors.add(p);
        for (const seg of m.normalizedPath) {
          p = typeof seg === "number" ? `${p}[${seg}]` : `${p}.${seg}`;
          ancestors.add(p);
        }
      }
    }
    return { exactMatchPaths: exact, ancestorMatchPaths: ancestors };
  }, [jsonPathResult]);

  // Auto-expand ancestors when JSONPath matches
  useMemo(() => {
    if (ancestorMatchPaths.size > 0) {
      setExpandedPaths((prev) => new Set([...prev, ...ancestorMatchPaths]));
    }
  }, [ancestorMatchPaths]);

  const toggleExpand = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const handleCopy = useCallback(async (text: string, label: string) => {
    await navigator.clipboard.writeText(text);
    setCopiedKey(label);
    setTimeout(() => setCopiedKey(null), 1500);
    useStudio.getState().notify(`Copied ${label}`);
  }, []);

  const handleSelectNode = useCallback((normalizedPath: (string | number)[], val: unknown) => {
    const valueType = val === null ? "null" : Array.isArray(val) ? "array" : typeof val;
    setSelectedNode({ path: normalizedPath, value: val, valueType });
  }, []);

  if (!parseResult.ok) {
    return (
      <div className="json-error-banner" role="alert">
        <AlertCircle size={14} />
        <span>{parseResult.error}</span>
      </div>
    );
  }

  return (
    <div className="json-tree-container" data-testid="json-tree-viewer">
      {/* Live JSONPath Toolbar */}
      <div className="jsonpath-bar">
        <div className="jsonpath-input-wrapper">
          <Search size={13} className="jsonpath-search-icon" />
          <input
            type="text"
            className={`jsonpath-input ${jsonPathResult && !jsonPathResult.ok ? "has-error" : ""}`}
            placeholder="JSONPath (e.g. $.store.book[*].title)"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label="JSONPath search"
            aria-invalid={jsonPathResult ? !jsonPathResult.ok : undefined}
          />
          {query && (
            <button
              type="button"
              className="icon-button jsonpath-clear"
              aria-label="Clear JSONPath"
              onClick={() => setQuery("")}
            >
              <X size={12} />
            </button>
          )}
        </div>
        <button
          type="button"
          className={`icon-button filter-toggle ${filterMode ? "active" : ""}`}
          title={filterMode ? "Showing matched subtrees only" : "Highlighting matches in full tree"}
          aria-label="Toggle subtree filter"
          aria-pressed={filterMode}
          onClick={() => setFilterMode((f) => !f)}
        >
          <Filter size={13} />
        </button>
      </div>

      {/* JSONPath Match Status */}
      {query.trim() && (
        <div className="jsonpath-status-bar">
          {jsonPathResult?.ok ? (
            <span className="jsonpath-match-count">
              {jsonPathResult.matches.length} match{jsonPathResult.matches.length === 1 ? "" : "es"}
            </span>
          ) : (
            <span className="jsonpath-error-label" role="alert">
              <AlertCircle size={11} />
              {jsonPathResult?.error || "Invalid syntax"}
            </span>
          )}
        </div>
      )}

      {/* Tree Expand/Collapse Controls */}
      <div className="json-tree-actions">
        <button
          type="button"
          className="button ghost small"
          onClick={() => {
            const all = new Set<string>(["$"]);
            function collect(val: unknown, p: string, depth: number) {
              if (depth > 4 || val === null || typeof val !== "object") return;
              all.add(p);
              if (Array.isArray(val)) {
                val.forEach((v, i) => collect(v, `${p}[${i}]`, depth + 1));
              } else {
                Object.entries(val as Record<string, unknown>).forEach(([k, v]) =>
                  collect(v, `${p}.${k}`, depth + 1),
                );
              }
            }
            collect(parseResult.data, "$", 0);
            setExpandedPaths(all);
          }}
        >
          Expand All
        </button>
        <button
          type="button"
          className="button ghost small"
          onClick={() => setExpandedPaths(new Set(["$"]))}
        >
          Collapse All
        </button>
      </div>

      {/* Interactive Tree View */}
      <div className="json-tree" role="tree" tabIndex={0}>
        <JsonTreeNode
          keyName={columnName}
          value={parseResult.data}
          path="$"
          normalizedPath={[]}
          depth={0}
          expandedPaths={expandedPaths}
          exactMatchPaths={exactMatchPaths}
          ancestorMatchPaths={ancestorMatchPaths}
          filterMode={filterMode}
          hasQuery={Boolean(query.trim() && jsonPathResult?.ok)}
          onToggle={toggleExpand}
          onCopy={handleCopy}
          onSelect={handleSelectNode}
          selectedPath={selectedNode?.path ?? []}
          copiedKey={copiedKey}
        />
      </div>

      {/* Snowflake Semi-Structured Variant Path Helper */}
      {isVariant && (
        <VariantPathHelper
          columnName={columnName}
          selectedPath={selectedNode?.path ?? []}
          valueType={selectedNode?.valueType}
          sampleValue={selectedNode?.value}
        />
      )}
    </div>
  );
}

interface JsonTreeNodeProps {
  keyName: string;
  value: unknown;
  path: string;
  normalizedPath: (string | number)[];
  depth: number;
  expandedPaths: Set<string>;
  exactMatchPaths: Set<string>;
  ancestorMatchPaths: Set<string>;
  filterMode: boolean;
  hasQuery: boolean;
  onToggle(path: string): void;
  onCopy(text: string, label: string): void;
  onSelect(normalizedPath: (string | number)[], value: unknown): void;
  selectedPath: (string | number)[];
  copiedKey: string | null;
}

function JsonTreeNode({
  keyName,
  value,
  path,
  normalizedPath,
  depth,
  expandedPaths,
  exactMatchPaths,
  ancestorMatchPaths,
  filterMode,
  hasQuery,
  onToggle,
  onCopy,
  onSelect,
  selectedPath,
  copiedKey,
}: JsonTreeNodeProps) {
  const isMatch = exactMatchPaths.has(path);
  const isAncestor = ancestorMatchPaths.has(path);

  if (filterMode && hasQuery && !isMatch && !isAncestor) {
    return null;
  }

  const isExpanded = expandedPaths.has(path);
  const isArray = Array.isArray(value);
  const isObject = value !== null && typeof value === "object" && !isArray;
  const isComposite = isArray || isObject;

  const isSelected =
    selectedPath.length === normalizedPath.length &&
    selectedPath.every((seg, idx) => seg === normalizedPath[idx]);

  const countBadge = isArray
    ? `[${value.length} element${value.length === 1 ? "" : "s"}]`
    : isObject
      ? `{${Object.keys(value as Record<string, unknown>).length} item${
          Object.keys(value as Record<string, unknown>).length === 1 ? "" : "s"
        }}`
      : null;

  return (
    <div
      className={`json-node ${isMatch ? "json-node-match" : ""} ${isSelected ? "json-node-selected" : ""}`}
      style={{ paddingLeft: `${depth * 14}px` }}
      role="treeitem"
      aria-expanded={isComposite ? isExpanded : undefined}
    >
      <div className="json-node-row" onClick={() => onSelect(normalizedPath, value)}>
        {isComposite ? (
          <button
            type="button"
            className="json-node-toggle"
            onClick={(e) => {
              e.stopPropagation();
              onToggle(path);
            }}
            aria-label={isExpanded ? "Collapse branch" : "Expand branch"}
          >
            {isExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          </button>
        ) : (
          <span className="json-node-spacer" />
        )}

        <span className="json-key">
          {keyName.startsWith("[") && keyName.endsWith("]") ? `${keyName}:` : `"${keyName}":`}
        </span>

        {isComposite ? (
          <span
            className="json-badge"
            onClick={(e) => {
              e.stopPropagation();
              onToggle(path);
            }}
          >
            {countBadge}
          </span>
        ) : (
          <JsonPrimitiveValue value={value} />
        )}

        <div className="json-node-hover-actions">
          <button
            type="button"
            className="icon-button micro"
            title="Copy path"
            aria-label={`Copy path ${path}`}
            onClick={(e) => {
              e.stopPropagation();
              onCopy(path, "path");
            }}
          >
            {copiedKey === "path" ? <Check size={10} /> : <Copy size={10} />}
          </button>
          <button
            type="button"
            className="icon-button micro"
            title="Copy value"
            aria-label="Copy value"
            onClick={(e) => {
              e.stopPropagation();
              onCopy(typeof value === "string" ? value : JSON.stringify(value, null, 2), "value");
            }}
          >
            <Copy size={10} />
          </button>
        </div>
      </div>

      {isComposite && isExpanded && (
        <div className="json-node-children" role="group">
          {isArray
            ? value.map((elem, idx) => (
                <JsonTreeNode
                  key={idx}
                  keyName={`[${idx}]`}
                  value={elem}
                  path={`${path}[${idx}]`}
                  normalizedPath={[...normalizedPath, idx]}
                  depth={depth + 1}
                  expandedPaths={expandedPaths}
                  exactMatchPaths={exactMatchPaths}
                  ancestorMatchPaths={ancestorMatchPaths}
                  filterMode={filterMode}
                  hasQuery={hasQuery}
                  onToggle={onToggle}
                  onCopy={onCopy}
                  onSelect={onSelect}
                  selectedPath={selectedPath}
                  copiedKey={copiedKey}
                />
              ))
            : Object.entries(value as Record<string, unknown>).map(([k, v]) => (
                <JsonTreeNode
                  key={k}
                  keyName={k}
                  value={v}
                  path={`${path}.${k}`}
                  normalizedPath={[...normalizedPath, k]}
                  depth={depth + 1}
                  expandedPaths={expandedPaths}
                  exactMatchPaths={exactMatchPaths}
                  ancestorMatchPaths={ancestorMatchPaths}
                  filterMode={filterMode}
                  hasQuery={hasQuery}
                  onToggle={onToggle}
                  onCopy={onCopy}
                  onSelect={onSelect}
                  selectedPath={selectedPath}
                  copiedKey={copiedKey}
                />
              ))}
        </div>
      )}
    </div>
  );
}

function JsonPrimitiveValue({ value }: { value: unknown }) {
  if (value === null) {
    return <span className="json-val json-null">null</span>;
  }
  if (typeof value === "string") {
    return <span className="json-val json-string">"{value}"</span>;
  }
  if (typeof value === "number") {
    return <span className="json-val json-number">{value}</span>;
  }
  if (typeof value === "boolean") {
    return <span className="json-val json-boolean">{String(value)}</span>;
  }
  return <span className="json-val">{String(value)}</span>;
}
