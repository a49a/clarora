/** Portable learning data only. Credentials and machine-specific settings never enter a vault. */
export const VAULT_COLUMNS = {
  sentence_cards: ['id', 'text', 'translation', 'notes', 'created_at'],
  words: ['id', 'word', 'meaning', 'source', 'created_at'],
  listening_practices: ['id', 'name', 'created_at'],
  listening_audios: ['id', 'practice_id', 'name', 'audio_uri', 'subtitle_uri', 'created_at'],
  clip_cards: ['id', 'en_text', 'zh_text', 'audio_uri', 'start_ms', 'end_ms', 'created_at'],
  ai_cards: ['id', 'question', 'answer', 'context_text', 'created_at'],
  video_clips: ['id', 'en_text', 'zh_text', 'video_uri', 'created_at'],
  review_schedule: ['card_kind', 'card_id', 'ease', 'interval_days', 'due_at', 'reps', 'lapses', 'suspended', 'last_graded_at', 'created_at'],
  study_stats: ['day', 'listen_seconds', 'review_count', 'pomodoro_count'],
  discover_favorites: ['key', 'payload'],
  ai_records: ['id', 'kind', 'payload', 'created_at'],
} as const;
export type VaultTable = keyof typeof VAULT_COLUMNS;
export type VaultRow = Record<string, string | number | null>;
export type VaultData = Record<VaultTable, VaultRow[]>;
export const VAULT_TABLES = Object.keys(VAULT_COLUMNS) as VaultTable[];
export function validateVault(data: unknown): asserts data is VaultData {
  if (!data || typeof data !== 'object') throw new Error('备份数据格式错误');
  const raw = data as Record<string, unknown>;
  // Older backups predate sentence cards; normalize the additive table before restore.
  if (!('sentence_cards' in raw)) raw.sentence_cards = [];
  for (const table of VAULT_TABLES) {
    const rows = raw[table];
    if (!Array.isArray(rows) || rows.length > 500000) throw new Error(`备份表 ${table} 无效`);
    for (const row of rows) {
      if (!row || typeof row !== 'object' || Array.isArray(row)) throw new Error('备份记录无效');
      for (const column of VAULT_COLUMNS[table]) {
        const value = row[column];
        if (!(value === null || typeof value === 'string' || typeof value === 'number' && Number.isFinite(value))) throw new Error(`备份字段 ${table}.${column} 无效`);
      }
    }
  }
}
export const newVaultId = () => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`;
