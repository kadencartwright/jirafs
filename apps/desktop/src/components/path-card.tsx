import type { AppStatusDto } from "../types";

type Props = {
  status: AppStatusDto;
};

function PathRow({ label, value }: { label: string; value: string | null }) {
  return (
    <div className="rounded-md bg-slate-50 p-3">
      <p className="mb-1 text-xs uppercase tracking-wide text-muted">{label}</p>
      <p className="break-all font-mono text-xs text-ink">
        {value ?? "unresolved"}
      </p>
    </div>
  );
}

export function PathCard({ status }: Props) {
  return (
    <section className="rounded-xl border border-slate-200 bg-panel p-4 shadow-sm">
      <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-muted">
        Paths
      </h2>
      <div className="grid grid-cols-1 gap-3">
        <PathRow label="Config path" value={status.config_path} />
        <PathRow label="Mountpoint" value={status.mountpoint} />
      </div>
    </section>
  );
}
