import { useState, useMemo, useCallback } from "react";
import { Copy, Check, Code } from "lucide-react";
import {
  generateSnowflakePath,
  inferDefaultSnowflakeCast,
  SNOWFLAKE_CAST_TYPES,
  type SnowflakeCastType,
} from "./variant-path";
import { useStudio } from "../../state/studio";

export interface VariantPathHelperProps {
  columnName: string;
  selectedPath: (string | number)[];
  valueType?: string;
  sampleValue?: unknown;
}

export function VariantPathHelper({
  columnName,
  selectedPath,
  valueType,
  sampleValue,
}: VariantPathHelperProps) {
  const [cast, setCast] = useState<SnowflakeCastType | "auto">("auto");
  const [style, setStyle] = useState<"colon" | "bracket">("colon");
  const [copied, setCopied] = useState(false);

  const defaultCast = useMemo(
    () => inferDefaultSnowflakeCast(valueType, sampleValue),
    [valueType, sampleValue],
  );

  const sqlExpression = useMemo(() => {
    return generateSnowflakePath({
      columnName,
      path: selectedPath,
      valueType,
      cast,
      style,
    });
  }, [columnName, selectedPath, valueType, cast, style]);

  const handleCopy = useCallback(async () => {
    await navigator.clipboard.writeText(sqlExpression);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
    useStudio.getState().notify("Copied Snowflake SQL expression");
  }, [sqlExpression]);

  return (
    <div className="variant-path-helper" data-testid="variant-path-helper">
      <div className="variant-path-header">
        <span className="variant-path-title">
          <Code size={11} />
          Snowflake SQL Path
        </span>
        <div className="variant-path-controls">
          <select
            className="variant-cast-select"
            value={cast}
            onChange={(e) => setCast(e.target.value as SnowflakeCastType | "auto")}
            aria-label="Select cast type"
            title="Cast return type"
          >
            <option value="auto">Auto (::{defaultCast})</option>
            {SNOWFLAKE_CAST_TYPES.map((t) => (
              <option key={t.id} value={t.id}>
                {t.label}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="icon-button micro"
            title={style === "colon" ? "Switch to bracket syntax" : "Switch to colon syntax"}
            aria-label="Toggle path syntax style"
            onClick={() => setStyle((s) => (s === "colon" ? "bracket" : "colon"))}
          >
            {style === "colon" ? ":" : "[]"}
          </button>
        </div>
      </div>

      <div className="variant-path-row">
        <code
          className="snowflake-sql-expression"
          data-testid="variant-sql-path"
          title={sqlExpression}
        >
          {sqlExpression}
        </code>
        <button
          type="button"
          className="button ghost small variant-copy-btn"
          aria-label="Copy SQL expression"
          title="Copy SQL Expression"
          onClick={handleCopy}
        >
          {copied ? <Check size={11} /> : <Copy size={11} />}
          <span>{copied ? "Copied" : "Copy SQL"}</span>
        </button>
      </div>
    </div>
  );
}
