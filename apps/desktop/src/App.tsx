import { ActionsCard } from "./components/actions-card";
import { PathCard } from "./components/path-card";
import { StatusCard } from "./components/status-card";
import { useAppStatus } from "./hooks/use-app-status";

export default function App() {
  const { status, loading, error, canTriggerSync, runAction } = useAppStatus();

  return (
    <main className="min-h-screen bg-canvas bg-[radial-gradient(circle_at_top,_rgba(15,118,110,0.08),_transparent_55%)] p-4 text-ink sm:p-6">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4">
        <header>
          <p className="text-xs uppercase tracking-[0.24em] text-muted">
            fs-jira desktop
          </p>
          <h1 className="mt-1 text-2xl font-semibold">Service Control Panel</h1>
        </header>

        {error ? (
          <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-danger">
            {error}
          </div>
        ) : null}
        {loading && !status ? (
          <div className="rounded-lg border border-slate-200 bg-white p-3 text-sm">
            Loading status...
          </div>
        ) : null}

        {status ? (
          <>
            <StatusCard status={status} />
            <PathCard status={status} />
            <ActionsCard
              disabled={!canTriggerSync}
              onFullResync={() => runAction("full_resync")}
              onResync={() => runAction("resync")}
            />

            {status.errors.length > 0 ? (
              <section className="rounded-xl border border-amber-300 bg-amber-50 p-4 text-sm text-warn">
                <h2 className="mb-2 font-semibold">Diagnostics</h2>
                <ul className="list-disc pl-4">
                  {status.errors.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </section>
            ) : null}
          </>
        ) : null}
      </div>
    </main>
  );
}
