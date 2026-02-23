import { useCallback, useEffect, useMemo, useState } from "react";
import { getAppStatus, triggerSync } from "../lib/tauri";
import type { AppStatusDto, TriggerSyncResultDto } from "../types";

const POLL_INTERVAL_MS = 5000;

export function useAppStatus() {
  const [status, setStatus] = useState<AppStatusDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastAction, setLastAction] = useState<TriggerSyncResultDto | null>(
    null,
  );

  const refresh = useCallback(async () => {
    try {
      const next = await getAppStatus();
      setStatus(next);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "failed to fetch status");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => {
      void refresh();
    }, POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [refresh]);

  const runAction = useCallback(
    async (kind: "resync" | "full_resync") => {
      const result = await triggerSync(kind);
      setLastAction(result);
      await refresh();
      return result;
    },
    [refresh],
  );

  const canTriggerSync = useMemo(() => {
    if (!status) {
      return false;
    }

    return status.service_running && !status.sync.sync_in_progress;
  }, [status]);

  return {
    status,
    loading,
    error,
    lastAction,
    canTriggerSync,
    refresh,
    runAction,
  };
}
