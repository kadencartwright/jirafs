import { useState } from "react";
import type { ServiceActionResultDto, TriggerSyncResultDto } from "../types";
import { FullResyncDialog } from "./full-resync-dialog";

type Props = {
  serviceRunning: boolean;
  serviceInstalled: boolean;
  syncDisabled: boolean;
  serviceActionDisabled: boolean;
  onServiceAction: () => Promise<ServiceActionResultDto>;
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

function mapServiceActionReason(
  reason: ServiceActionResultDto["reason"],
): string {
  switch (reason) {
    case "started":
      return "service started";
    case "restarted":
      return "service restarted";
    case "service_not_installed":
      return "service is not installed";
    case "action_failed":
      return "failed to start or restart service";
    default:
      return reason;
  }
}

export function ActionsCard({
  serviceRunning,
  serviceInstalled,
  syncDisabled,
  serviceActionDisabled,
  onServiceAction,
  onResync,
  onFullResync,
}: Props) {
  const [dialogOpen, setDialogOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const run = async (action: () => Promise<string>) => {
    setBusy(true);
    try {
      const nextMessage = await action();
      setMessage(nextMessage);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "action failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <section className="rounded-xl border border-border/70 bg-panel/80 p-4 shadow-sm shadow-black/25">
        <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-muted">
          Actions
        </h2>
        <div className="flex flex-wrap gap-2">
          <button
            className="rounded-md bg-primary px-3 py-2 text-sm font-medium text-canvas hover:bg-primary/85 disabled:cursor-not-allowed disabled:bg-border/50 disabled:text-muted"
            disabled={serviceActionDisabled || busy}
            onClick={() => {
              void run(async () => {
                const result = await onServiceAction();
                return mapServiceActionReason(result.reason);
              });
            }}
            type="button"
          >
            {serviceRunning ? "Restart Service" : "Start Service"}
          </button>
          <button
            className="rounded-md bg-accent px-3 py-2 text-sm font-medium text-canvas hover:bg-accent/85 disabled:cursor-not-allowed disabled:bg-border/50 disabled:text-muted"
            disabled={syncDisabled || busy}
            onClick={() => {
              void run(async () => {
                const result = await onResync();
                return mapReason(result.reason);
              });
            }}
            type="button"
          >
            Resync
          </button>
          <button
            className="rounded-md bg-danger px-3 py-2 text-sm font-medium text-canvas hover:bg-danger/85 disabled:cursor-not-allowed disabled:bg-border/50 disabled:text-muted"
            disabled={syncDisabled || busy}
            onClick={() => {
              setDialogOpen(true);
            }}
            type="button"
          >
            Full Resync
          </button>
        </div>
        {!serviceInstalled ? (
          <div className="mt-3 rounded-md border border-warn/50 bg-warn/15 p-3 text-xs text-warn">
            <p className="font-semibold">Service is not installed yet</p>
            <p className="mt-1 text-ink/85">
              Install and enable it from the repository root:
            </p>
            <p className="mt-1 font-mono text-ink">
              just service-install && just service-enable
            </p>
          </div>
        ) : null}
        {message ? (
          <p className="mt-3 text-xs text-muted">{message}</p>
        ) : null}
      </section>

      <FullResyncDialog
        onCancel={() => {
          setDialogOpen(false);
        }}
        onConfirm={() => {
          setDialogOpen(false);
          void run(async () => {
            const result = await onFullResync();
            return mapReason(result.reason);
          });
        }}
        open={dialogOpen}
      />
    </>
  );
}
