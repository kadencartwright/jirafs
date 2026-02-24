import type { AppStatusDto } from "../types";

type Props = {
  status: AppStatusDto;
};

const STATE_COLOR: Record<AppStatusDto["sync_state"], string> = {
  stopped: "bg-border/50 text-ink",
  running: "bg-success/20 text-success",
  syncing: "bg-warn/20 text-warn",
  degraded: "bg-danger/20 text-danger",
};

export function StatusCard({ status }: Props) {
  return (
    <section className="rounded-xl border border-border/70 bg-panel/80 p-4 shadow-sm shadow-black/25">
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-muted">
          Runtime Status
        </h2>
        <span
          className={`rounded-full px-3 py-1 text-xs font-semibold ${STATE_COLOR[status.sync_state]}`}
        >
          {status.sync_state}
        </span>
      </div>

      <dl className="grid grid-cols-1 gap-2 text-sm text-ink sm:grid-cols-2">
        <div>
          <dt className="text-muted">Platform</dt>
          <dd>{status.platform}</dd>
        </div>
        <div>
          <dt className="text-muted">Service</dt>
          <dd>
            {status.service_running
              ? "Running"
              : status.service_installed
                ? "Stopped"
                : "Not installed"}
          </dd>
        </div>
        <div>
          <dt className="text-muted">Last sync</dt>
          <dd>{status.sync.last_sync ?? "unknown"}</dd>
        </div>
        <div>
          <dt className="text-muted">Last full sync</dt>
          <dd>{status.sync.last_full_sync ?? "unknown"}</dd>
        </div>
        <div>
          <dt className="text-muted">Seconds to next sync</dt>
          <dd>{status.sync.seconds_to_next_sync ?? "unknown"}</dd>
        </div>
        <div>
          <dt className="text-muted">Path source</dt>
          <dd>{status.path_source}</dd>
        </div>
      </dl>
    </section>
  );
}
