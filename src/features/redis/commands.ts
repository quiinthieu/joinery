export const REDIS_COMMANDS = [
  "PING",
  "DBSIZE",
  "GET",
  "GETRANGE",
  "TYPE",
  "TTL",
  "PTTL",
  "STRLEN",
  "EXISTS",
  "MGET",
  "HGET",
  "HMGET",
  "HLEN",
  "HSCAN",
  "LLEN",
  "LRANGE",
  "SCARD",
  "SSCAN",
  "ZCARD",
  "ZRANGE",
  "ZSCAN",
  "SCAN",
  "SET",
  "HSET",
  "HDEL",
  "LSET",
  "LPUSH",
  "RPUSH",
  "SADD",
  "SREM",
  "ZADD",
  "ZREM",
  "DEL",
  "UNLINK",
  "EXPIRE",
  "PEXPIRE",
  "PERSIST",
];

export function redisArgument(value: string) {
  return `"${value.replace(/[\\"\u0000-\u001f\u007f]/g, (char) =>
    char === '"' || char === "\\"
      ? `\\${char}`
      : `\\x${char.charCodeAt(0).toString(16).padStart(2, "0")}`,
  )}"`;
}

export function redisEditCommand(operation: string, key: string, field: string, value: string) {
  if (
    ![
      "SET",
      "HSET",
      "HDEL",
      "LSET",
      "LPUSH",
      "RPUSH",
      "SADD",
      "SREM",
      "ZADD",
      "ZREM",
      "PEXPIRE",
      "PERSIST",
      "UNLINK",
    ].includes(operation)
  )
    throw new Error("Choose a supported Redis edit operation");
  if (operation === "LSET" && !/^-?\d+$/.test(field))
    throw new Error("Enter an integer list index");
  if (
    operation === "ZADD" &&
    (!/^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$/.test(field) || !Number.isFinite(Number(field)))
  )
    throw new Error("Enter a finite sorted-set score");
  if (operation === "PEXPIRE" && !/^\d+$/.test(value))
    throw new Error("Enter a nonnegative expiry in milliseconds");
  let command = `${operation} ${key}`;
  if (["HSET", "HDEL", "LSET", "ZADD"].includes(operation))
    command += ` ${["LSET", "ZADD"].includes(operation) ? field : redisArgument(field)}`;
  if (!["HDEL", "PERSIST", "UNLINK"].includes(operation))
    command += ` ${operation === "PEXPIRE" ? value : redisArgument(value)}`;
  if (operation === "SET") command += " KEEPTTL";
  return command;
}
