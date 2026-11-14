import type {
  AuditEntry,
  BookmarkTarget,
  Bootstrap,
  BrowseApproval,
  BrowseQuery,
  BrowseCatalog,
  DefineRelationRequest,
  InspectRelationRequest,
  InspectRelationConstraintsRequest,
  InspectRelationCheckConstraintsRequest,
  InspectRelationConstraintMappingsRequest,
  PreviewRelationRequest,
  RelationInspection,
  RelationDefinition,
  RelationConstraints,
  RelationCheckConstraints,
  RelationConstraintMappings,
  RelationPreview,
  CellChange,
  ChangePreview,
  CompareRequest,
  CompletionCatalog,
  ConnectionGroup,
  ConnectionProfile,
  ConnectionSecrets,
  CsvColumnMapping,
  CsvImportOptions,
  CsvImportPreview,
  DataCompareReport,
  DatabaseObjectRef,
  DatabaseObjectSummary,
  DdlStatement,
  EngineKind,
  ExportFormat,
  ExportRequest,
  FileSelection,
  HistoryEntry,
  HistoryPage,
  HistoryPageRequest,
  InboundForeignKey,
  IndexConstraintAction,
  IndexConstraintPreview,
  LockWaitSnapshot,
  RecoveryBackup,
  RecoveryOperation,
  RecoveryRequest,
  ObjectDetails,
  OpenedSqlFile,
  PreparedPlan,
  ProcessInfo,
  QueryParamInput,
  QueryParamSlot,
  RedisScanPage,
  RedisValuePreview,
  ResourceUsage,
  ResultChanges,
  ResultInfo,
  SavedBookmark,
  SavedQuery,
  SavedQueryInput,
  SavedSqlFile,
  SavedWorkspace,
  SchemaDdlPlan,
  SchemaDiagram,
  SchemaDiffReport,
  SchemaSnapshot,
  SessionContextOptions,
  SessionInfo,
  SessionRuntimeState,
  SwitchSchemaRequest,
  SqlFormatSettings,
  SqlSnippet,
  SqlSnippetInput,
  SshHostKey,
  TableChanges,
  TableInfo,
  TableMaintenanceAction,
  TableMaintenancePreview,
  TableObjectAction,
  TableObjectPreview,
  TablePage,
  TableRef,
  TableSchema,
  TransactionMode,
  TransactionControlRequest,
  TransactionControlResult,
  SnowflakeDmlResult,
  SnowflakeDdlPreflightRequest,
  SnowflakeDdlPreflightResponse,
  SnowflakeDdlRequest,
  SnowflakeDdlResult,
  PrepareSnowflakeGridEditsRequest,
  SnowflakeGridFinishAction,
  SnowflakeGridResult,
  SnowflakeConsoleRequest,
  SnowflakeConsolePlan,
  SnowflakeConsoleStart,
  SnowflakeConsoleStartResult,
  SnowflakeConsoleRunView,
  SnowflakeConsoleRequestLookup,
  SnowflakeConsoleReceiptPageRequest,
  SnowflakeConsoleReceiptPage,
  SnowflakeConsoleReceiptSummary,
  SnowflakeConsoleReceiptArchiveRequest,
  SnowflakeConsoleReceiptArchiveResult,
  SnowflakeConsolePending,
  SnowflakeConsoleStorage,
} from "./generated";

/**
 * Closed compile-time IPC command map (`I03`) pairing every Joinery desktop/test-harness
 * command with its exact argument payload and result DTO.
 */
export interface JoineryCommandMap {
  prepare_snowflake_console: {
    args: { sessionId: string; request: SnowflakeConsoleRequest };
    result: SnowflakeConsolePlan;
  };
  start_snowflake_console: {
    args: { request: SnowflakeConsoleStart; onEvent?: unknown; streamId?: string };
    result: SnowflakeConsoleStartResult;
  };
  snowflake_console_request: {
    args: { clientRequestId: string };
    result: SnowflakeConsoleRequestLookup | null;
  };
  snowflake_console_run: {
    args: { clientRequestId: string };
    result: SnowflakeConsoleRunView | null;
  };
  snowflake_console_receipts: {
    args: { request: SnowflakeConsoleReceiptPageRequest };
    result: SnowflakeConsoleReceiptPage;
  };
  snowflake_console_receipt_summary: {
    args: { clientRequestId: string };
    result: SnowflakeConsoleReceiptSummary | null;
  };
  set_snowflake_console_receipt_archive: {
    args: { request: SnowflakeConsoleReceiptArchiveRequest };
    result: SnowflakeConsoleReceiptArchiveResult;
  };
  snowflake_console_pending: { args: undefined; result: SnowflakeConsolePending };
  snowflake_console_storage: { args: undefined; result: SnowflakeConsoleStorage };
  release_snowflake_console_results: {
    args: { clientRequestId: string };
    result: null;
  };
  relation_catalog: { args: { sessionId: string }; result: BrowseCatalog };
  define_relation: {
    args: { sessionId: string; request: DefineRelationRequest };
    result: RelationDefinition;
  };
  inspect_relation: {
    args: { sessionId: string; request: InspectRelationRequest };
    result: RelationInspection;
  };
  inspect_relation_constraints: {
    args: { sessionId: string; request: InspectRelationConstraintsRequest };
    result: RelationConstraints;
  };
  inspect_relation_check_constraints: {
    args: { sessionId: string; request: InspectRelationCheckConstraintsRequest };
    result: RelationCheckConstraints;
  };
  inspect_relation_constraint_mappings: {
    args: { sessionId: string; request: InspectRelationConstraintMappingsRequest };
    result: RelationConstraintMappings;
  };
  preview_relation: {
    args: { sessionId: string; request: PreviewRelationRequest };
    result: RelationPreview;
  };
  prepare_connection: {
    args: { id: string };
    result: string;
  };
  connect_prepared: {
    args: { attemptId: string; secrets: ConnectionSecrets };
    result: SessionInfo;
  };
  cancel_connection_attempt: {
    args: { attemptId: string };
    result: void;
  };
  app_shutdown: { args: undefined; result: void };
  set_temporary_result_storage: {
    args: { enabled: boolean };
    result: boolean;
  };
  app_info: {
    args?: Record<string, unknown>;
    result: { name: string; version: string; identifier: string };
  };
  acknowledge_quit: {
    args: { generation: number };
    result: void;
  };
  bootstrap: {
    args?: Record<string, unknown>;
    result: Bootstrap;
  };
  save_workspace: {
    args: { workspace: SavedWorkspace };
    result: void;
  };
  recovery_backups: {
    args?: Record<string, unknown>;
    result: RecoveryBackup[];
  };
  start_metadata_recovery: {
    args: {
      id: string;
      request: RecoveryRequest;
      backupPassphrase: string;
      backupConfirmation: string;
      writePassphrase?: string | null;
    };
    result: RecoveryOperation;
  };
  delete_recovery_backup: {
    args: { id: string; digest: string; confirmation: string };
    result: void;
  };
  metadata_recovery_operation: {
    args: { id: string };
    result: RecoveryOperation;
  };
  cancel_metadata_recovery: {
    args: { id: string };
    result: RecoveryOperation;
  };
  reconcile_metadata_recovery: {
    args: { id: string };
    result: RecoveryOperation;
  };
  reconcile_unknown_metadata_recovery: {
    args?: Record<string, unknown>;
    result: RecoveryOperation;
  };
  metadata_recovery_admission_closed: {
    args?: Record<string, unknown>;
    result: boolean;
  };
  save_group: {
    args: { group: ConnectionGroup };
    result: ConnectionGroup;
  };
  delete_group: {
    args: { id: string };
    result: void;
  };
  audit_log: {
    args?: Record<string, unknown>;
    result: AuditEntry[];
  };
  save_profile: {
    args: { profile: ConnectionProfile; passphrase?: string | null };
    result: ConnectionProfile;
  };
  delete_profile: {
    args: { id: string };
    result: void;
  };
  connect: {
    args: { id: string; secrets: ConnectionSecrets };
    result: SessionInfo;
  };
  disconnect: {
    args: { sessionId: string };
    result: void;
  };
  probe_ssh: {
    args: { profileId: string; attemptId?: string | null };
    result: SshHostKey;
  };
  trust_ssh_key: {
    args: { challengeId: string | null; fingerprint: string };
    result: void;
  };
  set_passphrase: {
    args: { passphrase: string; previous?: string | null };
    result: void;
  };
  extract_query_params: {
    args: { engine: EngineKind; sql: string };
    result: QueryParamSlot[];
  };
  prepare_execution: {
    args: {
      sessionId: string;
      sql: string;
      params?: QueryParamInput[] | null;
    };
    result: PreparedPlan;
  };
  prepare_explain: {
    args: {
      sessionId: string;
      sql: string;
      analyze?: boolean | null;
      params?: QueryParamInput[] | null;
    };
    result: PreparedPlan;
  };
  authorize_plan: {
    args: {
      planId: string;
      passphrase?: string | null;
      confirmation?: string | null;
    };
    result: string;
  };
  execute_plan: {
    args: {
      planId: string;
      approval?: string | null;
      onEvent?: unknown;
      streamId?: string;
    };
    result: string;
  };
  cancel_run: {
    args: { runId: string };
    result: void;
  };
  catalog: {
    args: { sessionId: string };
    result: TableInfo[];
  };
  database_objects: {
    args: { sessionId: string };
    result: DatabaseObjectSummary[];
  };
  object_details: {
    args: { sessionId: string; object: DatabaseObjectRef };
    result: ObjectDetails;
  };
  table_schema: {
    args: { sessionId: string; table: TableRef };
    result: TableSchema;
  };
  referencing_foreign_keys: {
    args: { sessionId: string; table: TableRef };
    result: InboundForeignKey[];
  };
  schema_diagram: {
    args: { sessionId: string; schema: string };
    result: SchemaDiagram;
  };
  schema_snapshot: {
    args: { sessionId: string; schema: string };
    result: SchemaSnapshot;
  };
  compare_schemas: {
    args: { left: SchemaSnapshot; right: SchemaSnapshot };
    result: SchemaDiffReport;
  };
  generate_schema_ddl: {
    args: { snapshot: SchemaSnapshot; targetSchema?: string | null };
    result: SchemaDdlPlan;
  };
  generate_migration_ddl: {
    args: {
      left: SchemaSnapshot;
      right: SchemaSnapshot;
      targetSchema?: string | null;
    };
    result: SchemaDdlPlan;
  };
  prepare_migration_ddl: {
    args: {
      sessionId: string;
      expectedSnapshot: SchemaSnapshot;
      statements: DdlStatement[];
    };
    result: PreparedPlan;
  };
  preview_index_constraint_ddl: {
    args: { engine: EngineKind; action: IndexConstraintAction };
    result: IndexConstraintPreview;
  };
  prepare_index_constraint_ddl: {
    args: { sessionId: string; action: IndexConstraintAction };
    result: PreparedPlan;
  };
  preview_table_object_ddl: {
    args: { engine: EngineKind; action: TableObjectAction };
    result: TableObjectPreview;
  };
  prepare_table_object_ddl: {
    args: { sessionId: string; action: TableObjectAction };
    result: PreparedPlan;
  };
  preview_table_maintenance: {
    args: { engine: EngineKind; action: TableMaintenanceAction };
    result: TableMaintenancePreview;
  };
  prepare_table_maintenance: {
    args: { sessionId: string; action: TableMaintenanceAction };
    result: PreparedPlan;
  };
  prepare_browse_table: {
    args: { sessionId: string; table: TableRef; query: BrowseQuery };
    result: PreparedPlan | null;
  };
  browse_table: {
    args: {
      sessionId: string;
      table: TableRef;
      after?: Array<string | null> | null;
      query?: BrowseQuery | null;
      approval?: BrowseApproval | null;
    };
    result: TablePage;
  };
  result_range: {
    args: {
      resultId: string;
      start: number;
      count: number;
      onData?: unknown;
    };
    result: void;
  };
  release_result: {
    args: { resultId: string };
    result: void;
  };
  prepare_changes: {
    args: { sessionId: string; resultId: string; changes: CellChange[] };
    result: ChangePreview;
  };
  prepare_changes_batch: {
    args: { sessionId: string; inputs: ResultChanges[] };
    result: ChangePreview;
  };
  prepare_table_changes: {
    args: { sessionId: string; changes: TableChanges };
    result: ChangePreview;
  };
  processes: {
    args: { sessionId: string };
    result: ProcessInfo[];
  };
  lock_waits: {
    args: { sessionId: string };
    result: LockWaitSnapshot;
  };
  prepare_process_cancel: {
    args: { sessionId: string; id: string; identity: string };
    result: PreparedPlan;
  };
  health: {
    args: { sessionId: string };
    result: number;
  };
  history: {
    args: { search: string };
    result: HistoryEntry[];
  };
  history_page: {
    args: { request: HistoryPageRequest };
    result: HistoryPage;
  };
  history_entry: {
    args: { id: string };
    result: HistoryEntry | null;
  };
  clear_history: {
    args?: Record<string, unknown>;
    result: void;
  };
  resource_usage: {
    args?: Record<string, unknown>;
    result: ResourceUsage;
  };
  redis_scan: {
    args: { sessionId: string; pattern: string; cursor?: string | null };
    result: RedisScanPage;
  };
  redis_inspect: {
    args: { sessionId: string; argument: string; cursor?: string | null };
    result: RedisValuePreview;
  };
  completion_catalog: {
    args: { sessionId: string };
    result: CompletionCatalog;
  };
  completion_schema: {
    args: { sessionId: string; revision: string; table: TableRef };
    result: TableSchema;
  };
  inspection_schema: {
    args: { sessionId: string; revision: string; table: TableRef };
    result: TableSchema;
  };
  pick_export_destination: {
    args: { suggestedName: string; format: ExportFormat };
    result: FileSelection | null;
  };
  start_export: {
    args: {
      request: ExportRequest;
      onEvent?: unknown;
      streamId?: string;
    };
    result: string;
  };
  cancel_export: {
    args: { exportId: string };
    result: void;
  };
  pick_import_source: {
    args?: Record<string, unknown>;
    result: FileSelection | null;
  };
  preview_csv_import: {
    args: {
      sessionId: string;
      sourceToken: string;
      table: TableRef;
      options: CsvImportOptions;
      mappings: CsvColumnMapping[];
    };
    result: CsvImportPreview;
  };
  release_csv_import_preview: {
    args: { sessionId: string; sourceToken: string; previewToken: string };
    result: void;
  };
  prepare_csv_import_batch: {
    args: {
      sessionId: string;
      sourceToken: string;
      table: TableRef;
      options: CsvImportOptions;
      mappings: CsvColumnMapping[];
      batchIndex: number;
      previewToken: string;
    };
    result: ChangePreview;
  };
  compare_data: {
    args: { request: CompareRequest };
    result: DataCompareReport;
  };
  session_contexts: {
    args: { sessionId: string; resumeMetadata?: boolean };
    result: SessionContextOptions;
  };
  switch_session_context: {
    args: { sessionId: string; target: string };
    result: SessionContextOptions;
  };
  switch_session_schema: {
    args: { sessionId: string; request: SwitchSchemaRequest };
    result: SessionContextOptions;
  };
  prepare_transaction_control: {
    args: { sessionId: string; request: TransactionControlRequest };
    result: PreparedPlan;
  };
  execute_transaction_control: {
    args: { planId: string; approval?: string | null };
    result: TransactionControlResult;
  };
  execute_snowflake_dml: {
    args: { planId: string; approval?: string | null };
    result: SnowflakeDmlResult;
  };
  preflight_snowflake_ddl: {
    args: { request: SnowflakeDdlPreflightRequest };
    result: SnowflakeDdlPreflightResponse;
  };
  prepare_snowflake_ddl: {
    args: { sessionId: string; request: SnowflakeDdlRequest };
    result: PreparedPlan;
  };
  authorize_snowflake_ddl: {
    args: {
      planId: string;
      passphrase?: string | null;
      confirmation?: string | null;
      targetConfirmation?: string | null;
    };
    result: string;
  };
  execute_snowflake_ddl: {
    args: { planId: string; approval?: string | null };
    result: SnowflakeDdlResult;
  };
  refresh_snowflake_ddl_metadata: {
    args: { sessionId: string };
    result: SessionContextOptions;
  };
  prepare_snowflake_grid_plan: {
    args: { request: PrepareSnowflakeGridEditsRequest };
    result: ChangePreview;
  };
  execute_snowflake_grid_plan: {
    args: { planId: string; approval?: string | null };
    result: SnowflakeGridResult;
  };
  preview_snowflake_grid_transaction_relation: {
    args: { transactionId: string; request: PreviewRelationRequest };
    result: RelationPreview;
  };
  finish_snowflake_grid_transaction: {
    args: { transactionId: string; action: SnowflakeGridFinishAction };
    result: SnowflakeGridResult;
  };
  set_transaction_mode: {
    args: { sessionId: string; mode: TransactionMode };
    result: SessionRuntimeState;
  };
  commit_transaction: {
    args: { sessionId: string };
    result: SessionRuntimeState;
  };
  rollback_transaction: {
    args: { sessionId: string };
    result: SessionRuntimeState;
  };
  open_sql_file: {
    args?: Record<string, unknown>;
    result: OpenedSqlFile | null;
  };
  save_sql_file: {
    args: { token?: string | null; suggestedName: string; content: string };
    result: SavedSqlFile | null;
  };
  release_sql_file_grant: {
    args: { token: string };
    result: void;
  };
  save_saved_query: {
    args: { input: SavedQueryInput };
    result: SavedQuery;
  };
  delete_saved_query: {
    args: { id: string };
    result: void;
  };
  save_bookmark: {
    args: { label: string; target: BookmarkTarget };
    result: SavedBookmark;
  };
  delete_bookmark: {
    args: { id: string };
    result: void;
  };
  save_sql_format_settings: {
    args: { settings: SqlFormatSettings };
    result: SqlFormatSettings;
  };
  save_sql_snippet: {
    args: { input: SqlSnippetInput };
    result: SqlSnippet;
  };
  delete_sql_snippet: {
    args: { id: string };
    result: void;
  };
  native_smoke_fixture: {
    args?: Record<string, unknown>;
    result: ResultInfo;
  };
  native_smoke_checkpoint: {
    args: { phase: string; cycle?: number | null };
    result: Record<string, unknown>;
  };
  native_smoke_report: {
    args: { report: Record<string, unknown> };
    result: void;
  };
}

export type JoineryCommandName = keyof JoineryCommandMap;

export type NoArgCommandName = {
  [K in JoineryCommandName]: undefined extends JoineryCommandMap[K]["args"] ? K : never;
}[JoineryCommandName];

export type RequiredArgCommandName = Exclude<JoineryCommandName, NoArgCommandName>;

export type JoineryCommandArgs<K extends JoineryCommandName> = JoineryCommandMap[K]["args"];

export type JoineryCommandResult<K extends JoineryCommandName> = JoineryCommandMap[K]["result"];

export type JoineryCommandCallTuple = {
  [K in JoineryCommandName]: K extends NoArgCommandName
    ? [command: K, args?: JoineryCommandMap[K]["args"]]
    : [command: K, args: JoineryCommandMap[K]["args"]];
}[JoineryCommandName];
