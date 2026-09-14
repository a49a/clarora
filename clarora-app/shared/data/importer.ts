import { FileSystem } from "../services/platform";

/**
 * Parse a text file into word-meaning pairs.
 * Supports tab, comma, Chinese comma, " - ", " : ", "：" as separators.
 */
export function parseWordLines(
  text: string
): Array<{ word: string; meaning: string }> {
  const separators = ["\t", ",", "，", " - ", " : ", "："];
  const results: Array<{ word: string; meaning: string }> = [];

  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;

    for (const sep of separators) {
      const idx = trimmed.indexOf(sep);
      if (idx > 0) {
        const word = trimmed.slice(0, idx).trim();
        const meaning = trimmed.slice(idx + sep.length).trim();
        if (word && meaning) {
          results.push({ word, meaning });
          break;
        }
      }
    }
  }

  return results;
}

/**
 * Read a text file from URI and parse words.
 */
export async function importFromTextFile(
  fileUri: string
): Promise<Array<{ word: string; meaning: string }>> {
  const content = await FileSystem.readAsStringAsync(fileUri);
  return parseWordLines(content);
}

/**
 * Import flashcards from a directory tree:
 * each file name (without extension) is the front (word),
 * and the file content is the back (meaning).
 * Recursively includes every regular file in all subdirectories.
 */
export async function importFromDirectory(
  dirPath: string
): Promise<Array<{ word: string; meaning: string }>> {
  const paths = await FileSystem.listFilesAsync(dirPath);
  const results: Array<{ word: string; meaning: string }> = [];
  for (const path of paths) {
    const fileName = path.split("/").pop() ?? "";
    if (!fileName || fileName.startsWith(".")) continue;
    const word = fileName.replace(/\.[^.]+$/, "").trim();
    if (!word) continue;
    try {
      const meaning = (await FileSystem.readAsStringAsync(path)).trim();
      if (meaning) results.push({ word, meaning });
    } catch {
      // Skip files that cannot be read as text (e.g. binary assets)
    }
  }
  return results;
}
