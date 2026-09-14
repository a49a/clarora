// Windows file operations go through RNWindowsFiles in platform.ts. Avoid
// evaluating react-native-fs: its Windows project uses the retired C# bridge.
const unavailable = () => { throw new Error('Windows 文件操作应使用 FileSystem'); };
export default {
  DocumentDirectoryPath: '', readDir: unavailable, readFile: unavailable,
  writeFile: unavailable, mkdir: unavailable, copyFile: unavailable,
  unlink: unavailable, uploadFiles: unavailable, downloadFile: unavailable,
};
