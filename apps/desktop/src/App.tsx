import { useCallback, useEffect, useState } from "react";
import { ActionsCard } from "./components/actions-card";
import { LogsCard } from "./components/logs-card";
import { PathCard } from "./components/path-card";
import { StatusCard } from "./components/status-card";
import { WorkspacesCard } from "./components/workspaces-card";
import { useAppStatus } from "./hooks/use-app-status";
import {
  getSessionLogs,
  getWorkspaceJqlConfig,
  saveWorkspaceJqlConfig,
  validateWorkspaceJqls,
} from "./lib/tauri";
import type { LogLineDto, WorkspaceJqlInputDto } from "./types";

const LOG_POLL_INTERVAL_MS = 2000;

export default function App() {
  const {
    status,
    loading,
    error,
    canTriggerSync,
    canRunServiceAction,
    runAction,
    runServiceAction,
  } = useAppStatus();

  const [logs, setLogs] = useState<LogLineDto[]>([]);
  const [logsLoading, setLogsLoading] = useState(true);
  const [logsError, setLogsError] = useState<string | null>(null);
  const [workspaceRows, setWorkspaceRows] = useState<WorkspaceJqlInputDto[]>(
    [],
  );
  const [workspaceLoading, setWorkspaceLoading] = useState(true);
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);

  const refreshLogs = useCallback(async () => {
    try {
      const nextLogs = await getSessionLogs();
      setLogs(nextLogs);
      setLogsError(null);
    } catch (nextError) {
      setLogsError(
        nextError instanceof Error ? nextError.message : "failed to fetch logs",
      );
    } finally {
      setLogsLoading(false);
    }
  }, []);

  const refreshWorkspaces = useCallback(async () => {
    try {
      const rows = await getWorkspaceJqlConfig();
      setWorkspaceRows(rows);
      setWorkspaceError(null);
    } catch (nextError) {
      setWorkspaceError(
        nextError instanceof Error
          ? nextError.message
          : "failed to load workspace config",
      );
    } finally {
      setWorkspaceLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshLogs();
    const timer = window.setInterval(() => {
      void refreshLogs();
    }, LOG_POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [refreshLogs]);

  useEffect(() => {
    void refreshWorkspaces();
  }, [refreshWorkspaces]);

  return (
    <main className="min-h-screen bg-slate-950 p-4 text-slate-100 sm:p-6">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4">
        <header>
          <p className="text-xs uppercase tracking-[0.24em] text-slate-400">
            jirafs desktop
          </p>
          <h1 className="mt-1 text-2xl font-semibold text-slate-50">
            Service Control Panel
          </h1>
        </header>

        {error ? (
          <div className="rounded-lg border border-red-500/30 bg-red-950/40 p-3 text-sm text-red-200">
            {error}
          </div>
        ) : null}
        {loading && !status ? (
          <div className="rounded-lg border border-slate-700 bg-slate-900/70 p-3 text-sm text-slate-200">
            Loading status...
          </div>
        ) : null}

        {status ? (
          <>
            <StatusCard status={status} />
            <PathCard status={status} />
            <ActionsCard
              serviceRunning={status.service_running}
              serviceInstalled={status.service_installed}
              serviceActionDisabled={!canRunServiceAction}
              syncDisabled={!canTriggerSync}
              onServiceAction={runServiceAction}
              onFullResync={() => runAction("full_resync")}
              onResync={() => runAction("resync")}
            />
            <LogsCard error={logsError} loading={logsLoading} logs={logs} />
            <WorkspacesCard
              initialRows={workspaceRows}
              loadError={workspaceError}
              loading={workspaceLoading}
              onSave={async (rows) => {
                await saveWorkspaceJqlConfig(rows);
                await refreshWorkspaces();
              }}
              onValidate={validateWorkspaceJqls}
            />

            {status.errors.length > 0 ? (
              <section className="rounded-xl border border-amber-500/40 bg-amber-950/35 p-4 text-sm text-amber-200">
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
