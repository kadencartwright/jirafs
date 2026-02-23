type Props = {
  open: boolean;
  onCancel: () => void;
  onConfirm: () => void;
};

export function FullResyncDialog({ open, onCancel, onConfirm }: Props) {
  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-20 flex items-center justify-center bg-black/30 px-4">
      <div className="w-full max-w-sm rounded-xl bg-white p-5 shadow-lg">
        <h3 className="text-base font-semibold text-ink">
          Trigger full resync?
        </h3>
        <p className="mt-2 text-sm text-muted">
          This schedules a full upsert sync through{" "}
          <code>.sync_meta/full_refresh</code>.
        </p>
        <div className="mt-4 flex justify-end gap-2">
          <button
            className="rounded-md border border-slate-300 px-3 py-2 text-sm text-ink hover:bg-slate-100"
            onClick={onCancel}
            type="button"
          >
            Cancel
          </button>
          <button
            className="rounded-md bg-danger px-3 py-2 text-sm font-medium text-white hover:bg-red-700"
            onClick={onConfirm}
            type="button"
          >
            Full Resync
          </button>
        </div>
      </div>
    </div>
  );
}
