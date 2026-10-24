import { useEffect, useRef, useState } from "react";
import { client, messageOf } from "../../ipc/client";
import type { RedisScanPage, RedisValuePreview } from "../../ipc/generated";
import { redisEditCommand } from "./commands";

export function RedisTools({
  sessionId,
  disabled,
  readOnly,
  onCommand,
}: {
  sessionId: string | null;
  disabled: boolean;
  readOnly: boolean;
  onCommand(command: string): void;
}) {
  const [pattern, setPattern] = useState("*");
  const [page, setPage] = useState<RedisScanPage | null>(null);
  const [argument, setArgument] = useState('""');
  const [preview, setPreview] = useState<RedisValuePreview | null>(null);
  const [operation, setOperation] = useState("SET");
  const [field, setField] = useState("");
  const [value, setValue] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  useEffect(() => {
    generation.current++;
    setPage(null);
    setPreview(null);
    setError("");
    setBusy(false);
    return () => {
      generation.current++;
    };
  }, [sessionId]);

  async function scan(cursor: string | null = null) {
    if (!sessionId || busy || disabled) return;
    const current = ++generation.current;
    setBusy(true);
    setError("");
    try {
      const result = await client.call<RedisScanPage>("redis_scan", { sessionId, pattern, cursor });
      if (generation.current === current) setPage(result);
    } catch (error) {
      if (generation.current === current) setError(messageOf(error));
    } finally {
      if (generation.current === current) setBusy(false);
    }
  }
  async function inspect(key = argument, cursor: string | null = null) {
    if (!sessionId || busy || disabled) return;
    const current = ++generation.current;
    setBusy(true);
    setError("");
    setArgument(key);
    try {
      const result = await client.call<RedisValuePreview>("redis_inspect", {
        sessionId,
        argument: key,
        cursor,
      });
      if (generation.current !== current) return;
      if (preview?.key.argument !== result.key.argument) {
        setField("");
        setValue("");
      }
      setPreview(result);
      setOperation(
        (
          { string: "SET", hash: "HSET", list: "LSET", set: "SADD", zset: "ZADD" } as Record<
            string,
            string
          >
        )[result.key.kind] ?? "UNLINK",
      );
    } catch (error) {
      if (generation.current === current) {
        setPreview(null);
        setError(messageOf(error));
      }
    } finally {
      if (generation.current === current) setBusy(false);
    }
  }
  const inactive = disabled || busy || !sessionId;
  const type = preview?.key.kind;
  const operations =
    type === "string"
      ? ["SET"]
      : type === "hash"
        ? ["HSET", "HDEL"]
        : type === "list"
          ? ["LSET", "LPUSH", "RPUSH"]
          : type === "set"
            ? ["SADD", "SREM"]
            : type === "zset"
              ? ["ZADD", "ZREM"]
              : [];
  const needsField = ["HSET", "HDEL", "LSET", "ZADD"].includes(operation);
  const needsValue = !["HDEL", "UNLINK", "PERSIST"].includes(operation);
  function review() {
    if (!preview) return;
    try {
      onCommand(redisEditCommand(operation, preview.key.argument, field, value));
    } catch (error) {
      setError(messageOf(error));
    }
  }
  return (
    <details className="redis-tools">
      <summary>Key browser · live SCAN and typed value previews</summary>
      <div className="redis-panels">
        <section aria-label="Redis key browser">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void scan();
            }}
            className="redis-controls"
          >
            <input
              aria-label="Key pattern"
              value={pattern}
              maxLength={1024}
              disabled={inactive}
              onChange={(event) => {
                setPattern(event.target.value);
                setPage(null);
              }}
            />
            <button className="button small" disabled={inactive}>
              Scan keys
            </button>
            {page?.nextCursor && (
              <button
                type="button"
                className="button small ghost"
                disabled={inactive}
                onClick={() => void scan(page.nextCursor)}
              >
                Next keys
              </button>
            )}
          </form>
          <p className="redis-hint">
            {page
              ? `${page.keys.length} keys on this page${page.nextCursor ? " · more pages available" : " · scan complete"}`
              : "Choose a MATCH pattern and scan."}{" "}
            · Live keys can expire, change or repeat between pages.
          </p>
          <div className="redis-key-list">
            {page?.keys.map((key) => (
              <button
                key={key.argument}
                disabled={inactive}
                title={key.argument}
                onClick={() => void inspect(key.argument)}
              >
                <code>{key.key || '""'}</code>
                <span>
                  {key.kind} · {ttl(key.ttlMs)}
                </span>
              </button>
            ))}
          </div>
        </section>
        <section aria-label="Redis value inspector">
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void inspect();
            }}
            className="redis-controls"
          >
            <input
              aria-label="Quoted Redis key"
              value={argument}
              maxLength={262144}
              disabled={inactive}
              onChange={(event) => {
                setArgument(event.target.value);
                setPreview(null);
              }}
            />
            <button className="button small" disabled={inactive}>
              Inspect key
            </button>
            {preview?.nextCursor && (
              <button
                type="button"
                className="button small ghost"
                disabled={inactive}
                onClick={() => void inspect(preview.key.argument, preview.nextCursor)}
              >
                Next values
              </button>
            )}
          </form>
          {preview && (
            <>
              <p className="redis-hint">
                {preview.key.kind} · {ttl(preview.key.ttlMs)}
                {preview.truncated
                  ? " · bounded preview; use Next values or a console range for more"
                  : ""}
              </p>
              <div className="redis-value-list">
                <table>
                  <tbody>
                    {preview.entries.map(({ field, value, fieldBinary, valueBinary }, i) => (
                      <tr key={i}>
                        <th>
                          {field ?? i}
                          {fieldBinary && <small> · binary</small>}
                        </th>
                        <td>
                          <pre>{value === null ? "(nil)" : value === "" ? '""' : value}</pre>
                          {valueBinary && <small>Binary bytes · quoted escapes</small>}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {preview.key.kind !== "none" && !readOnly && (
                <div className="redis-controls redis-edit">
                  <select
                    aria-label="Redis edit operation"
                    value={operation}
                    disabled={inactive}
                    onChange={(event) => setOperation(event.target.value)}
                  >
                    {[...operations, "PEXPIRE", "PERSIST", "UNLINK"].map((command) => (
                      <option key={command}>{command}</option>
                    ))}
                  </select>
                  {needsField && (
                    <input
                      aria-label="Field, index or score"
                      placeholder={
                        operation === "LSET" ? "Index" : operation === "ZADD" ? "Score" : "Field"
                      }
                      value={field}
                      disabled={inactive}
                      onChange={(event) => setField(event.target.value)}
                    />
                  )}
                  {needsValue && (
                    <textarea
                      rows={1}
                      aria-label="Redis edit value"
                      placeholder={
                        operation === "PEXPIRE" ? "Expiry in milliseconds" : "Value / member"
                      }
                      value={value}
                      disabled={inactive}
                      maxLength={262144}
                      onChange={(event) => setValue(event.target.value)}
                    />
                  )}
                  <button className="button small" disabled={inactive} onClick={review}>
                    Review command
                  </button>
                </div>
              )}
            </>
          )}
        </section>
      </div>
      {error && (
        <p className="redis-error" role="alert">
          {error}
        </p>
      )}
    </details>
  );
}
function ttl(value: string) {
  return value === "-2" ? "expired / missing" : value === "-1" ? "no expiry" : `${value} ms TTL`;
}
