export type NotificationLevel = "Info" | "Warning" | "Error" | "Critical";

export interface Notification {
  id: string;
  code: string;
  level: NotificationLevel;
  title: string;
  message: string;
  suggestion?: string | null;
  source: string;
  timestamp: string;
  actionable: boolean;
}
