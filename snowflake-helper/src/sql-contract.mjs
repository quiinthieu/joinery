import contract from "./sql-contract.json" with { type: "json" };

// Bundled source only: callers cannot supply a replacement contract to Helper.
// Keep this closed schema aligned with the Rust build-time validator.
export function validateSqlContract(value) {
  const keys = ["contractVersion", "bootstrapContext", "version", "begin", "commit", "rollback"];
  if (
    value === null ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).length !== keys.length ||
    !keys.every((key) => Object.hasOwn(value, key)) ||
    value.contractVersion !== 1 ||
    !keys
      .slice(1)
      .every(
        (key) =>
          typeof value[key] === "string" &&
          value[key].length > 0 &&
          value[key].length <= 1024 &&
          /^[\x20-\x7e]+$/.test(value[key]) &&
          value[key].trim() === value[key] &&
          !value[key].includes(";"),
      )
  )
    throw new Error("Invalid bundled Snowflake SQL contract");
  return Object.freeze(Object.fromEntries(keys.map((key) => [key, value[key]])));
}

export const SQL_CONTRACT = validateSqlContract(contract);
