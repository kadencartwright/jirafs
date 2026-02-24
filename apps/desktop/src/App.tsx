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
    <main className="min-h-screen bg-canvas p-4 text-ink sm:p-6">
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4">
        <header>
          <p className="text-xs uppercase tracking-[0.24em] text-muted">
            jirafs desktop
          </p>
          <h1 className="mt-1 text-2xl font-semibold text-ink">
            Service Control Panel
          </h1>
        </header>

        {error ? (
          <div className="rounded-lg border border-danger/50 bg-danger/15 p-3 text-sm text-danger">
            {error}
          </div>
        ) : null}
        {loading && !status ? (
          <div className="rounded-lg border border-border/70 bg-panel/70 p-3 text-sm text-ink">
            Loading status...
          </div>
        ) : null}

        {status ? (
          <>
            <StatusCard status={status} />
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

            {status.errors.length > 0 ? (
              <section className="rounded-xl border border-warn/50 bg-warn/15 p-4 text-sm text-warn">
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
