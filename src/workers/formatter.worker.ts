import { format } from "sql-formatter";
self.onmessage = ({
  data,
}: MessageEvent<{ id: number; sql: string; language: "postgresql" | "mysql" | "snowflake" }>) => {
  try {
    self.postMessage({
      id: data.id,
      sql: format(data.sql, { language: data.language, keywordCase: "upper", tabWidth: 2 }),
    });
  } catch (error) {
    self.postMessage({
      id: data.id,
      error: error instanceof Error ? error.message : "Could not format SQL",
    });
  }
};
