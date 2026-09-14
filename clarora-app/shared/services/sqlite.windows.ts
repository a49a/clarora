import { NativeModules } from 'react-native';

type Statement = { sql: string; params: unknown[] };
type Result = { rows: Record<string, unknown>[]; rowsAffected: number; insertId: number };
const wrap = (result: Result) => ({
  ...result,
  rows: { length: result.rows.length, item: (i: number) => result.rows[i], raw: () => result.rows },
});
export type SQLiteDatabase = {
  executeSql: (sql: string, params?: unknown[]) => Promise<ReturnType<typeof wrap>[]>;
  transaction: (callback: (tx: { executeSql: (sql: string, params?: unknown[]) => void }) => void) => Promise<void>;
};
export default {
  enablePromise(_: boolean) {},
  async openDatabase({ name }: { name: string; location?: string }): Promise<SQLiteDatabase> {
    const native = NativeModules.RNWindowsDatabase;
    if (!native) throw new Error('Windows 数据库模块未加载，请重新构建客户端');
    await native.open(name);
    // The native module serializes batches, with BEGIN/COMMIT/ROLLBACK inside
    // its lock, so a failed import never leaves a half-written collection.
    return {
      async executeSql(sql, params = []) {
        const results: Result[] = await native.batch([{ sql, params }], false);
        return results.map(wrap);
      },
      async transaction(callback) {
        const statements: Statement[] = [];
        callback({ executeSql(sql, params = []) { statements.push({ sql, params }); } });
        await native.batch(statements, true);
      },
    };
  },
};
