import { nativeLearningAudio, nativePath } from './platform';
import type { DiscoverItem } from '../data/discover';

const native = nativeLearningAudio;
// Also serialize across screen unmount/remount, so old cleanup cannot unload
// a new screen's players. No operations touch the ordinary learning player.
let tail: Promise<unknown> = Promise.resolve();
function enqueue<T>(action: () => Promise<T>): Promise<T> {
  const task = tail.then(action);
  tail = task.catch(() => {});
  return task;
}

export class DiscoverAudio {
  private version = 0;
  private slots: Array<string | undefined> = [];
  private active = 0;
  private timer: ReturnType<typeof setInterval> | undefined;
  private disposed = false;
  private loadedKey: string | undefined;
  private position = 0;

  constructor(private update: (playing: boolean, position: number) => void, private fail: (message: string) => void) {}

  resume(item: DiscoverItem, next?: DiscoverItem) {
    const position = this.position >= (item.audio?.endMs ?? 0) ? undefined : this.position;
    this.show(item, next, position, true);
  }

  show(item: DiscoverItem, next?: DiscoverItem, startMs?: number, resume = false) {
    const version = ++this.version;
    const start = item.audio ? Math.max(item.audio.startMs, Math.min(startMs ?? item.audio.startMs, item.audio.endMs - 1)) : 0;
    clearInterval(this.timer);
    this.position = start;
    this.update(false, start);
    void enqueue(async () => {
      if (this.disposed || version !== this.version) return;
      const api = native();
      if (!api?.feedPrepare) {
        if (item.audio) throw new Error('请重新构建客户端以启用音频预加载');
        return;
      }
      await api.feedPause(this.active);
      if (item.audio) {
        const canResume = resume && startMs !== undefined && this.loadedKey === item.key;
        if (!canResume) {
          const cached = start === item.audio.startMs ? this.slots.indexOf(item.key) : -1;
          this.active = cached >= 0 ? cached : this.loadedKey === item.key ? this.active : 1 - this.active;
          this.loadedKey = undefined;
          if (cached < 0) {
            this.slots[this.active] = undefined;
            await api.feedPrepare(this.active, nativePath(item.audio.uri), start);
          }
          this.loadedKey = item.key;
        }
        if (this.disposed || version !== this.version) return;
        // A slot is consumed once; returning to it starts at the segment start.
        this.slots[this.active] = undefined;
        await api.feedPlay(this.active);
        if (this.disposed || version !== this.version) { await api.feedPause(this.active); return; }
        this.update(true, start);
        this.timer = setInterval(() => {
          void enqueue(async () => {
            if (this.disposed || version !== this.version) return;
            const status = await api.feedStatus(this.active);
            if (this.disposed || version !== this.version) return;
            if (status.positionMillis >= item.audio!.endMs || !status.isPlaying) {
              await api.feedPause(this.active);
              clearInterval(this.timer);
              this.position = item.audio!.endMs;
              this.update(false, this.position);
            } else {
              this.position = status.positionMillis;
              this.update(true, this.position);
            }
          }).catch(e => this.report(e, version));
        }, 100);
      }
      if (next?.audio && !this.disposed && version === this.version) {
        try { await this.prepare(next, 1 - this.active); } catch { /* Retry visibly when selected. */ }
      }
    }).catch(e => this.report(e, version));
  }

  private async prepare(item: DiscoverItem, slot: number) {
    if (!item.audio || this.slots[slot] === item.key) return;
    this.slots[slot] = undefined;
    const uri = item.audio.uri;
    const path = nativePath(uri);
    await native().feedPrepare(slot, path, item.audio.startMs);
    this.slots[slot] = item.key;
  }

  pause() {
    ++this.version;
    clearInterval(this.timer);
    this.update(false, this.position);
    void enqueue(async () => { await native()?.feedPause?.(this.active); }).catch(() => {});
  }

  private report(error: unknown, version: number) {
    if (!this.disposed && version === this.version) this.fail(error instanceof Error ? error.message : String(error));
  }

  dispose() {
    this.disposed = true;
    ++this.version;
    clearInterval(this.timer);
    void enqueue(async () => { await native()?.feedUnload?.(); }).catch(() => {});
  }
}
