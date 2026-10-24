import { expect, test } from "vitest";
import { redisArgument, redisEditCommand } from "./commands";

test("key and value forms cannot inject another command", () => {
  expect(redisArgument('a"\nDEL other\r\n\\b\0')).toBe('"a\\"\\x0aDEL other\\x0d\\x0a\\\\b\\x00"');
  expect(redisArgument("")).toBe('""');
  expect(redisArgument("café 🐳")).toBe('"café 🐳"');
});

test("numeric edit fields cannot inject commands and string edits preserve TTL", () => {
  for (const operation of ["LSET", "ZADD"])
    expect(() => redisEditCommand(operation, '"key"', "0 value\nUNLINK other", "v")).toThrow();
  expect(() => redisEditCommand("PEXPIRE", '"key"', "", "0\nUNLINK other")).toThrow();
  expect(redisEditCommand("SET", '"key"', "", "")).toBe('SET "key" "" KEEPTTL');
  expect(redisEditCommand("HSET", '"key"', "a\nb", "c\nd")).toBe('HSET "key" "a\\x0ab" "c\\x0ad"');
});
