using Microsoft.ReactNative;
using Microsoft.ReactNative.Managed;
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using Windows.Storage;

namespace Clarora
{
    // Windows includes SQLite. Use its UTF-16 API to preserve Chinese text and
    // paths without introducing a second, incompatible React Native bridge.
    [ReactModule("RNWindowsDatabase")]
    public sealed class WindowsDatabase : IDisposable
    {
        private readonly object gate = new object();
        private IntPtr db;
        private const int Row = 100, Done = 101;
        private static readonly IntPtr Transient = new IntPtr(-1);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Unicode, ExactSpelling = true)] private static extern int sqlite3_open16(string path, out IntPtr db);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_close(IntPtr db);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern IntPtr sqlite3_errmsg16(IntPtr db);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Unicode, ExactSpelling = true)] private static extern int sqlite3_prepare16_v2(IntPtr db, string sql, int bytes, out IntPtr statement, IntPtr tail);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl, CharSet = CharSet.Unicode, ExactSpelling = true)] private static extern int sqlite3_bind_text16(IntPtr statement, int index, string value, int bytes, IntPtr destructor);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_bind_double(IntPtr statement, int index, double value);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_bind_null(IntPtr statement, int index);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_step(IntPtr statement);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_finalize(IntPtr statement);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_column_count(IntPtr statement);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern IntPtr sqlite3_column_name16(IntPtr statement, int column);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_column_type(IntPtr statement, int column);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern IntPtr sqlite3_column_text16(IntPtr statement, int column);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern double sqlite3_column_double(IntPtr statement, int column);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern int sqlite3_changes(IntPtr db);
        [DllImport("winsqlite3", CallingConvention = CallingConvention.Cdecl)] private static extern long sqlite3_last_insert_rowid(IntPtr db);

        private void Check(int code) { if (code != 0) throw new InvalidOperationException(Marshal.PtrToStringUni(sqlite3_errmsg16(db))); }
        [ReactMethod("open")]
        public Task Open(string name) => Task.Run(() => {
            lock (gate) {
                if (name != "clarora.db") throw new ArgumentException("不支持的数据库名称");
                if (db != IntPtr.Zero) return;
                var code = sqlite3_open16(Path.Combine(ApplicationData.Current.LocalFolder.Path, name), out db);
                if (code != 0) { var error = Marshal.PtrToStringUni(sqlite3_errmsg16(db)); Dispose(); throw new InvalidOperationException(error); }
                Execute("PRAGMA foreign_keys = ON", new JSValueArray());
                Execute("PRAGMA busy_timeout = 5000", new JSValueArray());
            }
        });
        private JSValue Execute(string sql, JSValue values)
        {
            Check(sqlite3_prepare16_v2(db, sql, -1, out var statement, IntPtr.Zero));
            if (statement == IntPtr.Zero) throw new ArgumentException("SQL 不能为空");
            try {
                var index = 1;
                foreach (var value in values.AsArray()) {
                    int result;
                    if (value.Type == JSValueType.Null) result = sqlite3_bind_null(statement, index);
                    else if (value.Type == JSValueType.String) result = sqlite3_bind_text16(statement, index, value.AsString(), -1, Transient);
                    else result = sqlite3_bind_double(statement, index, value.AsDouble());
                    Check(result); index++;
                }
                var rows = new JSValueArray();
                int code;
                while ((code = sqlite3_step(statement)) == Row) {
                    var row = new JSValueObject();
                    for (var column = 0; column < sqlite3_column_count(statement); column++) {
                        var name = Marshal.PtrToStringUni(sqlite3_column_name16(statement, column));
                        var type = sqlite3_column_type(statement, column);
                        row[name] = type == 5 ? JSValue.Null : type == 1 || type == 2
                            ? new JSValue(sqlite3_column_double(statement, column))
                            : new JSValue(Marshal.PtrToStringUni(sqlite3_column_text16(statement, column)));
                    }
                    rows.Add(row);
                }
                if (code != Done) Check(code);
                return new JSValueObject { ["rows"] = rows, ["rowsAffected"] = sqlite3_changes(db), ["insertId"] = sqlite3_last_insert_rowid(db) };
            } finally { sqlite3_finalize(statement); }
        }
        [ReactMethod("batch")]
        public Task<JSValue> Batch(JSValue statements, bool transaction) => Task.Run(() => {
            lock (gate) {
                if (db == IntPtr.Zero) throw new InvalidOperationException("数据库尚未打开");
                var results = new JSValueArray();
                if (transaction) Execute("BEGIN IMMEDIATE", new JSValueArray());
                try {
                    foreach (var item in statements.AsArray()) results.Add(Execute(item["sql"].AsString(), item["params"]));
                    if (transaction) Execute("COMMIT", new JSValueArray());
                    return (JSValue)results;
                } catch {
                    if (transaction) Execute("ROLLBACK", new JSValueArray());
                    throw;
                }
            }
        });
        public void Dispose() { lock (gate) { if (db != IntPtr.Zero) sqlite3_close(db); db = IntPtr.Zero; } }
    }
}
