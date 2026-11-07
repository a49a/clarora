import type { SubtitleCue } from '../data/subtitles';
import type { BlindListeningMachine } from '../ui/blindListening';
import type { SoundLike } from './platform';

export type ListeningSubtitles = {
  audioId: string | null;
  generation: number;
  revision: number;
  cues: SubtitleCue[];
};

type PlaybackSnapshot = {
  sound: SoundLike | null;
  source: ListeningSubtitles;
  generation: number;
  active: boolean;
};

export function captureBlindPlayback(audioId: string | undefined, cues: SubtitleCue[], read: () => PlaybackSnapshot) {
  const captured = read();
  const { sound, source, generation } = captured;
  const isCurrent = () => {
    const current = read();
    return (
      !!audioId &&
      current.active &&
      sound !== null &&
      current.sound === sound &&
      current.generation === generation &&
      current.source === source &&
      source.audioId === audioId &&
      source.generation === generation &&
      source.cues === cues
    );
  };
  return sound && isCurrent() ? { sound, source, isCurrent } : null;
}

export async function revealListeningCue(
  machine: BlindListeningMachine,
  context: NonNullable<ReturnType<typeof captureBlindPlayback>>,
  audioId: string,
  timing: { positionMs: number; durationMs: number; isPlaying: boolean },
  enqueue: (action: () => Promise<void>) => Promise<void>,
  stillPlaying: () => boolean,
) {
  return machine.requestReveal({
    audioId,
    cues: context.source.cues.map((cue) => ({
      id: cue.id,
      startMs: cue.start * 1000,
      endMs: cue.end * 1000,
      text: cue.text,
    })),
    ...timing,
    isCurrent: context.isCurrent,
    pause: (requestIsCurrent) =>
      enqueue(async () => {
        if (requestIsCurrent() && context.isCurrent() && stillPlaying()) await context.sound.pauseAsync();
      }),
  });
}
