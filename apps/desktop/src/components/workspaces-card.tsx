import { useEffect, useMemo, useState } from "react";
import type { WorkspaceJqlInputDto, WorkspaceJqlValidationDto } from "../types";

type Props = {
  loading: boolean;
  loadError: string | null;
  initialRows: WorkspaceJqlInputDto[];
  onValidate: (
    rows: WorkspaceJqlInputDto[],
  ) => Promise<WorkspaceJqlValidationDto[]>;
  onSave: (rows: WorkspaceJqlInputDto[]) => Promise<void>;
};

export function WorkspacesCard({
  loading,
  loadError,
  initialRows,
  onValidate,
  onSave,
}: Props) {
  const [rows, setRows] = useState<WorkspaceJqlInputDto[]>(initialRows);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [validation, setValidation] = useState<WorkspaceJqlValidationDto[]>([]);

  useEffect(() => {
    setRows(initialRows);
  }, [initialRows]);

  const validationByName = useMemo(() => {
    const map = new Map<string, WorkspaceJqlValidationDto>();
    for (const item of validation) {
      map.set(item.name, item);
    }
    return map;
  }, [validation]);

  const updateRow = (index: number, field: "name" | "jql", value: string) => {
    setRows((prev) =>
      prev.map((row, idx) =>
        idx === index ? { ...row, [field]: value } : row,
      ),
    );
  };

  const runValidate = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const results = await onValidate(rows);
      setValidation(results);
      const invalid = results.filter((item) => !item.valid).length;
      setMessage(
        invalid === 0
          ? "all workspaces validated"
          : `${invalid} workspace(s) failed validation`,
      );
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "validation failed");
    } finally {
      setBusy(false);
    }
  };

  const runSave = async () => {
    setBusy(true);
    setMessage(null);
    try {
      await onSave(rows);
      setMessage("workspace JQL saved");
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "save failed");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="rounded-xl border border-border/70 bg-panel/80 p-4 shadow-sm shadow-black/25">
      <h2 className="mb-1 text-sm font-semibold uppercase tracking-wide text-muted">
        Workspace JQL
      </h2>
      <p className="mb-3 text-xs text-muted">
        Uses existing Jira credentials from config. Only
        <code> jira.workspaces.&lt;name&gt;.jql</code> is editable.
      </p>

      {loading ? (
        <p className="text-sm text-ink/85">Loading workspaces...</p>
      ) : null}
      {loadError ? (
        <p className="mb-2 rounded-md border border-danger/50 bg-danger/15 p-2 text-xs text-danger">
          {loadError}
        </p>
      ) : null}

      <div className="space-y-2">
        {rows.map((row, index) => {
          const outcome = validationByName.get(row.name.trim());
          return (
            <div
              className="rounded-md border border-border/60 bg-canvas/70 p-3"
              key={`${index}-${row.name}`}
            >
              <div className="grid grid-cols-1 gap-2 sm:grid-cols-5">
                <input
                  className="rounded-md border border-border/70 bg-panel px-2 py-2 text-sm text-ink sm:col-span-1"
                  onChange={(event) => {
                    updateRow(index, "name", event.target.value);
                  }}
                  placeholder="workspace"
                  value={row.name}
                />
                <input
                  className="rounded-md border border-border/70 bg-panel px-2 py-2 text-sm text-ink sm:col-span-4"
                  onChange={(event) => {
                    updateRow(index, "jql", event.target.value);
                  }}
                  placeholder="project = OPS ORDER BY updated DESC"
                  value={row.jql}
                />
              </div>
              {outcome && !outcome.valid ? (
                <p className="mt-2 text-xs text-danger">
                  {outcome.error ?? "validation failed"}
                </p>
              ) : null}
            </div>
          );
        })}
      </div>

      <div className="mt-3 flex flex-wrap gap-2">
        <button
          className="rounded-md border border-border/70 px-3 py-2 text-sm text-ink hover:bg-border/20"
          disabled={busy}
          onClick={() => {
            setRows((prev) => [...prev, { name: "", jql: "" }]);
          }}
          type="button"
        >
          Add Workspace
        </button>
        <button
          className="rounded-md border border-border/70 px-3 py-2 text-sm text-ink hover:bg-border/20"
          disabled={busy || rows.length <= 1}
          onClick={() => {
            setRows((prev) => prev.slice(0, prev.length - 1));
          }}
          type="button"
        >
          Remove Last
        </button>
        <button
          className="rounded-md bg-primary px-3 py-2 text-sm font-medium text-canvas hover:bg-primary/85 disabled:cursor-not-allowed disabled:bg-border/50 disabled:text-muted"
          disabled={busy || rows.length === 0}
          onClick={() => {
            void runValidate();
          }}
          type="button"
        >
          Validate
        </button>
        <button
          className="rounded-md bg-accent px-3 py-2 text-sm font-medium text-canvas hover:bg-accent/85 disabled:cursor-not-allowed disabled:bg-border/50 disabled:text-muted"
          disabled={busy || rows.length === 0}
          onClick={() => {
            void runSave();
          }}
          type="button"
        >
          Save
        </button>
      </div>

      {message ? (
        <p className="mt-3 text-xs text-ink/85">{message}</p>
      ) : null}
    </section>
  );
}
