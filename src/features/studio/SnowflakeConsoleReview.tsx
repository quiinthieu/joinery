import type { ConnectionProfile, SnowflakeConsolePlan } from "../../ipc/generated";
import { ReviewDialog } from "../safety/ReviewDialog";

export function SnowflakeConsoleReview({
  plan,
  profile,
  onExecute,
  onClose,
}: {
  plan: SnowflakeConsolePlan;
  profile?: ConnectionProfile;
  onExecute(approval: string): Promise<void>;
  onClose(): void;
}) {
  return (
    <ReviewDialog
      plan={{
        ...plan.review,
        warnings: [
          "Reviewed SQL console: approval authorizes these exact top-level units and bound values on the retained physical session. Acknowledgement does not prove committed effects.",
          "BEGIN, COMMIT, ROLLBACK, DDL and session settings have their own database effects. Joinery will not wrap, settle, retry or resume this script. An unattempted COMMIT or ROLLBACK stays unattempted.",
          "This connection preserves statement adjacency across runs: automatic SQL probes and generated metadata actions are disabled. Use a separate profile for metadata; it does not share transactions or temporary objects.",
          "Each unit currently has a fixed 65-second delivery deadline. Long-running query parity remains incomplete. PUT and GET file transfers are denied.",
          ...plan.validations.map(
            (validation, index) =>
              `Unit ${index + 1}: ${validation === "serverAtExecution" ? "selected unit; syntax and statement count are validated by Snowflake only at execution. This is not a locally verified statement." : "locally parsed syntax; effects and target identity are not verified."}`,
          ),
          ...plan.review.warnings,
        ],
      }}
      profile={profile}
      onExecute={onExecute}
      onClose={onClose}
    />
  );
}
