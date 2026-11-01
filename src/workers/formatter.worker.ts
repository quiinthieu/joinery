import type { SqlFormatSettings } from "../ipc/generated";
import { formatSqlText, type SqlFormatterDialect } from "../data/sql-format";

self.onmessage = ({
  data,
}: MessageEvent<{
  id: number;
  sql: string;
  language: SqlFormatterDialect;
  settings?: SqlFormatSettings;
}>) => {
  try {
    self.postMessage({
      id: data.id,
      sql: formatSqlText(data.sql, data.language, data.settings),
    });
  } catch (error) {
    self.postMessage({
      id: data.id,
      error: error instanceof Error ? error.message : "Could not format SQL",
    });
  }
};
