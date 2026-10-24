import { useState, type FormEvent } from "react";
import { Pencil, Plus, Trash2 } from "lucide-react";
import type { ConnectionGroup } from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { useStudio } from "../../state/studio";
import { InlineError } from "../../components/ui/Modal";

export function orderedGroups(groups: ConnectionGroup[]) {
  const output: { group: ConnectionGroup; depth: number; path: string }[] = [];
  const visited = new Set<string>();
  function add(parent: string | null, depth: number, prefix: string) {
    for (const group of groups
      .filter((g) => g.parentId === parent)
      .sort((a, b) => a.name.localeCompare(b.name))) {
      if (visited.has(group.id) || depth > 8) continue;
      visited.add(group.id);
      const path = prefix ? `${prefix} / ${group.name}` : group.name;
      output.push({ group, depth, path });
      add(group.id, depth + 1, path);
    }
  }
  add(null, 0, "");
  return output;
}

export function GroupManager() {
  const bootstrap = useStudio((state) => state.bootstrap),
    [editing, setEditing] = useState<ConnectionGroup | null>(null),
    [name, setName] = useState(""),
    [parentId, setParentId] = useState("local"),
    [busy, setBusy] = useState(false),
    [error, setError] = useState<string | null>(null);
  const groups = bootstrap?.groups ?? [],
    ordered = orderedGroups(groups);
  async function save(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await client.call("save_group", {
        group: {
          id: editing?.id ?? crypto.randomUUID(),
          name,
          parentId: parentId || null,
          environment:
            editing?.environment ?? groups.find((g) => g.id === parentId)?.environment ?? "local",
        },
      });
      await useStudio.getState().initialize();
      setEditing(null);
      setName("");
      setParentId("local");
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setBusy(false);
    }
  }
  async function remove(group: ConnectionGroup) {
    setBusy(true);
    setError(null);
    try {
      await client.call("delete_group", { id: group.id });
      await useStudio.getState().initialize();
      if (editing?.id === group.id) {
        setEditing(null);
        setName("");
      }
    } catch (error) {
      setError(messageOf(error));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="group-manager">
      <h3>Connection groups</h3>
      <p className="form-note">
        Child groups inherit their parent’s environment and safety defaults.
      </p>
      <div className="managed-groups">
        {ordered.map(({ group, depth }) => {
          const nonempty =
              groups.some((g) => g.parentId === group.id) ||
              bootstrap?.profiles.some((p) => p.groupId === group.id),
            root = ["production", "staging", "local"].includes(group.id);
          return (
            <div className="managed-group" key={group.id} style={{ paddingLeft: 10 + depth * 16 }}>
              <i className={`environment-dot ${group.environment}`} />
              <span>{group.name}</span>
              <button
                className="icon-button"
                aria-label={`Rename group ${group.name}`}
                disabled={busy}
                onClick={() => {
                  setEditing(group);
                  setName(group.name);
                  setParentId(group.parentId ?? "");
                }}
              >
                <Pencil size={12} />
              </button>
              {!root && (
                <button
                  className="icon-button"
                  aria-label={`Remove group ${group.name}`}
                  title={
                    nonempty ? "Move its connections and child groups first" : "Remove empty group"
                  }
                  disabled={busy || nonempty}
                  onClick={() => void remove(group)}
                >
                  <Trash2 size={12} />
                </button>
              )}
            </div>
          );
        })}
      </div>
      <form className="group-form" onSubmit={(e) => void save(e)}>
        <label>
          Group name
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            maxLength={100}
            required
            placeholder="e.g. Analytics"
          />
        </label>
        <label>
          Parent group
          <select
            aria-label="Parent group"
            value={parentId}
            onChange={(e) => setParentId(e.target.value)}
            disabled={!!editing && ["production", "staging", "local"].includes(editing.id)}
          >
            {editing?.parentId === null && <option value="">Top level</option>}
            {ordered
              .filter(({ group }) => group.id !== editing?.id)
              .map(({ group, path }) => (
                <option key={group.id} value={group.id}>
                  {path}
                </option>
              ))}
          </select>
        </label>
        <div className="group-form-actions">
          <button className="button ghost small" type="submit" disabled={busy}>
            <Plus size={12} />
            {editing ? "Save group" : "Add group"}
          </button>
          {editing && (
            <button
              type="button"
              className="button ghost small"
              onClick={() => {
                setEditing(null);
                setName("");
                setParentId("local");
              }}
            >
              Cancel edit
            </button>
          )}
        </div>
      </form>
      <InlineError message={error} />
    </section>
  );
}
