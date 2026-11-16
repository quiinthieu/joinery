import { useEffect, useId, useMemo, useRef, useState } from "react";
import type {
  BrowseCatalog,
  ChangePreview,
  PreviewRelationRequest,
  RelationCheckConstraints,
  RelationConstraintMappings,
  RelationConstraints,
  RelationDefinition,
  RelationFilter,
  RelationFilterOperator,
  RelationInspection,
  RelationPreview,
  RelationSort,
  RelationSortDirection,
  RelationSortNulls,
  SnowflakeGridCellInput,
  SnowflakeGridFinishAction,
  SnowflakeGridMode,
} from "../../ipc/generated";
import { client, messageOf } from "../../ipc/client";
import { querySessionForTab, revokeSchemaSelectionBeforeWork, useStudio } from "../../state/studio";
import { InlineError, Modal } from "../../components/ui/Modal";
import { tabNavigation } from "../../components/ui/tabs";
import { JsonTreeViewer, SpatialViewer } from "../grid/ValueDialog";
import {
  MAX_HEX_PREVIEW_BYTES,
  buildJsonTree,
  decodeBinaryPreview,
  formatHexDump,
  formatJsonLossless,
  isBinaryColumn,
  isJsonColumn,
  isRawHexBinaryType,
  isSpatialColumn,
  parseSpatialPreview,
} from "../grid/value-format";
import { ReviewDialog } from "../safety/ReviewDialog";
import { SnowflakeDdlDialog, type SnowflakeDdlDialogRequest } from "./SnowflakeDdlDialog";
import { VirtualForeignKeyDialog } from "./VirtualForeignKeyDialog";
import { DdlPanel } from "./ObjectInspectorView";
import {
  MAX_SNOWFLAKE_GRID_MUTATIONS,
  SAFE_SNOWFLAKE_BATCH_FRAME_BYTES,
  buildSnowflakeGridMutations,
  canColumnDefaultOnInsert,
  cloneSnowflakePreviewRow,
  createBlankInsertRow,
  estimateInsertRowsFrameBytes,
  isWritableSnowflakeColumn,
  stageCsvInsertBatches,
  type CsvFieldDelimiter,
  type StagedInsertRow,
} from "./snowflake-grid-staging";

const PAGE_ROWS = 25;
const PAGE_COLUMNS = 20;
const SERVER_PAGE_ROWS = 500;
const MAX_SERVER_OFFSET = 1_000_000;
const MAX_PREVIEW_FILTERS = 16;
const INSERT_PAGE_ROWS = 25;

interface AppliedPreviewSlice {
  offset: number;
  sort: RelationSort | null;
  filters: RelationFilter[];
}

interface DraftFilterRow {
  id: string;
  column: string;
  operator: RelationFilterOperator;
  value: string;
}

const DEFAULT_PREVIEW_SLICE: AppliedPreviewSlice = {
  offset: 0,
  sort: null,
  filters: [],
};

function buildPreviewRequest(
  revision: string,
  targetId: string,
  inspectionRevision: string,
  slice: AppliedPreviewSlice,
): PreviewRelationRequest {
  const req: PreviewRelationRequest = { revision, targetId, inspectionRevision };
  if (slice.offset > 0) req.offset = slice.offset;
  if (slice.sort) req.sort = slice.sort;
  if (slice.filters.length > 0) req.filters = slice.filters;
  return req;
}

function quotedName(...parts: string[]) {
  return parts.map((part) => `"${part.replaceAll('"', '""')}"`).join(".");
}

type KeyMapping = RelationConstraintMappings["mappings"][number];

function referencedDestination(catalog: BrowseCatalog | null, mapping: KeyMapping) {
  const reference = mapping.referencedTable;
  if (!catalog || mapping.kind !== "foreignKey" || mapping.status !== "available" || !reference)
    return null;
  if (reference.database !== catalog.database)
    return {
      reason:
        "The referenced table is in another database. Cross-database navigation is not available yet.",
      target: null,
    };
  // Count every exact identity before availability; a hidden/shadowed duplicate
  // must never let an available sibling win by elimination.
  const matches = catalog.relations.filter(
    (entry) =>
      entry.database === reference.database &&
      entry.table.schema === reference.table.schema &&
      entry.table.name === reference.table.name,
  );
  if (matches.length === 0)
    return {
      reason:
        "The referenced table is absent from this retained catalog. Refresh relations explicitly to check visibility.",
      target: null,
    };
  if (matches.length !== 1)
    return { reason: "The referenced table identity is ambiguous in this catalog.", target: null };
  if (matches[0].availability !== "available")
    return {
      reason: `The referenced table is unavailable (${matches[0].availability}).`,
      target: null,
    };
  return { reason: null, target: matches[0] };
}

function relationContextKey(
  state: ReturnType<typeof useStudio.getState>,
  sessionId: string,
  ownerTabId?: string,
) {
  const owner = state.tabs.find((tab) => tab.id === ownerTabId);
  const owned = owner && querySessionForTab(owner, state.sessions);
  const session =
    owned?.id === sessionId
      ? owned
      : Object.values(state.sessions).find((entry) => entry.id === sessionId);
  const runtime = state.sessionStates[sessionId];
  return JSON.stringify([
    sessionId,
    session?.id,
    session?.connectionId,
    !ownerTabId || owned?.id === sessionId,
    !!(
      state.sessionTransactionPending[sessionId] ||
      state.sessionMetadataRefreshRequired[sessionId] ||
      state.sessionDdlCatalogRefreshRequired[sessionId] ||
      state.sessionContextSwitching[sessionId] ||
      runtime?.transactionControlAuthority?.status === "rollbackOnly"
    ),
    state.sessionContextRevision[sessionId] ?? 0,
    // The first background observation installs an unknown SDK runtime. It
    // supplies no new context, so it must not revoke an already admitted read.
    runtime?.database ?? "",
    runtime?.activeContext ?? null,
    runtime?.role ?? null,
    runtime?.searchPath ?? [],
    runtime?.contextFingerprint ?? null,
  ]);
}

/** Every action carries server-held target revisions from this console session. */
export function RelationBrowser({
  sessionId,
  ownerTabId,
  onClose,
}: {
  sessionId: string;
  ownerTabId?: string;
  onClose(): void;
}) {
  const session = useStudio((state) => {
    const owner = state.tabs.find((tab) => tab.id === ownerTabId);
    const owned = owner && querySessionForTab(owner, state.sessions);
    return owned?.id === sessionId
      ? owned
      : Object.values(state.sessions).find((entry) => entry.id === sessionId);
  });
  const profile = useStudio((state) =>
    state.bootstrap?.profiles.find((entry) => entry.id === session?.connectionId),
  );
  const gridTx = useStudio((state) => state.sessionSnowflakeGridTransactions[sessionId] ?? null);
  const gridBusy = useStudio((state) => Boolean(state.sessionSnowflakeGridPending[sessionId]));
  const metadataRefreshRequired = useStudio((state) =>
    Boolean(
      state.sessionMetadataRefreshRequired[sessionId] ||
      state.sessionDdlCatalogRefreshRequired[sessionId],
    ),
  );
  const blocked = useStudio(
    (state) =>
      !!(
        state.sessionTransactionPending[sessionId] ||
        state.sessionSnowflakeGridPending[sessionId] ||
        state.sessionSnowflakeGridTransactions[sessionId] ||
        state.sessionMetadataRefreshRequired[sessionId] ||
        state.sessionDdlCatalogRefreshRequired[sessionId] ||
        state.sessionContextSwitching[sessionId] ||
        state.sessionStates[sessionId]?.transactionControlAuthority?.status === "rollbackOnly"
      ),
  );
  const ownerAvailable = useStudio((state) => {
    if (!ownerTabId) return true;
    const owner = state.tabs.find((tab) => tab.id === ownerTabId);
    return !!owner && querySessionForTab(owner, state.sessions)?.id === sessionId;
  });
  const contextKey = useStudio((state) => relationContextKey(state, sessionId, ownerTabId));
  const renderedOwner = ownerTabId
    ? useStudio.getState().tabs.find((tab) => tab.id === ownerTabId)
    : undefined;
  const canDdl =
    session?.capabilities.snowflakeReviewedDdl === true &&
    profile?.snowflake?.reviewedDdl === true &&
    profile.safeMode !== "readOnly";
  const canEditGrid =
    session?.capabilities.snowflakeGridEditing === true &&
    profile?.snowflake?.reviewedDml === true &&
    !profile?.snowflake?.reviewedConsole &&
    profile.safeMode !== "readOnly";
  const [ddlRequest, setDdlRequest] = useState<SnowflakeDdlDialogRequest | null>(null);
  const [gridReview, setGridReview] = useState<ChangePreview | null>(null);
  const [gridMode, setGridMode] = useState<SnowflakeGridMode>("automatic");
  const [deletedRows, setDeletedRows] = useState<Record<number, true>>({});
  const [updatedCells, setUpdatedCells] = useState<
    Record<number, Record<number, SnowflakeGridCellInput>>
  >({});
  const [insertedRows, setInsertedRows] = useState<StagedInsertRow[]>([]);
  const [insertPage, setInsertPage] = useState(0);
  const [csvImportQueue, setCsvImportQueue] = useState<{
    remainingBatches: StagedInsertRow[][];
    batchNumber: number;
    totalBatches: number;
    totalRows: number;
  } | null>(null);
  const [consumedSnapshotId, setConsumedSnapshotId] = useState<string | null>(null);
  const [gridError, setGridError] = useState<string | null>(null);
  const [csvPanelOpen, setCsvPanelOpen] = useState(false);
  const [csvRawText, setCsvRawText] = useState("");
  const [csvDelimiter, setCsvDelimiter] = useState<CsvFieldDelimiter>(",");
  const [csvHasHeader, setCsvHasHeader] = useState(true);
  const [csvNullToken, setCsvNullToken] = useState("\\N");
  const [draftSortColumn, setDraftSortColumn] = useState("");
  const [draftSortDirection, setDraftSortDirection] = useState<RelationSortDirection>("asc");
  const [draftSortNulls, setDraftSortNulls] = useState<"" | RelationSortNulls>("");
  const [draftFilters, setDraftFilters] = useState<DraftFilterRow[]>([]);
  const [appliedSlice, setAppliedSlice] = useState<AppliedPreviewSlice>(DEFAULT_PREVIEW_SLICE);
  const [catalog, setCatalog] = useState<BrowseCatalog | null>(null);
  const [inspection, setInspection] = useState<RelationInspection | null>(null);
  const [preview, setPreview] = useState<RelationPreview | null>(null);
  const [definition, setDefinition] = useState<{
    contextKey: string;
    value: RelationDefinition;
  } | null>(null);
  const [constraints, setConstraints] = useState<{
    contextKey: string;
    value: RelationConstraints;
  } | null>(null);
  const [constraintsError, setConstraintsError] = useState<string | null>(null);
  const [mappings, setMappings] = useState<{
    contextKey: string;
    value: RelationConstraintMappings;
  } | null>(null);
  const [mappingsError, setMappingsError] = useState<string | null>(null);
  const [checks, setChecks] = useState<{
    contextKey: string;
    value: RelationCheckConstraints;
  } | null>(null);
  const [checksError, setChecksError] = useState<string | null>(null);
  const [selected, setSelected] = useState("");
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rowPage, setRowPage] = useState(0);
  const [columnPage, setColumnPage] = useState(0);
  const [previewTransposed, setPreviewTransposed] = useState(false);
  const [virtualFkDialogOpen, setVirtualFkDialogOpen] = useState(false);
  const virtualForeignKeys = useStudio((state) => state.virtualForeignKeys);
  const [inspectedCell, setInspectedCell] = useState<{
    absRowIndex: number;
    colIndex: number;
  } | null>(null);
  const generation = useRef(0);
  const activationEpoch = generation.current;
  const navigationId = useId();
  const relationSelector = useRef<HTMLSelectElement>(null);
  const retainedSelection = useRef({ catalog, mappings, selected, sessionId, ownerTabId });
  retainedSelection.current = { catalog, mappings, selected, sessionId, ownerTabId };
  const mounted = useRef(false);
  const dispatchedGridPlanRef = useRef<string | null>(null);
  const pendingRefreshAfterResume = useRef(false);
  const renderedContext = useRef(contextKey);
  renderedContext.current = contextKey;
  const clearStagedGridEdits = () => {
    setDeletedRows({});
    setUpdatedCells({});
    setInsertedRows([]);
    setInsertPage(0);
    setCsvImportQueue(null);
    setConsumedSnapshotId(null);
    setGridReview(null);
    setGridError(null);
    setCsvPanelOpen(false);
    setInspectedCell(null);
  };
  const resetPreviewSlice = () => {
    setDraftSortColumn("");
    setDraftSortDirection("asc");
    setDraftSortNulls("");
    setDraftFilters([]);
    setPreviewTransposed(false);
    setAppliedSlice(DEFAULT_PREVIEW_SLICE);
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      generation.current++;
    };
  }, []);
  useEffect(() => {
    const invalidate = () => {
      generation.current++;
      setCatalog(null);
      setInspection(null);
      setPreview(null);
      setDefinition(null);
      setConstraints(null);
      setConstraintsError(null);
      setMappings(null);
      setMappingsError(null);
      setChecks(null);
      setChecksError(null);
      setSelected("");
      setBusy(false);
      setError(null);
      clearStagedGridEdits();
      resetPreviewSlice();
    };
    invalidate();
    // Observe every transition, including a hold set and cleared within one
    // React batch. A later response cannot revive its earlier authority.
    return useStudio.subscribe((next, previous) => {
      if (
        relationContextKey(next, sessionId, ownerTabId) !==
        relationContextKey(previous, sessionId, ownerTabId)
      )
        invalidate();
    });
  }, [sessionId, ownerTabId]);

  const run = async (
    work: (current: () => boolean) => Promise<void>,
    onFailure?: (message: string) => void,
    allowGridTransaction = false,
  ) => {
    const request = ++generation.current;
    const current = () =>
      mounted.current &&
      request === generation.current &&
      renderedContext.current === contextKey &&
      relationContextKey(useStudio.getState(), sessionId, ownerTabId) === contextKey;
    setBusy(true);
    setError(null);
    try {
      revokeSchemaSelectionBeforeWork(sessionId, false, allowGridTransaction);
      await work(current);
    } catch (err) {
      if (current()) {
        if (onFailure) {
          onFailure(messageOf(err));
          return;
        }
        setCatalog(null);
        setInspection(null);
        setPreview(null);
        setDefinition(null);
        setConstraints(null);
        setConstraintsError(null);
        setMappings(null);
        setMappingsError(null);
        setChecks(null);
        setChecksError(null);
        setSelected("");
        setError(messageOf(err));
      }
    } finally {
      if (current()) setBusy(false);
    }
  };
  const refresh = () => {
    clearStagedGridEdits();
    resetPreviewSlice();
    setCatalog(null);
    setInspection(null);
    setPreview(null);
    setDefinition(null);
    setConstraints(null);
    setConstraintsError(null);
    setMappings(null);
    setMappingsError(null);
    setChecks(null);
    setChecksError(null);
    setSelected("");
    setRowPage(0);
    setColumnPage(0);
    void run(async (current) => {
      const result = await client.call("relation_catalog", { sessionId });
      if (current()) setCatalog(result);
    });
  };
  useEffect(() => {
    if (pendingRefreshAfterResume.current && !metadataRefreshRequired && !gridTx && !gridBusy) {
      pendingRefreshAfterResume.current = false;
      refresh();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contextKey, metadataRefreshRequired, gridTx, gridBusy]);
  const resumeMetadata = () => {
    if (busy || gridBusy || gridTx) return;
    setBusy(true);
    setError(null);
    pendingRefreshAfterResume.current = true;
    void useStudio
      .getState()
      .refreshSessionContexts(sessionId, true)
      .catch((err) => {
        pendingRefreshAfterResume.current = false;
        if (mounted.current) {
          setBusy(false);
          setError(messageOf(err));
        }
      });
  };
  // Opening the browser is explicit user intent to load current visible metadata.
  useEffect(() => {
    // Defer dispatch until after React's development effect replay. One open
    // must not start duplicate catalog reads during StrictMode setup/cleanup.
    const timer = setTimeout(refresh, 0);
    return () => clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);
  const inspect = () => {
    const target = catalog?.relations.find((entry) => entry.targetId === selected);
    if (!catalog || !target || target.availability !== "available") return;
    clearStagedGridEdits();
    resetPreviewSlice();
    setInspection(null);
    setPreview(null);
    setRowPage(0);
    setColumnPage(0);
    void run(async (current) => {
      const result = await client.call("inspect_relation", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (current()) setInspection(result);
    });
  };
  const compileDraftSlice = (offset: number): AppliedPreviewSlice => {
    const filters: RelationFilter[] = draftFilters.map((row, idx) => {
      if (!row.column) {
        throw new Error(`Filter ${idx + 1} must select a column.`);
      }
      if (row.operator === "isNull" || row.operator === "isNotNull") {
        return { column: row.column, operator: row.operator };
      }
      if (row.operator === "contains" && row.value.length === 0) {
        throw new Error(`Filter ${idx + 1} contains value must not be empty.`);
      }
      return { column: row.column, operator: row.operator, value: row.value };
    });
    const sort: RelationSort | null = draftSortColumn
      ? {
          column: draftSortColumn,
          direction: draftSortDirection,
          ...(draftSortNulls ? { nulls: draftSortNulls } : {}),
        }
      : null;
    return { offset, sort, filters };
  };
  const fetchPreviewSlice = (nextSlice: AppliedPreviewSlice) => {
    if (!catalog || !inspection) return;
    clearStagedGridEdits();
    setRowPage(0);
    setColumnPage(0);
    if (gridTx) {
      if (gridTx.recordingError) return;
      const txId = gridTx.transactionId;
      void run(
        async (current) => {
          const result = await client.call("preview_snowflake_grid_transaction_relation", {
            transactionId: txId,
            request: buildPreviewRequest(
              catalog.revision,
              inspection.targetId,
              inspection.revision,
              nextSlice,
            ),
          });
          if (current()) {
            setAppliedSlice(nextSlice);
            setPreview(result);
          }
        },
        setGridError,
        true,
      );
      return;
    }
    setPreview(null);
    void run(async (current) => {
      const result = await client.call("preview_relation", {
        sessionId,
        request: buildPreviewRequest(
          catalog.revision,
          inspection.targetId,
          inspection.revision,
          nextSlice,
        ),
      });
      if (current()) {
        setAppliedSlice(nextSlice);
        setPreview(result);
      }
    });
  };
  const loadPreview = () => {
    if (!catalog || !inspection) return;
    try {
      const slice = compileDraftSlice(0);
      fetchPreviewSlice(slice);
    } catch (err) {
      setGridError(messageOf(err));
    }
  };
  const reloadPendingPreview = () => {
    if (!gridTx || gridTx.recordingError || !catalog || !inspection) return;
    fetchPreviewSlice(appliedSlice);
  };
  const finishGridTransaction = (action: SnowflakeGridFinishAction) => {
    if (!gridTx || gridBusy) return;
    setGridError(null);
    void useStudio
      .getState()
      .finishSnowflakeGridTransaction(sessionId, gridTx.transactionId, action)
      .then(() => {
        if (mounted.current) clearStagedGridEdits();
      })
      .catch((err) => {
        if (mounted.current) setGridError(messageOf(err));
      });
  };
  const loadDefinition = () => {
    const target = catalog?.relations.find((entry) => entry.targetId === selected);
    if (!catalog || target?.availability !== "available" || blocked || !ownerAvailable) return;
    setDefinition(null);
    void run(async (current) => {
      const result = await client.call("define_relation", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name
      )
        throw new Error("Relation definition changed. Refresh relations and try again.");
      setDefinition({ contextKey, value: result });
    });
  };
  const loadConstraints = () => {
    const target = catalog?.relations.find((entry) => entry.targetId === selected);
    if (!catalog || target?.availability !== "available" || blocked || !ownerAvailable) return;
    setConstraints(null);
    setConstraintsError(null);
    void run(async (current) => {
      const result = await client.call("inspect_relation_constraints", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name
      )
        throw new Error("Key constraint target changed. Refresh relations and try again.");
      setConstraints({ contextKey, value: result });
    }, setConstraintsError);
  };
  const target = catalog?.relations.find((entry) => entry.targetId === selected);
  const loadChecks = () => {
    if (
      !catalog ||
      target?.availability !== "available" ||
      blocked ||
      !ownerAvailable ||
      activationEpoch !== generation.current ||
      renderedContext.current !== contextKey ||
      relationContextKey(useStudio.getState(), sessionId, ownerTabId) !== contextKey
    )
      return;
    setChecks(null);
    setChecksError(null);
    void run(async (current) => {
      const result = await client.call("inspect_relation_check_constraints", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name ||
        result.constraints.some(
          (constraint) =>
            constraint.catalog !== catalog.database ||
            constraint.schema !== target.table.schema ||
            constraint.tableName !== target.table.name,
        )
      )
        throw new Error("CHECK constraint target changed. Refresh relations and try again.");
      setChecks({ contextKey, value: result });
    }, setChecksError);
  };
  const loadMappings = () => {
    if (!catalog || target?.availability !== "available" || blocked || !ownerAvailable) return;
    setMappings(null);
    setMappingsError(null);
    void run(async (current) => {
      const result = await client.call("inspect_relation_constraint_mappings", {
        sessionId,
        request: { revision: catalog.revision, targetId: target.targetId },
      });
      if (!current()) return;
      if (
        result.revision !== catalog.revision ||
        result.targetId !== target.targetId ||
        result.database !== catalog.database ||
        result.table.schema !== target.table.schema ||
        result.table.name !== target.table.name
      )
        throw new Error("Key column mapping target changed. Refresh relations and try again.");
      if (
        result.mappings.some(
          (mapping) =>
            mapping.status === "available" &&
            (mapping.columns.length === 0 ||
              (mapping.kind === "foreignKey" &&
                (!mapping.referencedTable ||
                  mapping.columns.some((column) => column.referencedColumnName === null)))),
        )
      )
        throw new Error("Key column mappings are incomplete. Refresh relations and try again.");
      setMappings({ contextKey, value: result });
    }, setMappingsError);
  };
  const selectReferencedTable = (mapping: KeyMapping) => {
    const fresh = useStudio.getState();
    const owner = ownerTabId ? fresh.tabs.find((tab) => tab.id === ownerTabId) : undefined;
    const liveSession = ownerTabId
      ? owner && querySessionForTab(owner, fresh.sessions)
      : Object.values(fresh.sessions).find((entry) => entry.id === sessionId);
    const runtime = fresh.sessionStates[sessionId];
    const retained = retainedSelection.current;
    if (
      !mounted.current ||
      activationEpoch !== generation.current ||
      busy ||
      !liveSession ||
      liveSession.id !== sessionId ||
      !liveSession.capabilities.relationBrowsing ||
      (ownerTabId && owner?.revision !== renderedOwner?.revision) ||
      fresh.sessionTransactionPending[sessionId] ||
      fresh.sessionMetadataRefreshRequired[sessionId] ||
      fresh.sessionDdlCatalogRefreshRequired[sessionId] ||
      fresh.sessionContextSwitching[sessionId] ||
      runtime?.transactionControlAuthority?.status === "rollbackOnly" ||
      renderedContext.current !== contextKey ||
      relationContextKey(fresh, sessionId, ownerTabId) !== contextKey ||
      retained.catalog !== catalog ||
      retained.mappings !== mappings ||
      retained.selected !== selected ||
      retained.sessionId !== sessionId ||
      retained.ownerTabId !== ownerTabId ||
      !catalog ||
      !mappings ||
      !target ||
      mappings.contextKey !== contextKey ||
      mappings.value.revision !== catalog.revision ||
      mappings.value.targetId !== target.targetId ||
      mappings.value.database !== catalog.database ||
      mappings.value.table.schema !== target.table.schema ||
      mappings.value.table.name !== target.table.name ||
      target.database !== catalog.database ||
      target.availability !== "available" ||
      !mappings.value.mappings.includes(mapping) ||
      mapping.columns.length === 0 ||
      mapping.columns.some((column) => column.referencedColumnName === null)
    )
      return;
    const destination = referencedDestination(catalog, mapping)?.target;
    if (!destination) return;
    generation.current++;
    setSearch("");
    setDdlRequest(null);
    setSelected(destination.targetId);
    setInspection(null);
    setPreview(null);
    setDefinition(null);
    setConstraints(null);
    setConstraintsError(null);
    setMappings(null);
    setMappingsError(null);
    setChecks(null);
    setChecksError(null);
    setError(null);
    setRowPage(0);
    setColumnPage(0);
    clearStagedGridEdits();
    resetPreviewSlice();
    relationSelector.current?.focus();
  };
  const selectVirtualRelation = (
    destTable: { schema: string; name: string },
    presetFilters?: DraftFilterRow[],
  ) => {
    if (!catalog || busy) return;
    const matchesInCatalog = catalog.relations.filter(
      (entry) =>
        entry.database === catalog.database &&
        entry.table.schema === destTable.schema &&
        entry.table.name === destTable.name &&
        entry.availability === "available",
    );
    if (matchesInCatalog.length !== 1) return;
    const destination = matchesInCatalog[0];
    generation.current++;
    setSearch("");
    setDdlRequest(null);
    setSelected(destination.targetId);
    setInspection(null);
    setPreview(null);
    setDefinition(null);
    setConstraints(null);
    setConstraintsError(null);
    setMappings(null);
    setMappingsError(null);
    setChecks(null);
    setChecksError(null);
    setError(null);
    setRowPage(0);
    setColumnPage(0);
    clearStagedGridEdits();
    resetPreviewSlice();
    if (presetFilters && presetFilters.length > 0) {
      setDraftFilters(presetFilters.slice(0, MAX_PREVIEW_FILTERS));
    }
    relationSelector.current?.focus();
  };
  const outboundVirtualFks = useMemo(
    () =>
      target && session?.connectionId
        ? virtualForeignKeys.filter(
            (v) =>
              v.connectionId === session.connectionId &&
              v.source.schema === target.table.schema &&
              v.source.name === target.table.name,
          )
        : [],
    [session?.connectionId, target, virtualForeignKeys],
  );
  const inboundVirtualFks = useMemo(
    () =>
      target && session?.connectionId
        ? virtualForeignKeys.filter(
            (v) =>
              v.connectionId === session.connectionId &&
              v.target.schema === target.table.schema &&
              v.target.name === target.table.name,
          )
        : [],
    [session?.connectionId, target, virtualForeignKeys],
  );
  const matches =
    catalog?.relations.filter((entry) =>
      `${entry.table.schema}.${entry.table.name}`
        .toLocaleLowerCase()
        .includes(search.toLocaleLowerCase()),
    ) ?? [];
  const columns =
    preview?.columns.slice(columnPage * PAGE_COLUMNS, (columnPage + 1) * PAGE_COLUMNS) ?? [];
  const rows = preview?.rows.slice(rowPage * PAGE_ROWS, (rowPage + 1) * PAGE_ROWS) ?? [];
  const qualified =
    target && catalog
      ? [catalog.database, target.table.schema, target.table.name]
          .map((part) => `"${part.replaceAll('"', '""')}"`)
          .join(".")
      : "";
  const activeSnapshotId =
    canEditGrid &&
    preview?.editSnapshotId &&
    preview.editSnapshotId !== consumedSnapshotId &&
    !gridTx?.recordingError
      ? preview.editSnapshotId
      : null;
  const gridEditableView = canEditGrid && Boolean(inspection) && Boolean(preview);
  const stagedBuild =
    gridEditableView && inspection
      ? buildSnowflakeGridMutations({
          deletedRows,
          updatedCells,
          insertedRows,
          columns: inspection.columns,
        })
      : { mutations: [], mixedUnknownInsertWarning: null };
  const totalStagedMutations = stagedBuild.mutations.length;

  const setCellOverride = (
    absRowIndex: number,
    colIndex: number,
    nextValue: SnowflakeGridCellInput | null,
  ) => {
    const originalVal = preview?.rows[absRowIndex]?.[colIndex];
    const matchesOriginal =
      nextValue === null ||
      (nextValue.kind === "null" && originalVal === null) ||
      (nextValue.kind === "text" && originalVal !== null && nextValue.value === originalVal);
    setUpdatedCells((prev) => {
      const next = { ...prev };
      const rowMap = { ...(next[absRowIndex] ?? {}) };
      if (matchesOriginal) {
        delete rowMap[colIndex];
      } else {
        rowMap[colIndex] = nextValue;
      }
      if (Object.keys(rowMap).length === 0) {
        delete next[absRowIndex];
      } else {
        next[absRowIndex] = rowMap;
      }
      return next;
    });
  };

  const toggleDeleteRow = (absRowIndex: number) => {
    setDeletedRows((prev) => {
      const next = { ...prev };
      if (next[absRowIndex]) {
        delete next[absRowIndex];
      } else {
        next[absRowIndex] = true;
      }
      return next;
    });
  };

  const handleStageCsv = () => {
    if (!inspection) return;
    setGridError(null);
    try {
      const tableTarget = {
        database: catalog?.database,
        schema: inspection.table.schema,
        name: inspection.table.name,
      };
      const batches = stageCsvInsertBatches({
        raw: csvRawText,
        delimiter: csvDelimiter,
        hasHeader: csvHasHeader,
        nullToken: csvNullToken,
        columns: inspection.columns,
        table: tableTarget,
      });
      const totalRows = batches.reduce((acc, batch) => acc + batch.length, 0);
      if (batches.length === 1 && !csvImportQueue) {
        const staged = batches[0];
        if (totalStagedMutations + staged.length > MAX_SNOWFLAKE_GRID_MUTATIONS) {
          setGridError(
            `Staging ${staged.length} CSV rows would exceed the ${MAX_SNOWFLAKE_GRID_MUTATIONS}-mutation batch limit. Review or discard existing staged edits first.`,
          );
          return;
        }
        if (
          insertedRows.length > 0 &&
          estimateInsertRowsFrameBytes(
            [...insertedRows, ...staged],
            inspection.columns,
            tableTarget,
          ) > SAFE_SNOWFLAKE_BATCH_FRAME_BYTES
        ) {
          setGridError(
            `Staging ${staged.length} CSV rows with existing staged inserts would exceed the Snowflake helper frame limit. Review or discard existing staged edits first.`,
          );
          return;
        }
        setInsertedRows((prev) => [...prev, ...staged]);
        setInsertPage(0);
        setCsvRawText("");
        setCsvPanelOpen(false);
        return;
      }
      if (totalStagedMutations > 0 || csvImportQueue) {
        setGridError(
          `Multi-batch CSV import (${totalRows} rows across ${batches.length} batches) requires an empty staging queue. Review or discard existing staged edits first.`,
        );
        return;
      }
      setGridMode("manual");
      setInsertedRows(batches[0]);
      setInsertPage(0);
      setCsvImportQueue({
        remainingBatches: batches.slice(1),
        batchNumber: 1,
        totalBatches: batches.length,
        totalRows,
      });
      setCsvRawText("");
      setCsvPanelOpen(false);
    } catch (err) {
      setGridError(messageOf(err));
    }
  };

  const prepareGridReview = async () => {
    if (
      !catalog ||
      !inspection ||
      !activeSnapshotId ||
      totalStagedMutations === 0 ||
      totalStagedMutations > MAX_SNOWFLAKE_GRID_MUTATIONS ||
      stagedBuild.mixedUnknownInsertWarning ||
      busy ||
      gridBusy
    )
      return;
    setGridError(null);
    setBusy(true);
    setConsumedSnapshotId(activeSnapshotId);
    try {
      const changePreview = await client.call("prepare_snowflake_grid_plan", {
        request: {
          sessionId,
          transactionId: gridTx?.transactionId ?? null,
          preview: buildPreviewRequest(
            catalog.revision,
            inspection.targetId,
            inspection.revision,
            appliedSlice,
          ),
          snapshotId: activeSnapshotId,
          mutations: stagedBuild.mutations,
          mode: gridTx || csvImportQueue ? "manual" : gridMode,
          rollbackPrior: Boolean(gridTx),
        },
      });
      if (mounted.current) {
        setGridReview(changePreview);
      }
    } catch (err) {
      if (mounted.current) {
        setGridError(messageOf(err));
      }
    } finally {
      if (mounted.current) {
        setBusy(false);
      }
    }
  };

  if (gridReview && profile)
    return (
      <ReviewDialog
        plan={gridReview.plan}
        preview={gridReview}
        profile={profile}
        onClose={() => {
          const planId = gridReview.plan.id;
          setGridReview(null);
          if (dispatchedGridPlanRef.current === planId) {
            dispatchedGridPlanRef.current = null;
            return;
          }
          const epoch = generation.current;
          void client
            .call("cancel_snowflake_grid_plan", { planId })
            .then((restoredSnapshotId) => {
              if (!mounted.current || generation.current !== epoch) return;
              if (restoredSnapshotId) {
                setPreview((current) =>
                  current ? { ...current, editSnapshotId: restoredSnapshotId } : null,
                );
                setConsumedSnapshotId(null);
              }
            })
            .catch(() => {});
        }}
        onExecute={async (approval) => {
          dispatchedGridPlanRef.current = gridReview.plan.id;
          const activeQueue = csvImportQueue;
          const result = await useStudio
            .getState()
            .executeSnowflakeGridPlan(gridReview.plan, approval);
          if (!mounted.current) return;
          if (
            result.outcome === "pending" &&
            result.transactionId &&
            !result.recordingError &&
            activeQueue &&
            activeQueue.remainingBatches.length > 0 &&
            catalog &&
            inspection
          ) {
            const [nextBatch, ...restBatches] = activeQueue.remainingBatches;
            setDeletedRows({});
            setUpdatedCells({});
            setInsertedRows(nextBatch);
            setInsertPage(0);
            setConsumedSnapshotId(null);
            setGridReview(null);
            setGridError(null);
            setCsvImportQueue({
              remainingBatches: restBatches,
              batchNumber: activeQueue.batchNumber + 1,
              totalBatches: activeQueue.totalBatches,
              totalRows: activeQueue.totalRows,
            });
            setPreview((current) => (current ? { ...current, editSnapshotId: undefined } : null));
            const epoch = generation.current;
            setBusy(true);
            try {
              const nextPreview = await client.call("preview_snowflake_grid_transaction_relation", {
                transactionId: result.transactionId,
                request: buildPreviewRequest(
                  catalog.revision,
                  inspection.targetId,
                  inspection.revision,
                  appliedSlice,
                ),
              });
              if (mounted.current && generation.current === epoch) {
                setPreview(nextPreview);
              }
            } catch (err) {
              if (mounted.current && generation.current === epoch) {
                setGridError(messageOf(err));
              }
            } finally {
              if (mounted.current && generation.current === epoch) {
                setBusy(false);
              }
            }
            return;
          }
          clearStagedGridEdits();
          setPreview((current) => (current ? { ...current, editSnapshotId: undefined } : null));
        }}
      />
    );
  if (ddlRequest && profile)
    return (
      <SnowflakeDdlDialog
        sessionId={sessionId}
        ownerTabId={ownerTabId}
        profile={profile}
        request={ddlRequest}
        onClose={onClose}
      />
    );
  if (inspectedCell && preview) {
    const cellCol = preview.columns[inspectedCell.colIndex];
    const inspectedCol = inspection?.columns[inspectedCell.colIndex];
    const rowValues = preview.rows[inspectedCell.absRowIndex];
    if (cellCol && rowValues) {
      const originalValue = rowValues[inspectedCell.colIndex] ?? null;
      const override = updatedCells[inspectedCell.absRowIndex]?.[inspectedCell.colIndex];
      const effective: SnowflakeGridCellInput =
        override ??
        (originalValue === null ? { kind: "null" } : { kind: "text", value: originalValue });
      const cellEditable =
        Boolean(activeSnapshotId) &&
        !deletedRows[inspectedCell.absRowIndex] &&
        Boolean(inspectedCol && isWritableSnowflakeColumn(inspectedCol));
      const effectiveDbType =
        inspectedCol?.fullType || inspectedCol?.dataType || cellCol.databaseType;
      return (
        <SnowflakeCellInspectorModal
          rowNumber={inspectedCell.absRowIndex + 1}
          columnName={cellCol.name}
          columnKind={cellCol.kind}
          databaseType={effectiveDbType}
          nullable={Boolean(inspectedCol?.nullable ?? cellCol.nullable)}
          editable={cellEditable}
          initialInput={effective}
          onApply={(nextInput) => {
            setCellOverride(inspectedCell.absRowIndex, inspectedCell.colIndex, nextInput);
            setInspectedCell(null);
          }}
          onClose={() => setInspectedCell(null)}
        />
      );
    }
  }
  return (
    <Modal
      title="Browse relations"
      subtitle="Visible relations in this console’s current database. Preview up to 500 rows per page (2 MiB) with optional sorting, filtering, and server paging."
      wide
      onClose={onClose}
    >
      <div className="modal-body relation-browser">
        <div className="query-actions">
          <button className="button small" disabled={Boolean(gridTx) || gridBusy} onClick={refresh}>
            Refresh relations
          </button>
          {metadataRefreshRequired && !gridTx && !gridBusy && (
            <button className="button small primary" disabled={busy} onClick={resumeMetadata}>
              Resume session metadata
            </button>
          )}
          <span role="status">
            {busy || gridBusy
              ? "Loading…"
              : catalog
                ? `${catalog.relations.length} visible relations · ${catalog.database}`
                : "Refresh required"}
          </span>
        </div>
        {gridTx && (
          <section
            aria-label="Pending Snowflake grid transaction"
            className="safety-warning"
            style={{ display: "block" }}
          >
            <p role="status">
              <strong>Open manual Snowflake grid transaction:</strong> {gridTx.transactionId}
            </p>
            {gridTx.recordingError && (
              <p className="inline-error" role="alert">
                Outcome recording blocked: {gridTx.recordingError}. Continuation and Commit are
                disabled; Rollback remains available.
              </p>
            )}
            <div className="query-actions">
              <button
                type="button"
                className="button small"
                disabled={
                  busy || gridBusy || Boolean(gridTx.recordingError) || !catalog || !inspection
                }
                onClick={reloadPendingPreview}
              >
                Reload pending preview
              </button>
              <button
                type="button"
                className="button small primary"
                disabled={busy || gridBusy || Boolean(gridTx.recordingError)}
                onClick={() => finishGridTransaction("commit")}
              >
                Commit transaction
              </button>
              <button
                type="button"
                className="button small danger"
                disabled={gridBusy}
                onClick={() => finishGridTransaction("rollback")}
              >
                Rollback transaction
              </button>
            </div>
          </section>
        )}
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        {gridError && (
          <p className="inline-error" role="alert">
            {gridError}
          </p>
        )}
        <label>
          Find relation
          <input
            aria-label="Find relation"
            disabled={busy || Boolean(gridTx) || gridBusy}
            value={search}
            onChange={(event) => {
              generation.current++;
              setSearch(event.target.value);
              setSelected("");
              setInspection(null);
              setPreview(null);
              setDefinition(null);
              setConstraints(null);
              setConstraintsError(null);
              setMappings(null);
              setMappingsError(null);
              setChecks(null);
              setChecksError(null);
              clearStagedGridEdits();
              resetPreviewSlice();
              setBusy(false);
            }}
          />
        </label>
        <label>
          Relation
          <select
            ref={relationSelector}
            aria-label="Relation"
            disabled={busy || !catalog || Boolean(gridTx) || gridBusy}
            value={selected}
            onChange={(event) => {
              generation.current++;
              setSelected(event.target.value);
              setInspection(null);
              setPreview(null);
              setDefinition(null);
              setConstraints(null);
              setConstraintsError(null);
              setMappings(null);
              setMappingsError(null);
              setChecks(null);
              setChecksError(null);
              clearStagedGridEdits();
              resetPreviewSlice();
              setBusy(false);
            }}
          >
            <option value="">Select a relation</option>
            {matches.map((entry) => (
              <option key={entry.targetId} value={entry.targetId}>
                {entry.table.schema}.{entry.table.name} · {entry.tableType}
                {entry.traits.temporary ? " · temporary" : ""}
                {entry.availability !== "available" ? ` · ${entry.availability}` : ""}
              </option>
            ))}
          </select>
        </label>
        {target && (
          <p>
            {target.tableType} ·{" "}
            {Object.entries(target.traits)
              .map(
                ([name, value]) => `${name}: ${value === null ? "unknown" : value ? "yes" : "no"}`,
              )
              .join(" · ")}
          </p>
        )}
        {target && target.availability !== "available" && (
          <p role="note">
            Inspection is unavailable for this target ({target.availability}). Refresh after
            resolving the ambiguity or unsupported metadata.
          </p>
        )}
        <div className="query-actions">
          <button
            className="button small"
            disabled={busy || Boolean(gridTx) || gridBusy || target?.availability !== "available"}
            onClick={inspect}
          >
            Inspect relation
          </button>
          <button
            className="button small"
            disabled={busy || Boolean(gridTx) || gridBusy || !inspection}
            onClick={loadPreview}
          >
            Preview rows
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadDefinition}
          >
            View definition
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadConstraints}
          >
            View key constraints
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadMappings}
          >
            View key column mappings
          </button>
          <button
            className="button small"
            disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
            onClick={loadChecks}
          >
            View CHECK constraints
          </button>
          {session?.capabilities.objectExplorer && session.connectionId && (
            <button
              className="button small"
              disabled={busy || blocked || !ownerAvailable || target?.availability !== "available"}
              onClick={() => {
                if (!target || !session.connectionId) return;
                useStudio.getState().openObject(session.connectionId, {
                  schema: target.table.schema,
                  name: target.table.name,
                  kind:
                    target.tableType === "MATERIALIZED VIEW"
                      ? "materializedView"
                      : target.tableType === "VIEW"
                        ? "view"
                        : "table",
                  identity: null,
                });
                onClose();
              }}
            >
              Inspect object
            </button>
          )}
          {canDdl && (
            <>
              <button
                className="button small"
                disabled={busy || blocked || !catalog}
                onClick={() =>
                  catalog &&
                  setDdlRequest({
                    kind: "createTable",
                    database: catalog.database,
                    schema: useStudio.getState().sessionStates[sessionId]?.activeContext ?? "",
                  })
                }
              >
                Create table…
              </button>
              <button
                className="button small"
                disabled={busy || blocked || !catalog}
                onClick={() =>
                  catalog &&
                  setDdlRequest({
                    kind: "createView",
                    database: catalog.database,
                    schema: useStudio.getState().sessionStates[sessionId]?.activeContext ?? "",
                  })
                }
              >
                Create view…
              </button>
              <button
                className="button small"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  !inspection ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  inspection &&
                  target &&
                  setDdlRequest({
                    kind: "addColumn",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    inspectionRevision: inspection.revision,
                    qualifiedTarget: qualified,
                  })
                }
              >
                Add nullable column…
              </button>
              <button
                className="button small"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  !inspection ||
                  inspection.columns.filter((col) => col.kind === "column").length === 0 ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  inspection &&
                  target &&
                  setDdlRequest({
                    kind: "alterColumnNullability",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    inspectionRevision: inspection.revision,
                    qualifiedTarget: qualified,
                    columns: inspection.columns.filter((col) => col.kind === "column"),
                  })
                }
              >
                Alter column nullability…
              </button>
              <button
                className="button small"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  !inspection ||
                  inspection.columns.filter((col) => col.kind === "column").length === 0 ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  inspection &&
                  target &&
                  setDdlRequest({
                    kind: "renameColumn",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    inspectionRevision: inspection.revision,
                    qualifiedTarget: qualified,
                    columns: inspection.columns.filter((col) => col.kind === "column"),
                  })
                }
              >
                Rename column…
              </button>
              <button
                className="button small danger"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  !inspection ||
                  inspection.columns.filter((col) => col.kind === "column").length <= 1 ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  inspection &&
                  target &&
                  setDdlRequest({
                    kind: "dropColumn",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    inspectionRevision: inspection.revision,
                    qualifiedTarget: qualified,
                    columns: inspection.columns.filter((col) => col.kind === "column"),
                  })
                }
              >
                Drop column…
              </button>
              <button
                className="button small danger"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  target?.availability !== "available" ||
                  target.tableType !== "BASE TABLE"
                }
                onClick={() =>
                  catalog &&
                  target &&
                  setDdlRequest({
                    kind: "dropTable",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    qualifiedTarget: qualified,
                  })
                }
              >
                Drop table…
              </button>
              <button
                className="button small danger"
                disabled={
                  busy ||
                  blocked ||
                  !catalog ||
                  target?.availability !== "available" ||
                  target.tableType !== "VIEW"
                }
                onClick={() =>
                  catalog &&
                  target &&
                  setDdlRequest({
                    kind: "dropView",
                    browseRevision: catalog.revision,
                    targetId: target.targetId,
                    qualifiedTarget: qualified,
                  })
                }
              >
                Drop view…
              </button>
            </>
          )}
        </div>
        {definition &&
          definition.contextKey === contextKey &&
          !blocked &&
          definition.value.targetId === selected &&
          definition.value.revision === catalog?.revision && (
            <section aria-label="Relation definition">
              <h3>Snowflake relation definition</h3>
              <p role="note">
                Snowflake-generated DDL may use CREATE OR REPLACE, normalize types, or omit hidden
                policy and tag information. It is not proof that replay is safe or complete.
              </p>
              <DdlPanel
                connectionId={null}
                ddlStatus={{ status: "available", ddl: definition.value.ddl, truncated: false }}
              />
            </section>
          )}
        {target?.availability === "available" && !blocked && ownerAvailable && (
          <section aria-label="Relation CHECK constraints">
            <h3>CHECK constraints</h3>
            <p role="note">
              These role-visible CHECK clauses are read-only metadata. They do not report
              enforcement or validation, establish engine eligibility, or grant row editing
              authority. Metadata can lag schema changes.
            </p>
            {checksError ? (
              <p className="inline-error" role="alert">
                CHECK constraints are unavailable: {checksError}
              </p>
            ) : checks &&
              checks.contextKey === contextKey &&
              checks.value.targetId === selected &&
              checks.value.revision === catalog?.revision ? (
              checks.value.constraints.length === 0 ? (
                <p>No visible CHECK constraints returned.</p>
              ) : (
                <div className="relation-table-scroll">
                  <table>
                    <caption>CHECK constraint inventory</caption>
                    <thead>
                      <tr>
                        <th>Constraint</th>
                        <th>Table</th>
                        <th>CHECK clause</th>
                      </tr>
                    </thead>
                    <tbody>
                      {checks.value.constraints.map((constraint) => (
                        <tr
                          key={JSON.stringify([
                            constraint.catalog,
                            constraint.schema,
                            constraint.tableName,
                            constraint.name,
                          ])}
                        >
                          <th scope="row">
                            {quotedName(constraint.catalog, constraint.schema, constraint.name)}
                          </th>
                          <td>
                            {quotedName(
                              constraint.catalog,
                              constraint.schema,
                              constraint.tableName,
                            )}
                          </td>
                          <td>
                            <pre>{constraint.checkClause}</pre>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <p>CHECK constraints have not been fetched.</p>
            )}
          </section>
        )}
        {target?.availability === "available" && !blocked && ownerAvailable && (
          <section aria-label="Relation key constraints">
            <h3>Key constraints</h3>
            <p role="note">
              Visible key declarations may not be enforced and do not establish row editing safety.
              This declaration inventory excludes column mappings, CHECK constraints, and NOT NULL
              constraints; column inspection reports nullability.
            </p>
            {constraintsError ? (
              <p className="inline-error" role="alert">
                Key constraints are unavailable: {constraintsError}
              </p>
            ) : constraints &&
              constraints.contextKey === contextKey &&
              constraints.value.targetId === selected &&
              constraints.value.revision === catalog?.revision ? (
              constraints.value.constraints.length === 0 ? (
                <p>No visible declared primary, unique, or foreign keys were returned.</p>
              ) : (
                <div className="relation-table-scroll">
                  <table>
                    <caption>Key constraint inventory</caption>
                    <thead>
                      <tr>
                        <th>Name</th>
                        <th>Type</th>
                        <th>Enforced</th>
                        <th>Deferrable</th>
                        <th>Initially deferred</th>
                        <th>Rely</th>
                        <th>Comment</th>
                        <th>Referenced constraint</th>
                        <th>Match option</th>
                        <th>Update rule</th>
                        <th>Delete rule</th>
                      </tr>
                    </thead>
                    <tbody>
                      {constraints.value.constraints.map((constraint) => (
                        <tr
                          key={JSON.stringify([
                            constraint.catalog,
                            constraint.schema,
                            constraint.name,
                          ])}
                        >
                          <th scope="row">
                            {[constraint.catalog, constraint.schema, constraint.name]
                              .map((part) => `"${part.replaceAll('"', '""')}"`)
                              .join(".")}
                          </th>
                          <td>
                            {constraint.kind === "primaryKey"
                              ? "Primary key"
                              : constraint.kind === "unique"
                                ? "Unique"
                                : "Foreign key"}
                          </td>
                          <td>{constraint.enforced ? "yes" : "no"}</td>
                          <td>{constraint.isDeferrable ? "yes" : "no"}</td>
                          <td>{constraint.initiallyDeferred ? "yes" : "no"}</td>
                          <td>{constraint.rely ? "yes" : "no"}</td>
                          <td>{constraint.comment ?? "—"}</td>
                          <td>
                            {constraint.reference
                              ? [
                                  constraint.reference.catalog,
                                  constraint.reference.schema,
                                  constraint.reference.name,
                                ]
                                  .map((part) => `"${part.replaceAll('"', '""')}"`)
                                  .join(".")
                              : constraint.referenceStatus === "notVisible"
                                ? "Not visible"
                                : "Not applicable"}
                          </td>
                          <td>{constraint.reference?.matchOption ?? "—"}</td>
                          <td>{constraint.reference?.updateRule ?? "—"}</td>
                          <td>{constraint.reference?.deleteRule ?? "—"}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <p>Key constraints have not been fetched.</p>
            )}
          </section>
        )}
        {target?.availability === "available" && !blocked && ownerAvailable && (
          <section aria-label="Relation key column mappings">
            <h3>Key column mappings</h3>
            <p role="note">
              Visible column mappings can lag schema changes and do not establish row editing
              safety. Foreign key columns are paired with referenced columns in local key order.
            </p>
            {mappingsError ? (
              <p className="inline-error" role="alert">
                Key column mappings are unavailable: {mappingsError}
              </p>
            ) : mappings &&
              mappings.contextKey === contextKey &&
              mappings.value.targetId === selected &&
              mappings.value.revision === catalog?.revision ? (
              mappings.value.mappings.length === 0 ? (
                <p>No visible key column mappings were returned.</p>
              ) : (
                <div className="relation-table-scroll">
                  <table>
                    <caption>Key column mapping inventory</caption>
                    <thead>
                      <tr>
                        <th>Constraint</th>
                        <th>Type</th>
                        <th>Status</th>
                        <th>Ordinal</th>
                        <th>Column</th>
                        <th>Referenced table</th>
                        <th>Referenced column</th>
                        <th>Navigation</th>
                      </tr>
                    </thead>
                    <tbody>
                      {mappings.value.mappings.flatMap((mapping, mappingIndex) => {
                        const identity = quotedName(mapping.catalog, mapping.schema, mapping.name);
                        const kind =
                          mapping.kind === "primaryKey"
                            ? "Primary key"
                            : mapping.kind === "unique"
                              ? "Unique"
                              : "Foreign key";
                        const status =
                          mapping.status === "available"
                            ? "Available"
                            : mapping.status === "notVisible"
                              ? "Not visible"
                              : "Unsupported";
                        const destination = referencedDestination(catalog, mapping);
                        const columns =
                          mapping.status === "available"
                            ? [...mapping.columns].sort(
                                (left, right) => left.ordinal - right.ordinal,
                              )
                            : [null];
                        return columns.map((column, index) => (
                          <tr
                            key={JSON.stringify([
                              mapping.catalog,
                              mapping.schema,
                              mapping.name,
                              column?.ordinal,
                            ])}
                          >
                            <th scope="row">{identity}</th>
                            <td>{kind}</td>
                            <td>{status}</td>
                            <td>{column?.ordinal ?? "—"}</td>
                            <td>{column ? quotedName(column.columnName) : "—"}</td>
                            <td>
                              {mapping.status !== "available"
                                ? "—"
                                : mapping.referencedTable
                                  ? quotedName(
                                      mapping.referencedTable.database,
                                      mapping.referencedTable.table.schema,
                                      mapping.referencedTable.table.name,
                                    )
                                  : "Not applicable"}
                            </td>
                            <td>
                              {column?.referencedColumnName == null
                                ? mapping.status === "available" && mapping.kind !== "foreignKey"
                                  ? "Not applicable"
                                  : "—"
                                : quotedName(column.referencedColumnName)}
                            </td>
                            <td>
                              {destination && mapping.referencedTable && index === 0 ? (
                                <>
                                  <button
                                    className="button small"
                                    aria-label={`Select referenced table ${quotedName(mapping.referencedTable.database, mapping.referencedTable.table.schema, mapping.referencedTable.table.name)}`}
                                    aria-describedby={
                                      destination.reason
                                        ? `${navigationId}-${mappingIndex}`
                                        : undefined
                                    }
                                    disabled={
                                      busy ||
                                      !session?.capabilities.relationBrowsing ||
                                      !destination.target
                                    }
                                    onClick={() => selectReferencedTable(mapping)}
                                  >
                                    Select referenced table
                                  </button>
                                  {destination.reason && (
                                    <p id={`${navigationId}-${mappingIndex}`} role="note">
                                      {destination.reason}
                                    </p>
                                  )}
                                </>
                              ) : (
                                "—"
                              )}
                            </td>
                          </tr>
                        ));
                      })}
                    </tbody>
                  </table>
                </div>
              )
            ) : (
              <p>Key column mappings have not been fetched.</p>
            )}
          </section>
        )}
        {target && session?.connectionId && (
          <section aria-label="Virtual relationships">
            <div className="query-actions" style={{ justifyContent: "space-between" }}>
              <h4>Virtual relationships (Joinery local)</h4>
              <button
                type="button"
                className="button small"
                disabled={busy}
                onClick={() => setVirtualFkDialogOpen(true)}
              >
                Add virtual FK…
              </button>
            </div>
            {outboundVirtualFks.length === 0 && inboundVirtualFks.length === 0 ? (
              <p>
                No local virtual foreign keys defined on {target.table.schema}.{target.table.name}.
              </p>
            ) : (
              <div className="relation-table-scroll">
                <table>
                  <caption>Virtual relationships</caption>
                  <thead>
                    <tr>
                      <th scope="col">Name</th>
                      <th scope="col">Direction</th>
                      <th scope="col">Mapping</th>
                      <th scope="col">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {outboundVirtualFks.map((vfk) => {
                      const canNavigate = Boolean(
                        catalog &&
                        catalog.relations.filter(
                          (entry) =>
                            entry.database === catalog.database &&
                            entry.table.schema === vfk.target.schema &&
                            entry.table.name === vfk.target.name &&
                            entry.availability === "available",
                        ).length === 1,
                      );
                      const firstRowFilters =
                        preview && preview.rows.length > 0
                          ? (() => {
                              const filters: DraftFilterRow[] = [];
                              for (let idx = 0; idx < vfk.columns.length; idx++) {
                                const colName = vfk.columns[idx]!;
                                const targetCol = vfk.targetColumns[idx];
                                const colPos = preview.columns.findIndex((c) => c.name === colName);
                                if (colPos < 0 || !targetCol) return undefined;
                                const val = preview.rows[rowPage * PAGE_ROWS]?.[colPos] ?? null;
                                // SQL NULL keys reference nothing; never filter referenced rows by IS NULL.
                                if (val === null) return undefined;
                                filters.push({
                                  id: `vfk-out-${vfk.id}-${idx}`,
                                  column: targetCol,
                                  operator: "equals" as RelationFilterOperator,
                                  value: val,
                                });
                              }
                              return filters.length > 0 ? filters : undefined;
                            })()
                          : undefined;
                      return (
                        <tr key={`out-${vfk.id}`}>
                          <td className="mono">{vfk.name}</td>
                          <td>References</td>
                          <td className="mono">
                            ({vfk.columns.join(", ")}) → {vfk.target.schema}.{vfk.target.name}(
                            {vfk.targetColumns.join(", ")})
                          </td>
                          <td>
                            <button
                              type="button"
                              className="button small"
                              disabled={busy || !canNavigate}
                              onClick={() => selectVirtualRelation(vfk.target, firstRowFilters)}
                            >
                              Select {vfk.target.name}
                            </button>{" "}
                            <button
                              type="button"
                              className="button small danger-text"
                              aria-label={`Delete virtual foreign key ${vfk.name}`}
                              disabled={busy}
                              onClick={() =>
                                void useStudio
                                  .getState()
                                  .deleteVirtualForeignKey(vfk.id)
                                  .catch((err) => setError(messageOf(err)))
                              }
                            >
                              Remove
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                    {inboundVirtualFks.map((vfk) => {
                      const canNavigate = Boolean(
                        catalog &&
                        catalog.relations.filter(
                          (entry) =>
                            entry.database === catalog.database &&
                            entry.table.schema === vfk.source.schema &&
                            entry.table.name === vfk.source.name &&
                            entry.availability === "available",
                        ).length === 1,
                      );
                      const firstRowFilters =
                        preview && preview.rows.length > 0
                          ? (() => {
                              const filters: DraftFilterRow[] = [];
                              for (let idx = 0; idx < vfk.targetColumns.length; idx++) {
                                const targetCol = vfk.targetColumns[idx]!;
                                const sourceCol = vfk.columns[idx];
                                const colPos = preview.columns.findIndex(
                                  (c) => c.name === targetCol,
                                );
                                if (colPos < 0 || !sourceCol) return undefined;
                                const val = preview.rows[rowPage * PAGE_ROWS]?.[colPos] ?? null;
                                if (val === null) return undefined;
                                filters.push({
                                  id: `vfk-in-${vfk.id}-${idx}`,
                                  column: sourceCol,
                                  operator: "equals" as RelationFilterOperator,
                                  value: val,
                                });
                              }
                              return filters.length > 0 ? filters : undefined;
                            })()
                          : undefined;
                      return (
                        <tr key={`in-${vfk.id}`}>
                          <td className="mono">{vfk.name}</td>
                          <td>Referenced by</td>
                          <td className="mono">
                            {vfk.source.schema}.{vfk.source.name}({vfk.columns.join(", ")}) → (
                            {vfk.targetColumns.join(", ")})
                          </td>
                          <td>
                            <button
                              type="button"
                              className="button small"
                              disabled={busy || !canNavigate}
                              onClick={() => selectVirtualRelation(vfk.source, firstRowFilters)}
                            >
                              Select {vfk.source.name}
                            </button>{" "}
                            <button
                              type="button"
                              className="button small danger-text"
                              aria-label={`Delete virtual foreign key ${vfk.name}`}
                              disabled={busy}
                              onClick={() =>
                                void useStudio
                                  .getState()
                                  .deleteVirtualForeignKey(vfk.id)
                                  .catch((err) => setError(messageOf(err)))
                              }
                            >
                              Remove
                            </button>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        )}
        {inspection && (
          <>
            <h3>
              {inspection.database}.{inspection.table.schema}.{inspection.table.name}
            </h3>
            <p>Column metadata does not establish edit eligibility.</p>
            <details open={!preview}>
              <summary>{inspection.columns.length} columns</summary>
              <div className="relation-table-scroll">
                <table>
                  <caption>Column inspection</caption>
                  <thead>
                    <tr>
                      <th>Name</th>
                      <th>Type</th>
                      <th>Nullable</th>
                      <th>Snowflake expression default</th>
                      <th>Descriptor default / Iceberg initial default</th>
                      <th>Write default</th>
                      <th>Identity</th>
                      <th>Kind</th>
                      <th>Comment</th>
                    </tr>
                  </thead>
                  <tbody>
                    {inspection.columns.map((column) => (
                      <tr key={column.name}>
                        <th scope="row">{column.name}</th>
                        <td>{column.fullType}</td>
                        <td>{column.nullable ? "yes" : "no"}</td>
                        <td>{column.defaultExpression ?? "—"}</td>
                        <td>{column.descriptorDefaultExpression ?? "—"}</td>
                        <td>
                          {column.writeDefaultStatus === "notReported"
                            ? "not reported"
                            : column.writeDefaultExpression === null
                              ? "NULL"
                              : column.writeDefaultExpression === ""
                                ? "“”"
                                : column.writeDefaultExpression}
                        </td>
                        <td>
                          {column.identity.isIdentity === null
                            ? "unknown"
                            : column.identity.isIdentity
                              ? `${column.identity.generation ?? "identity"}; start ${column.identity.start ?? "unknown"}; increment ${column.identity.increment ?? "unknown"}`
                              : "no"}
                        </td>
                        <td>{column.kind}</td>
                        <td>{column.comment ?? "—"}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </details>
            <section aria-label="Preview query slice">
              <div className="query-actions">
                <label>
                  Sort{" "}
                  <select
                    aria-label="Preview sort column"
                    disabled={busy || gridBusy || Boolean(csvImportQueue)}
                    value={draftSortColumn}
                    onChange={(e) => setDraftSortColumn(e.target.value)}
                  >
                    <option value="">Unordered sample</option>
                    {inspection.columns.map((col) => (
                      <option key={col.name} value={col.name}>
                        {col.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Direction{" "}
                  <select
                    aria-label="Preview sort direction"
                    disabled={!draftSortColumn || busy || gridBusy || Boolean(csvImportQueue)}
                    value={draftSortDirection}
                    onChange={(e) => setDraftSortDirection(e.target.value as RelationSortDirection)}
                  >
                    <option value="asc">ASC</option>
                    <option value="desc">DESC</option>
                  </select>
                </label>
                <label>
                  Nulls{" "}
                  <select
                    aria-label="Preview sort nulls"
                    disabled={!draftSortColumn || busy || gridBusy || Boolean(csvImportQueue)}
                    value={draftSortNulls}
                    onChange={(e) => setDraftSortNulls(e.target.value as "" | RelationSortNulls)}
                  >
                    <option value="">Default</option>
                    <option value="first">Nulls first</option>
                    <option value="last">Nulls last</option>
                  </select>
                </label>
                <button
                  type="button"
                  className="button small"
                  disabled={
                    busy ||
                    gridBusy ||
                    Boolean(csvImportQueue) ||
                    draftFilters.length >= MAX_PREVIEW_FILTERS ||
                    inspection.columns.length === 0
                  }
                  onClick={() =>
                    setDraftFilters((prev) => [
                      ...prev,
                      {
                        id: crypto.randomUUID(),
                        column: inspection.columns[0]?.name ?? "",
                        operator: "equals",
                        value: "",
                      },
                    ])
                  }
                >
                  Add filter
                </button>
                <button
                  type="button"
                  className="button small"
                  disabled={
                    busy || gridBusy || Boolean(csvImportQueue) || Boolean(gridTx?.recordingError)
                  }
                  onClick={loadPreview}
                >
                  Apply sort & filters
                </button>
                {(Boolean(draftSortColumn) ||
                  draftFilters.length > 0 ||
                  appliedSlice.offset > 0 ||
                  Boolean(appliedSlice.sort) ||
                  appliedSlice.filters.length > 0) && (
                  <button
                    type="button"
                    className="button small"
                    disabled={busy || gridBusy || Boolean(csvImportQueue)}
                    onClick={() => {
                      resetPreviewSlice();
                      if (preview) {
                        fetchPreviewSlice(DEFAULT_PREVIEW_SLICE);
                      }
                    }}
                  >
                    Reset slice
                  </button>
                )}
              </div>
              {draftFilters.length > 0 && (
                <div className="filter-builder">
                  {draftFilters.map((filterRow, filterIdx) => {
                    const needsValue =
                      filterRow.operator === "equals" ||
                      filterRow.operator === "notEquals" ||
                      filterRow.operator === "contains";
                    return (
                      <div key={filterRow.id} className="query-actions">
                        <select
                          aria-label={`Filter ${filterIdx + 1} column`}
                          disabled={busy || gridBusy || Boolean(csvImportQueue)}
                          value={filterRow.column}
                          onChange={(e) =>
                            setDraftFilters((prev) =>
                              prev.map((entry) =>
                                entry.id === filterRow.id
                                  ? { ...entry, column: e.target.value }
                                  : entry,
                              ),
                            )
                          }
                        >
                          {inspection.columns.map((col) => (
                            <option key={col.name} value={col.name}>
                              {col.name}
                            </option>
                          ))}
                        </select>
                        <select
                          aria-label={`Filter ${filterIdx + 1} operator`}
                          disabled={busy || gridBusy || Boolean(csvImportQueue)}
                          value={filterRow.operator}
                          onChange={(e) =>
                            setDraftFilters((prev) =>
                              prev.map((entry) =>
                                entry.id === filterRow.id
                                  ? {
                                      ...entry,
                                      operator: e.target.value as RelationFilterOperator,
                                    }
                                  : entry,
                              ),
                            )
                          }
                        >
                          <option value="equals">=</option>
                          <option value="notEquals">!=</option>
                          <option value="contains">contains (case-insensitive)</option>
                          <option value="isNull">IS NULL</option>
                          <option value="isNotNull">IS NOT NULL</option>
                        </select>
                        {needsValue && (
                          <input
                            aria-label={`Filter ${filterIdx + 1} value`}
                            disabled={busy || gridBusy || Boolean(csvImportQueue)}
                            placeholder="Filter value"
                            value={filterRow.value}
                            onChange={(e) =>
                              setDraftFilters((prev) =>
                                prev.map((entry) =>
                                  entry.id === filterRow.id
                                    ? { ...entry, value: e.target.value }
                                    : entry,
                                ),
                              )
                            }
                          />
                        )}
                        <button
                          type="button"
                          className="button small"
                          aria-label={`Remove filter ${filterIdx + 1}`}
                          disabled={busy || gridBusy || Boolean(csvImportQueue)}
                          onClick={() =>
                            setDraftFilters((prev) =>
                              prev.filter((entry) => entry.id !== filterRow.id),
                            )
                          }
                        >
                          Remove
                        </button>
                      </div>
                    );
                  })}
                </div>
              )}
            </section>
          </>
        )}
        {preview && (
          <>
            <p role="status">
              {preview.rows.length} retained rows
              {appliedSlice.offset > 0
                ? ` (server offset ${appliedSlice.offset.toLocaleString()})`
                : ""}
              {preview.truncated ? " · Preview limit reached" : ""}. NULL is shown as ∅; an empty
              string as “”.{" "}
              {appliedSlice.offset === 0 && !appliedSlice.sort && appliedSlice.filters.length === 0
                ? "This unordered sample has no continuation."
                : [
                    appliedSlice.sort
                      ? `Sorted by ${appliedSlice.sort.column} ${appliedSlice.sort.direction.toUpperCase()}`
                      : "Unordered",
                    appliedSlice.filters.length > 0
                      ? `${appliedSlice.filters.length} filter${appliedSlice.filters.length === 1 ? "" : "s"} active`
                      : null,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
            </p>
            {gridEditableView && inspection && (
              <section aria-label="Snowflake grid editing controls">
                {csvImportQueue && (
                  <p role="status" className="safety-warning" style={{ display: "block" }}>
                    <strong>Multi-batch CSV import:</strong> Batch {csvImportQueue.batchNumber} of{" "}
                    {csvImportQueue.totalBatches} staged ({insertedRows.length} rows in this batch ·{" "}
                    {csvImportQueue.totalRows.toLocaleString()} total CSV rows in manual
                    transaction). Review and apply each batch, then Commit or Rollback transaction.
                  </p>
                )}
                {!activeSnapshotId ? (
                  <p role="note">
                    {gridTx?.recordingError
                      ? "Outcome recording is blocked on this manual transaction. Roll back before editing further."
                      : gridTx
                        ? "Preview edit snapshot was consumed. Click Reload pending preview above to inspect uncommitted rows and stage continuation edits."
                        : !preview.editSnapshotId
                          ? "Generated grid editing is unavailable for this relation or column shape (supported on standard, temporary, and transient base tables up to 1,000 columns across text, NUMBER/DECIMAL, FLOAT, BOOLEAN, DATE, TIME, TIMESTAMP_NTZ/LTZ/TZ, BINARY, and VARIANT/OBJECT/ARRAY without duplicate object keys)."
                          : "Preview edit snapshot was consumed during plan preparation. Click Preview rows to obtain a fresh snapshot."}
                  </p>
                ) : (
                  <>
                    <div className="query-actions">
                      <label>
                        Mode{" "}
                        <select
                          aria-label="Grid transaction mode"
                          disabled={Boolean(gridTx) || Boolean(csvImportQueue) || busy || gridBusy}
                          value={gridTx || csvImportQueue ? "manual" : gridMode}
                          onChange={(e) => setGridMode(e.target.value as SnowflakeGridMode)}
                        >
                          <option value="automatic">Auto-commit batch</option>
                          <option value="manual">Manual transaction</option>
                        </select>
                      </label>
                      <button
                        type="button"
                        className="button small"
                        disabled={
                          busy ||
                          gridBusy ||
                          Boolean(csvImportQueue) ||
                          totalStagedMutations >= MAX_SNOWFLAKE_GRID_MUTATIONS
                        }
                        onClick={() =>
                          setInsertedRows((prev) => [
                            ...prev,
                            createBlankInsertRow(inspection.columns),
                          ])
                        }
                      >
                        Add row
                      </button>
                      <button
                        type="button"
                        className="button small"
                        disabled={busy || gridBusy || Boolean(csvImportQueue)}
                        onClick={() => setCsvPanelOpen((open) => !open)}
                      >
                        {csvPanelOpen ? "Hide CSV import" : "Import CSV rows…"}
                      </button>
                      {totalStagedMutations > 0 && (
                        <button
                          type="button"
                          className="button small"
                          disabled={busy || gridBusy}
                          onClick={clearStagedGridEdits}
                        >
                          {csvImportQueue ? "Discard CSV import queue" : "Discard staged edits"}
                        </button>
                      )}
                      <button
                        type="button"
                        className="button small primary"
                        disabled={
                          busy ||
                          gridBusy ||
                          totalStagedMutations === 0 ||
                          totalStagedMutations > MAX_SNOWFLAKE_GRID_MUTATIONS ||
                          Boolean(stagedBuild.mixedUnknownInsertWarning)
                        }
                        onClick={() => void prepareGridReview()}
                      >
                        {csvImportQueue
                          ? `Review batch ${csvImportQueue.batchNumber}/${csvImportQueue.totalBatches} (${totalStagedMutations} rows)…`
                          : `Review ${totalStagedMutations} change${totalStagedMutations === 1 ? "" : "s"}…`}
                      </button>
                    </div>
                    {stagedBuild.mixedUnknownInsertWarning && (
                      <p className="inline-error" role="alert">
                        {stagedBuild.mixedUnknownInsertWarning}
                      </p>
                    )}
                    {csvPanelOpen && (
                      <section aria-label="Stage CSV rows">
                        <div className="query-actions">
                          <label>
                            Delimiter{" "}
                            <select
                              aria-label="CSV delimiter"
                              value={csvDelimiter}
                              onChange={(e) => setCsvDelimiter(e.target.value as CsvFieldDelimiter)}
                            >
                              <option value=",">Comma (,)</option>
                              <option value={"\t"}>Tab</option>
                              <option value=";">Semicolon (;)</option>
                              <option value="|">Pipe (|)</option>
                            </select>
                          </label>
                          <label>
                            <input
                              type="checkbox"
                              aria-label="First CSV row is header"
                              checked={csvHasHeader}
                              onChange={(e) => setCsvHasHeader(e.target.checked)}
                            />{" "}
                            First row is header
                          </label>
                          <label>
                            NULL token{" "}
                            <input
                              aria-label="CSV NULL token"
                              value={csvNullToken}
                              onChange={(e) => setCsvNullToken(e.target.value)}
                              style={{ width: "5rem" }}
                            />
                          </label>
                          <label>
                            Load CSV file{" "}
                            <input
                              type="file"
                              aria-label="Load CSV file"
                              accept=".csv,.tsv,text/csv,text/tab-separated-values,text/plain"
                              onChange={(e) => {
                                const file = e.target.files?.[0];
                                if (!file) return;
                                void file
                                  .text()
                                  .then((content) => {
                                    if (mounted.current) setCsvRawText(content);
                                  })
                                  .catch((err) => {
                                    if (mounted.current) setGridError(messageOf(err));
                                  });
                              }}
                            />
                          </label>
                        </div>
                        <label>
                          CSV content
                          <textarea
                            aria-label="CSV rows to stage"
                            rows={4}
                            value={csvRawText}
                            onChange={(e) => setCsvRawText(e.target.value)}
                            placeholder="Paste CSV rows (up to 100,000 rows; >500 rows run as reviewed 500-row batches in one manual transaction)"
                          />
                        </label>
                        <div className="query-actions">
                          <button
                            type="button"
                            className="button small primary"
                            disabled={!csvRawText.trim()}
                            onClick={handleStageCsv}
                          >
                            Stage CSV rows
                          </button>
                        </div>
                      </section>
                    )}
                    {insertedRows.length > 0 && (
                      <section aria-label="Staged insert rows">
                        <h4>Staged inserts ({insertedRows.length})</h4>
                        {insertedRows.length > INSERT_PAGE_ROWS && (
                          <div className="query-actions">
                            <button
                              type="button"
                              className="button small"
                              disabled={insertPage === 0}
                              onClick={() => setInsertPage((p) => Math.max(0, p - 1))}
                            >
                              Previous staged inserts
                            </button>
                            <span>
                              Staged rows {insertPage * INSERT_PAGE_ROWS + 1}–
                              {Math.min((insertPage + 1) * INSERT_PAGE_ROWS, insertedRows.length)}{" "}
                              of {insertedRows.length}
                            </span>
                            <button
                              type="button"
                              className="button small"
                              disabled={(insertPage + 1) * INSERT_PAGE_ROWS >= insertedRows.length}
                              onClick={() => setInsertPage((p) => p + 1)}
                            >
                              Next staged inserts
                            </button>
                          </div>
                        )}
                        <div className="relation-table-scroll">
                          <table>
                            <caption>Staged insert rows</caption>
                            <thead>
                              <tr>
                                <th>Action</th>
                                {inspection.columns.map((col, colIdx) =>
                                  isWritableSnowflakeColumn(col) ? (
                                    <th key={colIdx}>{col.name}</th>
                                  ) : null,
                                )}
                              </tr>
                            </thead>
                            <tbody>
                              {insertedRows
                                .slice(
                                  insertPage * INSERT_PAGE_ROWS,
                                  (insertPage + 1) * INSERT_PAGE_ROWS,
                                )
                                .map((draftRow, localIdx) => {
                                  const draftIdx = insertPage * INSERT_PAGE_ROWS + localIdx;
                                  return (
                                    <tr key={draftRow.id}>
                                      <td>
                                        <button
                                          type="button"
                                          className="button small"
                                          aria-label={`Remove insert row ${draftIdx + 1}`}
                                          disabled={Boolean(csvImportQueue)}
                                          onClick={() =>
                                            setInsertedRows((prev) =>
                                              prev.filter((entry) => entry.id !== draftRow.id),
                                            )
                                          }
                                        >
                                          Remove
                                        </button>
                                      </td>
                                      {inspection.columns.map((col, colIdx) => {
                                        if (!isWritableSnowflakeColumn(col)) return null;
                                        const cell = draftRow.cells[colIdx] ?? {
                                          kind: "text" as const,
                                          value: "",
                                        };
                                        const allowDefault = canColumnDefaultOnInsert(col);
                                        return (
                                          <td key={colIdx}>
                                            <select
                                              aria-label={`Insert row ${draftIdx + 1} ${col.name} mode`}
                                              disabled={Boolean(csvImportQueue)}
                                              value={cell.kind}
                                              onChange={(e) => {
                                                const mode = e.target
                                                  .value as SnowflakeGridCellInput["kind"];
                                                const nextCell: SnowflakeGridCellInput =
                                                  mode === "default"
                                                    ? { kind: "default" }
                                                    : mode === "null"
                                                      ? { kind: "null" }
                                                      : {
                                                          kind: "text",
                                                          value:
                                                            cell.kind === "text"
                                                              ? cell.value
                                                              : col.resultKind === "boolean"
                                                                ? "false"
                                                                : "",
                                                        };
                                                setInsertedRows((prev) =>
                                                  prev.map((entry) =>
                                                    entry.id === draftRow.id
                                                      ? {
                                                          ...entry,
                                                          cells: {
                                                            ...entry.cells,
                                                            [colIdx]: nextCell,
                                                          },
                                                        }
                                                      : entry,
                                                  ),
                                                );
                                              }}
                                            >
                                              {allowDefault && (
                                                <option value="default">DEFAULT</option>
                                              )}
                                              {col.nullable && <option value="null">NULL</option>}
                                              <option value="text">Value</option>
                                            </select>
                                            {cell.kind === "text" && (
                                              <input
                                                aria-label={`Insert row ${draftIdx + 1} ${col.name} value`}
                                                disabled={Boolean(csvImportQueue)}
                                                value={cell.value}
                                                onChange={(e) => {
                                                  const nextVal = e.target.value;
                                                  setInsertedRows((prev) =>
                                                    prev.map((entry) =>
                                                      entry.id === draftRow.id
                                                        ? {
                                                            ...entry,
                                                            cells: {
                                                              ...entry.cells,
                                                              [colIdx]: {
                                                                kind: "text",
                                                                value: nextVal,
                                                              },
                                                            },
                                                          }
                                                        : entry,
                                                    ),
                                                  );
                                                }}
                                              />
                                            )}
                                          </td>
                                        );
                                      })}
                                    </tr>
                                  );
                                })}
                            </tbody>
                          </table>
                        </div>
                      </section>
                    )}
                  </>
                )}
              </section>
            )}
            <div className="query-actions">
              <button
                type="button"
                aria-label="Previous server page"
                disabled={
                  appliedSlice.offset === 0 ||
                  busy ||
                  gridBusy ||
                  Boolean(csvImportQueue) ||
                  Boolean(gridTx?.recordingError)
                }
                onClick={() =>
                  fetchPreviewSlice({
                    ...appliedSlice,
                    offset: Math.max(0, appliedSlice.offset - SERVER_PAGE_ROWS),
                  })
                }
              >
                Previous 500
              </button>
              <button
                type="button"
                aria-label="Next server page"
                disabled={
                  !preview.truncated ||
                  appliedSlice.offset + SERVER_PAGE_ROWS > MAX_SERVER_OFFSET ||
                  busy ||
                  gridBusy ||
                  Boolean(csvImportQueue) ||
                  Boolean(gridTx?.recordingError)
                }
                onClick={() =>
                  fetchPreviewSlice({
                    ...appliedSlice,
                    offset: appliedSlice.offset + SERVER_PAGE_ROWS,
                  })
                }
              >
                Next 500
              </button>
              <button disabled={rowPage === 0} onClick={() => setRowPage(rowPage - 1)}>
                Previous rows
              </button>
              <span>
                Rows {preview.rows.length ? rowPage * PAGE_ROWS + 1 : 0}–
                {Math.min((rowPage + 1) * PAGE_ROWS, preview.rows.length)}
              </span>
              <button
                disabled={(rowPage + 1) * PAGE_ROWS >= preview.rows.length}
                onClick={() => setRowPage(rowPage + 1)}
              >
                Next rows
              </button>
              <button disabled={columnPage === 0} onClick={() => setColumnPage(columnPage - 1)}>
                Previous columns
              </button>
              <span>
                Columns {columnPage * PAGE_COLUMNS + 1}–
                {Math.min((columnPage + 1) * PAGE_COLUMNS, preview.columns.length)}
              </span>
              <button
                disabled={(columnPage + 1) * PAGE_COLUMNS >= preview.columns.length}
                onClick={() => setColumnPage(columnPage + 1)}
              >
                Next columns
              </button>
              <button
                type="button"
                aria-pressed={previewTransposed}
                onClick={() => setPreviewTransposed((prev) => !prev)}
              >
                {previewTransposed ? "Table view" : "Transpose"}
              </button>
            </div>
            <div className="relation-table-scroll">
              {previewTransposed ? (
                <table aria-label="Transposed Snowflake preview">
                  <caption>Transposed Snowflake preview</caption>
                  <thead>
                    <tr>
                      <th scope="col">Column</th>
                      <th scope="col">Type</th>
                      {rows.map((_, index) => {
                        const absRowIndex = rowPage * PAGE_ROWS + index;
                        return <th key={absRowIndex}>{`Row ${absRowIndex + 1}`}</th>;
                      })}
                    </tr>
                  </thead>
                  <tbody>
                    {columns.map((column, i) => {
                      const colIndex = columnPage * PAGE_COLUMNS + i;
                      const inspectedCol = inspection?.columns[colIndex];
                      const effectiveDbType =
                        inspectedCol?.fullType || inspectedCol?.dataType || column.databaseType;
                      return (
                        <tr key={column.id}>
                          <th scope="row" className="mono">
                            {column.name}
                          </th>
                          <td className="mono">{effectiveDbType}</td>
                          {rows.map((row, index) => {
                            const absRowIndex = rowPage * PAGE_ROWS + index;
                            const override = updatedCells[absRowIndex]?.[colIndex];
                            const rawValue =
                              override === undefined
                                ? row[colIndex]
                                : override.kind === "text"
                                  ? override.value
                                  : null;
                            return (
                              <td
                                key={absRowIndex}
                                className="mono"
                                onDoubleClick={() => setInspectedCell({ absRowIndex, colIndex })}
                              >
                                {override?.kind === "default" ? (
                                  <span aria-label="DEFAULT">DEFAULT</span>
                                ) : rawValue === null ? (
                                  <span aria-label="NULL">∅</span>
                                ) : rawValue === "" ? (
                                  <span aria-label="Empty string">“”</span>
                                ) : (
                                  rawValue
                                )}
                              </td>
                            );
                          })}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              ) : (
                <table>
                  <caption>
                    {activeSnapshotId ? "Editable Snowflake preview" : "Read-only preview"}
                  </caption>
                  <thead>
                    <tr>
                      {activeSnapshotId && <th>Row action</th>}
                      {columns.map((column) => (
                        <th key={column.id}>{column.name}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {rows.map((row, index) => {
                      const absRowIndex = rowPage * PAGE_ROWS + index;
                      const rowDeleted = Boolean(deletedRows[absRowIndex]);
                      return (
                        <tr key={absRowIndex}>
                          {activeSnapshotId && (
                            <td>
                              <button
                                type="button"
                                className="button small"
                                aria-label={
                                  rowDeleted
                                    ? `Undo delete row ${absRowIndex + 1}`
                                    : `Mark row ${absRowIndex + 1} for delete`
                                }
                                onClick={() => toggleDeleteRow(absRowIndex)}
                              >
                                {rowDeleted ? "Undo delete" : "Delete"}
                              </button>
                              {inspection && (
                                <>
                                  {" "}
                                  <button
                                    type="button"
                                    className="button small"
                                    aria-label={`Clone row ${absRowIndex + 1}`}
                                    disabled={
                                      busy ||
                                      gridBusy ||
                                      Boolean(csvImportQueue) ||
                                      totalStagedMutations >= MAX_SNOWFLAKE_GRID_MUTATIONS
                                    }
                                    onClick={() => {
                                      const cloned = cloneSnowflakePreviewRow(
                                        inspection.columns,
                                        row,
                                        updatedCells[absRowIndex],
                                      );
                                      if (
                                        estimateInsertRowsFrameBytes(
                                          [...insertedRows, cloned],
                                          inspection.columns,
                                        ) > SAFE_SNOWFLAKE_BATCH_FRAME_BYTES
                                      ) {
                                        setError(
                                          "Staged Snowflake inserts reached the single-batch frame limit; review and apply current edits first.",
                                        );
                                        return;
                                      }
                                      setInsertedRows((prev) => [...prev, cloned]);
                                    }}
                                  >
                                    Clone
                                  </button>
                                </>
                              )}
                            </td>
                          )}
                          {columns.map((column, i) => {
                            const colIndex = columnPage * PAGE_COLUMNS + i;
                            const value = row[colIndex];
                            const inspectedCol = inspection?.columns[colIndex];
                            const effectiveDbType =
                              inspectedCol?.fullType ||
                              inspectedCol?.dataType ||
                              column.databaseType;
                            const classifierCol = {
                              kind: column.kind,
                              databaseType: effectiveDbType,
                            };
                            const richInspectorCol =
                              isJsonColumn(classifierCol) ||
                              isBinaryColumn(classifierCol) ||
                              isSpatialColumn(classifierCol);
                            const cellEditable =
                              Boolean(activeSnapshotId) &&
                              !rowDeleted &&
                              Boolean(inspectedCol && isWritableSnowflakeColumn(inspectedCol));
                            if (cellEditable && inspectedCol) {
                              const override = updatedCells[absRowIndex]?.[colIndex];
                              const effective: SnowflakeGridCellInput =
                                override ??
                                (value === null ? { kind: "null" } : { kind: "text", value });
                              const textForCheck =
                                effective.kind === "text" ? effective.value : null;
                              const showInspect =
                                richInspectorCol ||
                                (textForCheck !== null &&
                                  (textForCheck.length > 64 || textForCheck.includes("\n")));
                              return (
                                <td
                                  key={column.id}
                                  onDoubleClick={() => setInspectedCell({ absRowIndex, colIndex })}
                                >
                                  <input
                                    aria-label={`Row ${absRowIndex + 1} ${column.name}`}
                                    disabled={effective.kind === "null"}
                                    placeholder={effective.kind === "null" ? "NULL" : ""}
                                    value={effective.kind === "text" ? effective.value : ""}
                                    onChange={(e) =>
                                      setCellOverride(absRowIndex, colIndex, {
                                        kind: "text",
                                        value: e.target.value,
                                      })
                                    }
                                  />
                                  {showInspect && (
                                    <button
                                      type="button"
                                      className="button small"
                                      aria-label={`Inspect row ${absRowIndex + 1} ${column.name}`}
                                      onClick={() => setInspectedCell({ absRowIndex, colIndex })}
                                    >
                                      Inspect
                                    </button>
                                  )}
                                  {inspectedCol.nullable && (
                                    <button
                                      type="button"
                                      className="button small"
                                      aria-label={`Toggle NULL for row ${absRowIndex + 1} ${column.name}`}
                                      onClick={() =>
                                        setCellOverride(
                                          absRowIndex,
                                          colIndex,
                                          effective.kind === "null"
                                            ? { kind: "text", value: value ?? "" }
                                            : { kind: "null" },
                                        )
                                      }
                                    >
                                      {effective.kind === "null" ? "Text" : "NULL"}
                                    </button>
                                  )}
                                  {override && (
                                    <button
                                      type="button"
                                      className="button small"
                                      aria-label={`Revert row ${absRowIndex + 1} ${column.name}`}
                                      onClick={() => setCellOverride(absRowIndex, colIndex, null)}
                                    >
                                      Revert
                                    </button>
                                  )}
                                </td>
                              );
                            }
                            const showInspectReadOnly =
                              value !== null &&
                              (richInspectorCol || value.length > 64 || value.includes("\n"));
                            return (
                              <td
                                key={column.id}
                                onDoubleClick={() => setInspectedCell({ absRowIndex, colIndex })}
                              >
                                {value === null ? (
                                  <span aria-label="NULL">∅</span>
                                ) : value === "" ? (
                                  <span aria-label="Empty string">“”</span>
                                ) : (
                                  value
                                )}
                                {showInspectReadOnly && (
                                  <>
                                    {" "}
                                    <button
                                      type="button"
                                      className="button small"
                                      aria-label={`Inspect row ${absRowIndex + 1} ${column.name}`}
                                      onClick={() => setInspectedCell({ absRowIndex, colIndex })}
                                    >
                                      Inspect
                                    </button>
                                  </>
                                )}
                              </td>
                            );
                          })}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}
            </div>
          </>
        )}
      </div>
      {virtualFkDialogOpen && target && session?.connectionId && (
        <VirtualForeignKeyDialog
          connectionId={session.connectionId}
          initialSource={target.table}
          sourceColumns={inspection?.columns.map((c) => c.name) ?? []}
          knownTables={
            catalog?.relations.filter((r) => r.availability === "available").map((r) => r.table) ??
            []
          }
          onClose={() => setVirtualFkDialogOpen(false)}
        />
      )}
    </Modal>
  );
}

function SnowflakeCellInspectorModal({
  rowNumber,
  columnName,
  columnKind,
  databaseType,
  nullable,
  editable,
  initialInput,
  onApply,
  onClose,
}: {
  rowNumber: number;
  columnName: string;
  columnKind: RelationPreview["columns"][number]["kind"];
  databaseType: string;
  nullable: boolean;
  editable: boolean;
  initialInput: SnowflakeGridCellInput;
  onApply: (next: SnowflakeGridCellInput) => void;
  onClose: () => void;
}) {
  const notify = useStudio((s) => s.notify);
  const classifier = { kind: columnKind, databaseType };
  const viewerKind = isBinaryColumn(classifier)
    ? "binary"
    : isSpatialColumn(classifier)
      ? "spatial"
      : isJsonColumn(classifier)
        ? "json"
        : null;

  const [isNull, setIsNull] = useState(initialInput.kind === "null");
  const [value, setValue] = useState(initialInput.kind === "text" ? initialInput.value : "");
  const [viewMode, setViewMode] = useState<"formatted" | "tree" | "raw">(() => {
    if (initialInput.kind === "null") return "raw";
    if (viewerKind === "binary" || viewerKind === "spatial") return "formatted";
    if (viewerKind === "json") {
      const trimmed = initialInput.kind === "text" ? initialInput.value.trim() : "";
      return trimmed.length > 0 && buildJsonTree(trimmed, columnName).ok ? "tree" : "raw";
    }
    return "raw";
  });
  const [error, setError] = useState<string | null>(null);

  const showViewerTabs = viewerKind !== null && !isNull;

  const binaryPreview = useMemo(() => {
    if (viewerKind !== "binary" || isNull) return null;
    const preview = decodeBinaryPreview(
      value,
      MAX_HEX_PREVIEW_BYTES,
      isRawHexBinaryType(databaseType),
    );
    return { ...preview, dump: formatHexDump(preview.bytes) };
  }, [viewerKind, value, isNull, databaseType]);

  const spatialPreview = useMemo(() => {
    if (viewerKind !== "spatial" || viewMode !== "formatted" || isNull) return null;
    return parseSpatialPreview(value);
  }, [viewerKind, viewMode, value, isNull]);

  const jsonPreview = useMemo(() => {
    if (viewerKind !== "json" || viewMode !== "formatted" || isNull) return null;
    return formatJsonLossless(value);
  }, [viewerKind, viewMode, value, isNull]);

  const jsonTree = useMemo(() => {
    if (viewerKind !== "json" || viewMode !== "tree" || isNull) return null;
    return buildJsonTree(value, columnName);
  }, [viewerKind, viewMode, value, isNull, columnName]);

  const handleFormatJson = () => {
    const formatted = formatJsonLossless(value);
    if (!formatted.ok) {
      setError(formatted.error);
      setViewMode("raw");
      return;
    }
    setError(null);
    setValue(formatted.formatted);
    setViewMode("raw");
  };

  const handleSubmit = () => {
    setError(null);
    if (!isNull && viewerKind === "json") {
      try {
        JSON.parse(value);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Enter valid JSON");
        return;
      }
    }
    if (!isNull && viewerKind === "spatial") {
      const parsed = parseSpatialPreview(value);
      if (!parsed.ok) {
        setError(parsed.error);
        return;
      }
      const isGeography = databaseType.trim().toUpperCase().startsWith("GEOGRAPHY");
      if (isGeography && parsed.srid !== null && parsed.srid !== 4326) {
        setError("Snowflake GEOGRAPHY requires WGS84 SRID 4326");
        return;
      }
      if (
        isGeography &&
        parsed.bbox &&
        (parsed.bbox[0] < -180 ||
          parsed.bbox[2] > 180 ||
          parsed.bbox[1] < -90 ||
          parsed.bbox[3] > 90)
      ) {
        setError(
          "GEOGRAPHY coordinates must fall within longitude [-180, 180] and latitude [-90, 90]",
        );
        return;
      }
    }
    onApply(isNull ? { kind: "null" } : { kind: "text", value });
  };

  const formattedTabLabel =
    viewerKind === "binary" ? "Hex / ASCII" : viewerKind === "spatial" ? "Spatial" : "Formatted";

  return (
    <Modal
      title={`${editable ? "Inspect & stage" : "Inspect"} ${columnName}`}
      subtitle={`Row ${rowNumber} · ${databaseType}`}
      wide
      onClose={onClose}
    >
      <div className="modal-body" data-testid="snowflake-cell-inspector">
        {editable && nullable && (
          <label className="checkbox-label">
            <input type="checkbox" checked={isNull} onChange={(e) => setIsNull(e.target.checked)} />
            Set to NULL
          </label>
        )}
        {showViewerTabs && (
          <div className="value-viewer-toolbar">
            <div
              className="value-viewer-tabs"
              role="tablist"
              aria-label="Value view"
              onKeyDown={tabNavigation}
            >
              {viewerKind === "json" && (
                <button
                  type="button"
                  role="tab"
                  aria-selected={viewMode === "tree"}
                  tabIndex={viewMode === "tree" ? 0 : -1}
                  className={`value-viewer-tab ${viewMode === "tree" ? "active" : ""}`}
                  onClick={() => setViewMode("tree")}
                >
                  Tree (:path)
                </button>
              )}
              <button
                type="button"
                role="tab"
                aria-selected={viewMode === "formatted"}
                tabIndex={viewMode === "formatted" ? 0 : -1}
                className={`value-viewer-tab ${viewMode === "formatted" ? "active" : ""}`}
                onClick={() => setViewMode("formatted")}
              >
                {formattedTabLabel}
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={viewMode === "raw"}
                tabIndex={viewMode === "raw" ? 0 : -1}
                className={`value-viewer-tab ${viewMode === "raw" ? "active" : ""}`}
                onClick={() => setViewMode("raw")}
              >
                Raw
              </button>
            </div>
            <div className="value-viewer-meta">
              {viewerKind === "binary" && binaryPreview && (
                <span>
                  {binaryPreview.totalBytes.toLocaleString()}{" "}
                  {binaryPreview.totalBytes === 1 ? "byte" : "bytes"}
                  {binaryPreview.truncated
                    ? ` · showing first ${MAX_HEX_PREVIEW_BYTES.toLocaleString()} bytes`
                    : ""}
                </span>
              )}
              {viewerKind === "json" && viewMode === "tree" && jsonTree?.ok && (
                <span>
                  {jsonTree.totalNodes.toLocaleString()}{" "}
                  {jsonTree.totalNodes === 1 ? "node" : "nodes"}
                  {jsonTree.truncated
                    ? ` · showing first ${jsonTree.nodes.length.toLocaleString()}`
                    : ""}
                </span>
              )}
              {viewerKind === "json" && editable && (
                <button type="button" className="button ghost small" onClick={handleFormatJson}>
                  Format JSON
                </button>
              )}
            </div>
          </div>
        )}
        {showViewerTabs && viewMode === "tree" && viewerKind === "json" && jsonTree ? (
          <JsonTreeViewer tree={jsonTree} onCopyPath={(p) => notify(`Copied path ${p}`)} />
        ) : showViewerTabs && viewMode === "formatted" ? (
          viewerKind === "binary" && binaryPreview ? (
            <pre
              className="value-viewer hex-dump"
              data-testid="binary-hex-viewer"
              aria-label="Binary hex and ASCII view"
              tabIndex={0}
            >
              {binaryPreview.totalBytes === 0 ? "0 bytes (empty binary value)" : binaryPreview.dump}
            </pre>
          ) : viewerKind === "spatial" && spatialPreview ? (
            <SpatialViewer spatial={spatialPreview} />
          ) : viewerKind === "json" && jsonPreview ? (
            jsonPreview.ok ? (
              <pre
                className="value-viewer json-viewer"
                data-testid="json-formatted-viewer"
                aria-label="Formatted JSON view"
                tabIndex={0}
              >
                {jsonPreview.formatted}
              </pre>
            ) : (
              <div className="value-viewer array-viewer-empty" data-testid="json-formatted-viewer">
                Current text is not valid JSON ({jsonPreview.error}). Switch to Raw view to edit.
              </div>
            )
          ) : null
        ) : (
          <textarea
            className="value-editor"
            aria-label="Cell value"
            value={isNull ? "NULL" : value}
            onChange={(e) => setValue(e.target.value)}
            readOnly={!editable || isNull}
            spellCheck={false}
          />
        )}
        <InlineError message={error} />
      </div>
      <footer className="modal-footer">
        <button type="button" className="button ghost" onClick={onClose}>
          {editable ? "Back to preview" : "Close"}
        </button>
        {editable && (
          <button type="button" className="button primary" onClick={handleSubmit}>
            Apply to staged cell
          </button>
        )}
      </footer>
    </Modal>
  );
}
