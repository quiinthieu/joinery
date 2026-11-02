import { expect, test, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { history, undo } from "@codemirror/commands";
import { nextSnippetField } from "@codemirror/autocomplete";
import { applySqlTemplate } from "./sql-template-context";

test("context variables preserve quoted identifiers, literal selected SQL, tab stops and one undo", async () => {
  const selected = "SELECT '${9:literal}', '{{database}}', '\\{value\\}'\n\tFROM orders";
  const prefix = "  ";
  const editor = new EditorView({
    state: EditorState.create({
      doc: prefix + selected,
      selection: { anchor: prefix.length, head: prefix.length + selected.length },
      extensions: [history()],
    }),
  });
  try {
    await applySqlTemplate(
      "SELECT ${1:*} FROM {{database}}.{{schema}}.${2:orders};\n{{selection}}${0}",
      "postgres",
      editor,
      undefined,
      prefix.length,
      editor.state.doc.length,
      {
        resolveContext: async () => ({ database: 'Odd"Db', schema: 'Odd"Schema' }),
        isCurrent: () => true,
      },
    );
    expect(editor.state.doc.toString()).toBe(
      '  SELECT * FROM "Odd""Db"."Odd""Schema".orders;\n  ' + selected,
    );
    expect(
      editor.state.sliceDoc(editor.state.selection.main.from, editor.state.selection.main.to),
    ).toBe("*");
    expect(nextSnippetField(editor)).toBe(true);
    expect(
      editor.state.sliceDoc(editor.state.selection.main.from, editor.state.selection.main.to),
    ).toBe("orders");
    expect(nextSnippetField(editor)).toBe(true);
    expect(editor.state.selection.main.head).toBe(editor.state.doc.length);
    expect(undo(editor)).toBe(true);
    expect(editor.state.doc.toString()).toBe(prefix + selected);
  } finally {
    editor.destroy();
  }
});

test.each(["postgres", "mysql", "snowflake"] as const)(
  "%s quotes context values without parsing their contents",
  async (engine) => {
    const editor = new EditorView({
      state: EditorState.create({ doc: "ctx", selection: { anchor: 3 } }),
    });
    try {
      const database = 'Odd`"${9:literal}{{schema}}\\{日本語}';
      await applySqlTemplate("{{database}}.{{schema}}${0}", engine, editor, undefined, 0, 3, {
        resolveContext: async () => ({ database, schema: "MixedCase" }),
        isCurrent: () => true,
      });
      const quote = engine === "mysql" ? "`" : '"';
      expect(editor.state.doc.toString()).toBe(
        quote +
          database.split(quote).join(quote + quote) +
          quote +
          "." +
          quote +
          "MixedCase" +
          quote,
      );
    } finally {
      editor.destroy();
    }
  },
);

test("lower and upper change selected text without reading context", async () => {
  const selected = "JOINERY_TEMPLATE_VALUE_0_0_END Select Id";
  const editor = new EditorView({
    state: EditorState.create({ doc: selected, selection: { anchor: 0, head: selected.length } }),
  });
  const resolveContext = vi.fn(async () => ({}));
  try {
    await applySqlTemplate(
      "{{lower(selection)}}\n{{upper(selection)}}${0}",
      "postgres",
      editor,
      undefined,
      0,
      selected.length,
      { resolveContext, isCurrent: () => true },
    );
    expect(editor.state.doc.toString()).toBe(
      "joinery_template_value_0_0_end select id\nJOINERY_TEMPLATE_VALUE_0_0_END SELECT ID",
    );
    expect(resolveContext).not.toHaveBeenCalled();
  } finally {
    editor.destroy();
  }
});

test("a lowercased selection stays literal beside a quoted database", async () => {
  const selected = "Select Id";
  const editor = new EditorView({
    state: EditorState.create({ doc: selected, selection: { anchor: 0, head: selected.length } }),
  });
  try {
    await applySqlTemplate(
      "{{database}}.{{lower(selection)}}${0}",
      "mysql",
      editor,
      undefined,
      0,
      selected.length,
      {
        resolveContext: async () => ({ database: "Odd`Db" }),
        isCurrent: () => true,
      },
    );
    expect(editor.state.doc.toString()).toBe("`Odd``Db`.select id");
  } finally {
    editor.destroy();
  }
});

test.each([
  ["{{unknown}}", "Unsupported template variable"],
  ["{{title(selection)}}", "Unsupported template variable"],
  ["{{database}", "Incomplete template variable"],
  ["{{selection}}", "Select SQL"],
  ["{{lower(selection)}}", "Select SQL"],
  ["{{upper(selection)}}", "Select SQL"],
  ["{{database}}", "No observed current database"],
  ["{{schema}}", "No observed current schema"],
])(
  "%s blocks incomplete or unavailable values without changing the editor",
  async (body, message) => {
    const editor = new EditorView({ state: EditorState.create({ doc: "unchanged" }) });
    const resolveContext = vi.fn(async () => ({}));
    try {
      await expect(
        applySqlTemplate(body, "postgres", editor, undefined, 0, 0, {
          resolveContext,
          isCurrent: () => true,
        }),
      ).rejects.toThrow(message);
      expect(editor.state.doc.toString()).toBe("unchanged");
      expect(resolveContext).toHaveBeenCalledTimes(
        body === "{{database}}" || body === "{{schema}}" ? 1 : 0,
      );
    } finally {
      editor.destroy();
    }
  },
);

test.each(["document", "selection", "context"] as const)(
  "rejects a pending expansion after %s changes",
  async (change) => {
    const editor = new EditorView({
      state: EditorState.create({ doc: "ctx", selection: { anchor: 3 } }),
    });
    let finish!: (value: { database: string }) => void;
    let current = true;
    try {
      const pending = applySqlTemplate("{{database}}", "postgres", editor, undefined, 0, 3, {
        resolveContext: () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
        isCurrent: () => current,
      });
      if (change === "document") editor.dispatch({ changes: { from: 0, insert: "-- edited\n" } });
      if (change === "selection") editor.dispatch({ selection: { anchor: 0, head: 3 } });
      if (change === "context") current = false;
      const before = editor.state.doc.toString();
      finish({ database: "other" });
      await expect(pending).rejects.toThrow("changed");
      expect(editor.state.doc.toString()).toBe(before);
    } finally {
      editor.destroy();
    }
  },
);

test("a context read failure preserves text and selection", async () => {
  const editor = new EditorView({
    state: EditorState.create({ doc: "ctx", selection: { anchor: 3 } }),
  });
  try {
    await expect(
      applySqlTemplate("{{database}}", "postgres", editor, undefined, 0, 3, {
        resolveContext: async () => {
          throw new Error("Connection closed");
        },
        isCurrent: () => true,
      }),
    ).rejects.toThrow("Connection closed");
    expect(editor.state.doc.toString()).toBe("ctx");
    expect(editor.state.selection.main.head).toBe(3);
  } finally {
    editor.destroy();
  }
});

test("selected SQL repeats literally without context reads, including marker-like text", async () => {
  const selected = "JOINERY_TEMPLATE_VALUE_0_0_END\n\t${1:literal} {{selection}}";
  const editor = new EditorView({
    state: EditorState.create({ doc: selected, selection: { anchor: 0, head: selected.length } }),
  });
  const resolveContext = vi.fn(async () => ({}));
  try {
    await applySqlTemplate(
      "{{selection}}\n{{selection}}${0}",
      "mysql",
      editor,
      undefined,
      0,
      selected.length,
      { resolveContext, isCurrent: () => true },
    );
    expect(editor.state.doc.toString()).toBe(selected + "\n" + selected);
    expect(resolveContext).not.toHaveBeenCalled();
  } finally {
    editor.destroy();
  }
});

test("context variables work inside repeated editable fields", async () => {
  const editor = new EditorView({ state: EditorState.create() });
  try {
    await applySqlTemplate(
      "${1:{{schema}}}.orders; ${1:{{schema}}}.customers;${0}",
      "postgres",
      editor,
      undefined,
      0,
      0,
      {
        resolveContext: async () => ({ schema: "public" }),
        isCurrent: () => true,
      },
    );
    expect(editor.state.doc.toString()).toBe('"public".orders; "public".customers;');
    expect(
      editor.state.sliceDoc(editor.state.selection.main.from, editor.state.selection.main.to),
    ).toBe('"public"');
    expect(nextSnippetField(editor)).toBe(true);
    expect(editor.state.selection.main.head).toBe(editor.state.doc.length);
  } finally {
    editor.destroy();
  }
});

test("repeated selection expansion is bounded", async () => {
  const selected = "x".repeat(600_000);
  const editor = new EditorView({
    state: EditorState.create({ doc: selected, selection: { anchor: 0, head: selected.length } }),
  });
  try {
    await expect(
      applySqlTemplate(
        "{{selection}}{{selection}}",
        "postgres",
        editor,
        undefined,
        0,
        selected.length,
        {
          resolveContext: async () => ({}),
          isCurrent: () => true,
        },
      ),
    ).rejects.toThrow("1 MiB");
    expect(editor.state.doc.length).toBe(selected.length);
  } finally {
    editor.destroy();
  }
});
