import type { AVPlaybackStatus, SoundLike } from './platform';

type CreateSound = (uri: string, rate: number, onStatus: (status: AVPlaybackStatus) => void) => Promise<SoundLike>;

// The listening routes may unmount/remount while a native load is in flight.
// Across those sessions all operations share the same underlying player.
let tail: Promise<unknown> = Promise.resolve();

export class ListeningPlayback {
  generation = 0;
  sound: SoundLike | null = null;

  constructor(private readonly createSound: CreateSound) {}

  enqueue<T>(run: () => Promise<T>): Promise<T> {
    const operation = tail.then(run);
    tail = operation.catch(() => {});
    return operation;
  }

  load(uri: string, rate: number, onStatus: (status: AVPlaybackStatus) => void) {
    const generation = ++this.generation;
    const current = () => generation === this.generation;
    return this.enqueue(async () => {
      if (!current()) return null;
      await this.releaseSound();
      if (!current()) return null;
      const sound = await this.createSound(uri, rate, (status) => {
        if (current()) onStatus(status);
      });
      if (!current()) {
        await sound.unloadAsync();
        return null;
      }
      this.sound = sound;
      return generation;
    });
  }

  unload() {
    return this.discard(++this.generation);
  }

  discard(generation: number) {
    return this.enqueue(async () => {
      if (generation === this.generation) await this.releaseSound();
    });
  }

  private async releaseSound() {
    const sound = this.sound;
    this.sound = null;
    await sound?.unloadAsync();
  }
}

export type ListeningRepeatMode = 'off' | 'one' | 'practice' | 'pass';

export function nextRepeatAudio<T extends { id: string }>(
  audios: readonly T[],
  currentId: string | null,
  mode: ListeningRepeatMode,
) {
  if (mode !== 'practice' && mode !== 'pass') return;
  const index = audios.findIndex((audio) => audio.id === currentId);
  return index < 0 ? undefined : (audios[index + 1] ?? (mode === 'practice' ? audios[0] : undefined));
}
