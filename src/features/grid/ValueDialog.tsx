import { useState, type FormEvent } from "react";
import { Modal, InlineError } from "../../components/ui/Modal";
import { dirtyStore } from "../../data/dirty-store";
import { resultCache } from "../../data/result-cache";
import type { CellTarget } from "./DatabaseGrid";

export function ValueDialog({ target, onClose }: { target: CellTarget; onClose(): void }) {
  const [value, setValue] = useState(target.value ?? ""), [isNull, setNull] = useState(target.value === null), [error, setError] = useState<string | null>(null);
  function save(event: FormEvent) {
    event.preventDefault();
    try {
      if (!isNull && target.column.kind === "json") JSON.parse(value);
      if (!isNull && target.column.kind === "integer" && !/^[+-]?\d+$/.test(value)) throw new Error("Enter a whole number without losing precision");
      dirtyStore.set({ resultId: target.resultId, row: target.row, column: target.column.id, original: resultCache.read(target.resultId, target.column.id, target.row, true)?.value ?? null, value: isNull ? null : value }); onClose();
    } catch (error) { setError(error instanceof Error ? error.message : "The value is invalid"); }
  }
  return <Modal title={target.column.editable ? `Edit ${target.column.name}` : target.column.name} subtitle={`${target.column.databaseType}${target.column.primaryKey ? " · primary key" : ""}`} onClose={onClose} wide>
    <form onSubmit={save}><div className="modal-body">
      {target.column.editable && target.column.nullable && <label className="checkbox-label"><input type="checkbox" checked={isNull} onChange={(e) => setNull(e.target.checked)} />Set to NULL</label>}
      {target.column.editable && target.column.databaseType === "date" && !isNull ? <input type="date" value={value} onChange={(e) => setValue(e.target.value)} /> : <textarea className="value-editor" aria-label="Cell value" autoFocus value={isNull ? "NULL" : value} onChange={(e) => setValue(e.target.value)} readOnly={!target.column.editable || isNull} spellCheck={false} />}
      <InlineError message={error} />
    </div><footer className="modal-footer"><button className="button ghost" type="button" onClick={onClose}>{target.column.editable ? "Cancel" : "Close"}</button>{target.column.editable && <button type="submit" className="button primary">Keep edit</button>}</footer></form>
  </Modal>;
}
