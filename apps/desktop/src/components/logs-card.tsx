import { useEffect, useRef, useState } from "react";
import type { LogLineDto } from "../types";

type Props = {
  logs: LogLineDto[];
  loading: boolean;
  error: string | null;
};

export function LogsCard({ logs, loading, error }: Props) {
  const [expanded, setExpanded] = useState(false);
  const [stickToBottom, setStickToBottom] = useState(true);
  const logViewportRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (
      !expanded ||
      !stickToBottom ||
      !logViewportRef.current ||
      logs.length === 0
    ) {
      return;
    }

    const viewport = logViewportRef.current;
    viewport.scrollTop = viewport.scrollHeight;
  }, [expanded, logs, stickToBottom]);

  const handleScroll = () => {
    const viewport = logViewportRef.current;
    if (!viewport) {
      return;
    }

    const distanceFromBottom =
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
    setStickToBottom(distanceFromBottom < 20);
  };

  return (
    <section className="rounded-xl border border-border/70 bg-panel/80 p-4 shadow-sm shadow-black/25">
      <div className="mb-3 flex items-center justify-between gap-3">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-muted">
          Session Logs
        </h2>
        <button
          className="rounded-md border border-border/70 px-3 py-1.5 text-xs font-medium text-ink hover:bg-border/20"
          onClick={() => {
            setExpanded((prev) => !prev);
            setStickToBottom(true);
          }}
          type="button"
        >
          {expanded ? "Collapse" : `Expand (${logs.length})`}
        </button>
      </div>

      {loading ? (
        <p className="text-sm text-ink/85">Loading logs...</p>
      ) : null}
      {error ? (
        <p className="rounded-md border border-danger/50 bg-danger/15 p-2 text-xs text-danger">
          {error}
        </p>
      ) : null}
      {!loading && !error && logs.length === 0 ? (
        <p className="text-sm text-muted">
          No logs captured in this desktop session yet.
        </p>
      ) : null}

      {expanded && logs.length > 0 ? (
        <div
          className="max-h-72 overflow-y-auto rounded-md border border-border/60 bg-canvas/70 p-2 font-mono text-xs text-ink/90"
          onScroll={handleScroll}
          ref={logViewportRef}
        >
          {logs.map((entry, idx) => (
            <div
              className="whitespace-pre-wrap break-words py-0.5"
              key={`${idx}-${entry.line}`}
            >
              <span className="text-muted">[{entry.source}] </span>
              {entry.line}
            </div>
          ))}
        </div>
      ) : null}
    </section>
  );
}
