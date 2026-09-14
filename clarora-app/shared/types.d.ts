declare module "*.json" {
  const value: any;
  export default value;
}

declare module "react-native-sqlite-storage" {
  const SQLite: any;
  export type SQLiteDatabase = any;
  export default SQLite;
}
