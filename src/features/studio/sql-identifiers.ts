import { MySQL, PostgreSQL, SQLDialect, StandardSQL } from "@codemirror/lang-sql";
import type { EngineKind } from "../../ipc/generated";
import { identifierValue, type Identifier } from "../../data/sql-identifier";
export { identifierValue, type Identifier } from "../../data/sql-identifier";

// Snowflake strings may contain arbitrary SQL, including semicolons and FROM.
const Snowflake = SQLDialect.define({
  ...StandardSQL.spec,
  doubleDollarQuotedStrings: true,
  backslashEscapes: true,
});

export function sqlDialect(engine: EngineKind) {
  return engine === "postgres" ? PostgreSQL : engine === "mysql" ? MySQL : Snowflake;
}

export function readIdentifier(raw: string, complete = true): Identifier {
  const quote = raw[0];
  if (quote !== '"' && quote !== "`") return { value: raw, quoted: false };
  const body = raw.slice(1, complete && raw.endsWith(quote) && raw.length > 1 ? -1 : undefined);
  return { value: body.split(quote + quote).join(quote), quoted: true };
}

export function quoteIdentifier(name: string, engine: EngineKind): string {
  // Ask the installed dialect whether a simple name is an identifier rather
  // than maintaining a second, inevitably incomplete reserved-word list.
  if (
    /^[\p{L}_][\p{L}\p{N}_$]*$/u.test(name) &&
    identifierValue({ value: name, quoted: false }, engine) === name
  ) {
    const statement = sqlDialect(engine).language.parser.parse(name).topNode.firstChild;
    if (statement?.firstChild?.name === "Identifier" && !statement.firstChild.nextSibling)
      return name;
  }
  const quote = engine === "mysql" ? "`" : '"';
  return quote + name.split(quote).join(quote + quote) + quote;
}
