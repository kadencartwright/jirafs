import { useState } from "react";
import type { TriggerSyncResultDto } from "../types";
import { FullResyncDialog } from "./full-resync-dialog";

type Props = {
  disabled: boolean;
  onResync: () => Promise<TriggerSyncResultDto>;
  onFullResync: () => Promise<TriggerSyncResultDto>;
};

function mapReason(reason: TriggerSyncResultDto["reason"]): string {
  switch (reason) {
    case "accepted":
      return "sync request accepted";
    case "already_syncing":
      return "sync already in progress";
    case "service_not_running":
      return "service is not running";
    case "mountpoint_unavailable":
      return "mountpoint unavailable";
    case "trigger_write_failed":
      return "failed to write trigger file";
    default:
      return reason;
  }
}

export function ActionsCard({ disabled, onResync, onFullResync }: Props) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const run = async (action: () => Promise<TriggerSyncResultDto>) => {
    setBusy(true);
    try {
      const result = await action();
      setMessage(mapReason(result.reason));
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "action failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <section className="rounded-xl border border-slate-200 bg-panel p-4 shadow-sm">
        <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-muted">
          Actions
        </h2>
        <div className="flex flex-wrap gap-2">
          <button
            className="rounded-md bg-accent px-3 py-2 text-sm font-medium text-white hover:bg-teal-700 disabled:cursor-not-allowed disabled:bg-slate-300"
            disabled={disabled || busy}
            onClick={() => {
              void run(onResync);
            }}
            type="button"
          >
            Resync
          </button>
          <button
            className="rounded-md bg-danger px-3 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:bg-slate-300"
            disabled={disabled || busy}
            onClick={() => {
              setDialogOpen(true);
            }}
            type="button"
          >
            Full Resync
          </button>
        </div>
        {message ? <p className="mt-3 text-xs text-muted">{message}</p> : null}
      </section>

      <FullResyncDialog
        onCancel={() => {
          setDialogOpen(false);
        }}
        onConfirm={() => {
          setDialogOpen(false);
          void run(onFullResync);
        }}
        open={dialogOpen}
      />
    </>
  );
}
