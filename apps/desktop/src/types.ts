export type SyncStateValue = "stopped" | "running" | "syncing" | "degraded";
export type PathSource = "service_args" | "known_defaults" | "config_resolver";

export type TriggerReason =
  | "accepted"
  | "already_syncing"
  | "service_not_running"
  | "mountpoint_unavailable"
  | "trigger_write_failed";

export type SyncStatusDto = {
  last_sync: string | null;
  last_full_sync: string | null;
  seconds_to_next_sync: number | null;
  sync_in_progress: boolean;
};

export type AppStatusDto = {
  platform: string;
  service_installed: boolean;
  service_running: boolean;
  sync_state: SyncStateValue;
  config_path: string | null;
  mountpoint: string | null;
  path_source: PathSource;
  sync: SyncStatusDto;
  errors: string[];
};

export type TriggerSyncResultDto = {
  accepted: boolean;
  reason: TriggerReason;
};
