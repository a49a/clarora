import {
  addAudioToPractice,
  deleteListeningAudio,
  deleteListeningPractice,
  type ListeningAudio,
} from '../data/database';
import { parseSubtitleCues, serializeSubtitleCues, type SubtitleCue } from '../data/subtitles';
import { FileSystem, mergeAudios } from './platform';

export type CommandAudioFile = { path: string; name: string };

export function commandAudioFiles(output: string): CommandAudioFile[] {
  const audioExtensions = new Set(['.mp3', '.m4a', '.wav', '.aac', '.aiff', '.flac', '.ogg', '.mp4', '.mov']);
  const files: Array<{ path: string; name: string }> = [];
  for (const rawLine of output.split('\n')) {
    const line = rawLine.trim().replace(/^["']|["']$/g, '');
    if (!line || line.endsWith('(deleted)')) continue;
    const dot = line.lastIndexOf('.');
    const ext = dot === -1 ? '' : line.slice(dot).toLowerCase();
    if (!audioExtensions.has(ext)) continue;
    const name = (line.split('/').pop() ?? line).replace(/\.[^.]+$/, '');
    if (!name) continue;
    files.push({ path: line, name });
  }
  return files;
}

export async function importCommandAudio(practiceId: string, scanFiles: CommandAudioFile[]) {
  let imported = 0;
  let skipped = 0;
  let lastImportedId: string | null = null;
  for (const file of scanFiles) {
    const name = file.name.trim();
    if (!name) {
      skipped += 1; // empty name
      continue;
    }
    try {
      const created = await addAudioToPractice(practiceId, name, file.path, '');
      lastImportedId = created.id;
      imported += 1;
    } catch {
      skipped += 1; // duplicate name etc.
    }
  }
  return { imported, skipped, lastImportedId };
}

async function cleanupFiles(uris: string[]) {
  if (uris.length === 0) return;
  try {
    const base = await FileSystem.getDocumentDirectoryAsync();
    for (const uri of uris) {
      if (uri && uri.startsWith(base)) {
        try {
          await FileSystem.deleteAsync(uri);
        } catch {
          // file already gone — ignore
        }
      }
    }
  } catch {
    // base dir unavailable — skip cleanup
  }
}

export async function removeLibraryAudio(id: string) {
  await cleanupFiles(await deleteListeningAudio(id));
}

export async function removeLibraryPractice(id: string) {
  await cleanupFiles(await deleteListeningPractice(id));
}

// Merge in the practice's playback order and shift cues by each actual duration.
export async function mergeLibraryAudio(practiceId: string, chosen: ListeningAudio[], name: string) {
  const documentDir = await FileSystem.getDocumentDirectoryAsync();
  const directory = `${documentDir}Clarora/Merged`;
  await FileSystem.makeDirectoryAsync(directory);
  const base = `${directory}/merged_${Date.now()}`;
  const audioUri = `${base}.m4a`;
  const merged = await mergeAudios(
    chosen.map((audio) => audio.audio_uri),
    audioUri,
  );

  let subtitleUri = '';
  const cues: SubtitleCue[] = [];
  let offsetMs = 0;
  for (let index = 0; index < chosen.length; index++) {
    const audio = chosen[index];
    if (audio.subtitle_uri) {
      try {
        const content = await FileSystem.readAsStringAsync(audio.subtitle_uri);
        for (const cue of parseSubtitleCues(content)) {
          cues.push({
            ...cue,
            start: (cue.start * 1000 + offsetMs) / 1000,
            end: (cue.end * 1000 + offsetMs) / 1000,
          });
        }
      } catch {
        // One unreadable subtitle should not fail the whole merge.
      }
    }
    offsetMs += merged.segmentDurationMs[index] ?? 0;
  }
  if (cues.length > 0) {
    subtitleUri = `${base}.srt`;
    await FileSystem.writeFileAsync(subtitleUri, serializeSubtitleCues(cues));
  }

  return await addAudioToPractice(practiceId, name, audioUri, subtitleUri);
}
